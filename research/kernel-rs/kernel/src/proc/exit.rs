//! Task and process termination, zombies, and wait().

use super::process::{self, ProcState, Process};
use super::signal;
use crate::mm::errno::*;
use crate::sched::task::Task;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;

pub const WNOHANG: u32 = 1;
pub const WUNTRACED: u32 = 2;
pub const WCONTINUED: u32 = 8;
pub const WEXITED: u32 = 4;
pub const WSTOPPED: u32 = 2;
pub const WNOWAIT: u32 = 0x0100_0000;
pub const __WALL: u32 = 0x4000_0000;
pub const __WCLONE: u32 = 0x8000_0000;

/// Kill the whole thread group with `code` (exit_group) or a signal.
pub fn exit_group(code: i32) -> ! {
    let t = crate::sched::current();
    let p = t.proc.clone();
    if !p.group_exit.swap(true, Ordering::AcqRel) {
        p.exit_code.store((code & 0xff) << 8, Ordering::Relaxed);
    }
    kill_other_threads(&p, &t);
    drop(p);
    drop(t);
    crate::sched::exit_current_task(code)
}

pub fn exit_group_signal(signo: i32) -> ! {
    let t = crate::sched::current();
    let p = t.proc.clone();
    if !p.group_exit.swap(true, Ordering::AcqRel) {
        p.exit_code.store(signo & 0x7f, Ordering::Relaxed);
    }
    kill_other_threads(&p, &t);
    drop(p);
    drop(t);
    crate::sched::exit_current_task(128 + signo)
}

fn kill_other_threads(p: &Arc<Process>, me: &Arc<Task>) {
    let threads = p.threads.lock().clone();
    for t in threads {
        if !Arc::ptr_eq(&t, me) {
            t.post_signal(signal::SIGKILL);
        }
    }
}

/// Per-thread teardown; called by the scheduler before the final switch.
pub fn task_exit(t: &Arc<Task>, code: i32) {
    t.exit_code.store(code, Ordering::Relaxed);
    // clear_child_tid: write 0 and wake one futex waiter
    let ctid = t.clear_child_tid.swap(0, Ordering::Relaxed);
    if ctid != 0 && t.proc.aspace().is_some() {
        let _ = crate::mm::user::write_user::<u32>(ctid as usize, 0);
        crate::syscall::futex::wake(ctid as usize, 1, true);
    }
    let p = t.proc.clone();
    if p.pid == 0 {
        return; // kernel thread
    }
    let remaining = p.remove_thread(t);
    if remaining > 0 {
        return;
    }
    process_exit(&p, code);
}

fn process_exit(p: &Arc<Process>, code: i32) {
    if !p.group_exit.load(Ordering::Relaxed) {
        p.exit_code.store((code & 0xff) << 8, Ordering::Relaxed);
    }
    // release files and memory
    let files = p.files();
    let dropped = files.clear();
    drop(dropped);
    // switch away from the dying address space before destroying it
    crate::arch::x86_64::cpu::write_cr3(crate::mm::vmm::kernel_pml4());
    let old = p.set_aspace(None);
    drop(old);
    *p.fs.lock() = None;
    p.vfork_done.store(true, Ordering::Release);
    p.vfork_wq.wake_all();
    // controlling terminal of a session leader: hang up
    if let Some(tty) = p.tty.lock().take() {
        if p.sid() == p.pid && tty.session.load(Ordering::Relaxed) == p.pid {
            let fg = tty.fg_pgrp.load(Ordering::Relaxed);
            if fg != 0 && fg != p.pgid() {
                signal::send_to_pgrp(fg, signal::SIGHUP);
            }
            tty.session.store(0, Ordering::Relaxed);
            tty.fg_pgrp.store(0, Ordering::Relaxed);
        }
    }
    // reparent children to init
    let children: alloc::vec::Vec<Arc<Process>> = core::mem::take(&mut *p.children.lock());
    if !children.is_empty() {
        if let Some(init) = process::init_process() {
            for c in children {
                init.add_child(&c);
                if c.is_zombie() {
                    signal::send(&init, signal::SIGCHLD);
                }
            }
            init.child_wq.wake_all();
        }
    }
    p.set_state(ProcState::Zombie);
    if p.pid == 1 {
        klog!("proc", "init exited with status {:#x}", p.exit_code.load(Ordering::Relaxed));
    }
    // notify the parent
    if let Some(parent) = p.parent() {
        let a = parent.sighand().get(signal::SIGCHLD);
        if a.handler == signal::SIG_IGN || a.flags & signal::SA_NOCLDWAIT != 0 {
            // auto-reap
            parent.children.lock().retain(|c| c.pid != p.pid);
            process::unregister(p.pid);
        } else {
            let sig = p.exit_signal.load(Ordering::Relaxed);
            if sig > 0 {
                signal::send(&parent, sig);
            }
        }
        parent.child_wq.wake_all();
    } else {
        process::unregister(p.pid);
    }
}

pub struct WaitResult {
    pub pid: u32,
    pub status: i32,
    pub utime_ns: u64,
    pub stime_ns: u64,
}

fn matches(child: &Process, which: i64, me: &Process) -> bool {
    if which == -1 {
        true
    } else if which > 0 {
        child.pid == which as u32
    } else if which == 0 {
        child.pgid() == me.pgid()
    } else {
        child.pgid() == (-which) as u32
    }
}

/// wait4/waitid core.
pub fn wait(which: i64, options: u32) -> Result<Option<WaitResult>> {
    let t = crate::sched::current();
    let me = t.proc.clone();
    loop {
        let children = me.children.lock().clone();
        let mut any = false;
        for c in &children {
            if !matches(c, which, &me) {
                continue;
            }
            any = true;
            if c.is_zombie() {
                if options & WNOWAIT == 0 {
                    me.children.lock().retain(|x| x.pid != c.pid);
                    process::unregister(c.pid);
                }
                let cpu = c.threads.lock().iter().map(|t| t.cpu_time_ns.load(Ordering::Relaxed)).sum::<u64>();
                return Ok(Some(WaitResult { pid: c.pid, status: c.exit_code.load(Ordering::Relaxed), utime_ns: cpu, stime_ns: 0 }));
            }
            if options & WUNTRACED != 0 && c.state() == ProcState::Stopped && !c.stopped_reported.swap(true, Ordering::Relaxed) {
                let sig = c.stop_signal.load(Ordering::Relaxed);
                return Ok(Some(WaitResult { pid: c.pid, status: 0x7f | (sig << 8), utime_ns: 0, stime_ns: 0 }));
            }
            if options & WCONTINUED != 0 && c.state() == ProcState::Running && c.stopped_reported.load(Ordering::Relaxed) && !c.continued_reported.swap(true, Ordering::Relaxed) {
                c.stopped_reported.store(false, Ordering::Relaxed);
                return Ok(Some(WaitResult { pid: c.pid, status: 0xffff, utime_ns: 0, stime_ns: 0 }));
            }
        }
        if !any {
            return Err(ECHILD);
        }
        if options & WNOHANG != 0 {
            return Ok(None);
        }
        me.child_wq.wait_until(|| {
            let ch = me.children.lock();
            ch.iter().any(|c| {
                matches(c, which, &me)
                    && (c.is_zombie()
                        || (options & WUNTRACED != 0 && c.state() == ProcState::Stopped && !c.stopped_reported.load(Ordering::Relaxed))
                        || (options & WCONTINUED != 0 && c.state() == ProcState::Running && c.stopped_reported.load(Ordering::Relaxed) && !c.continued_reported.load(Ordering::Relaxed)))
            })
        })
        .map_err(|_| ERESTARTSYS)?;
    }
}
