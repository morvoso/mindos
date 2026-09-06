//! fork / clone.

use super::process::{FsInfo, Process};
use crate::arch::x86_64::interrupts::TrapFrame;
use crate::mm::errno::*;
use crate::sched::task::Task;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;

pub const CLONE_VM: u64 = 0x100;
pub const CLONE_FS: u64 = 0x200;
pub const CLONE_FILES: u64 = 0x400;
pub const CLONE_SIGHAND: u64 = 0x800;
pub const CLONE_PIDFD: u64 = 0x1000;
pub const CLONE_PTRACE: u64 = 0x2000;
pub const CLONE_VFORK: u64 = 0x4000;
pub const CLONE_PARENT: u64 = 0x8000;
pub const CLONE_THREAD: u64 = 0x10000;
pub const CLONE_NEWNS: u64 = 0x20000;
pub const CLONE_SYSVSEM: u64 = 0x40000;
pub const CLONE_SETTLS: u64 = 0x80000;
pub const CLONE_PARENT_SETTID: u64 = 0x100000;
pub const CLONE_CHILD_CLEARTID: u64 = 0x200000;
pub const CLONE_DETACHED: u64 = 0x400000;
pub const CLONE_UNTRACED: u64 = 0x800000;
pub const CLONE_CHILD_SETTID: u64 = 0x1000000;
pub const CLONE_NEWCGROUP: u64 = 0x2000000;
pub const CLONE_NEWUTS: u64 = 0x4000000;
pub const CLONE_NEWIPC: u64 = 0x8000000;
pub const CLONE_NEWUSER: u64 = 0x10000000;
pub const CLONE_NEWPID: u64 = 0x20000000;
pub const CLONE_NEWNET: u64 = 0x40000000;
pub const CLONE_IO: u64 = 0x80000000;

pub struct CloneArgs {
    pub flags: u64,
    pub child_stack: u64,
    pub parent_tid: u64,
    pub child_tid: u64,
    pub tls: u64,
    pub exit_signal: i32,
}

pub fn do_clone(frame: &TrapFrame, a: CloneArgs) -> Result<u32> {
    let cur = crate::sched::current();
    let parent = cur.proc.clone();
    let flags = a.flags;
    if flags & (CLONE_NEWNS | CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNET | CLONE_NEWUTS | CLONE_NEWIPC) != 0 {
        return Err(EINVAL);
    }
    if flags & CLONE_THREAD != 0 && flags & (CLONE_VM | CLONE_SIGHAND) != (CLONE_VM | CLONE_SIGHAND) {
        return Err(EINVAL);
    }

    // the child's register state
    let mut child_frame = *frame;
    child_frame.rax = 0;
    if a.child_stack != 0 {
        child_frame.rsp = a.child_stack;
    }

    // save the parent's live FPU state so the child inherits it
    unsafe { crate::arch::x86_64::cpu::xsave(cur.fpu_area()) };

    let (proc, is_thread) = if flags & CLONE_THREAD != 0 {
        (parent.clone(), true)
    } else {
        let p = Process::new();
        // address space
        let aspace = if flags & CLONE_VM != 0 {
            parent.aspace()
        } else {
            match parent.aspace() {
                Some(a) => Some(a.fork()?),
                None => None,
            }
        };
        p.set_aspace(aspace);
        // files
        let files = if flags & CLONE_FILES != 0 { parent.files() } else { parent.files().duplicate() };
        *p.files.lock() = files;
        // fs info (cwd/root) -- always copied (CLONE_FS sharing approximated)
        {
            let pf = parent.fs.lock();
            *p.fs.lock() = pf.as_ref().map(|f| FsInfo { cwd: f.cwd.clone(), cwd_path: f.cwd_path.clone(), root: f.root.clone() });
        }
        // signal handlers
        let sh = if flags & CLONE_SIGHAND != 0 { parent.sighand() } else { parent.sighand().duplicate() };
        *p.sighand.lock() = sh;
        *p.comm.lock() = parent.comm();
        *p.exe.lock() = parent.exe.lock().clone();
        *p.cmdline.lock() = parent.cmdline.lock().clone();
        *p.tty.lock() = parent.tty.lock().clone();
        *p.rlimits.lock() = *parent.rlimits.lock();
        p.pgid.store(parent.pgid(), Ordering::Relaxed);
        p.sid.store(parent.sid(), Ordering::Relaxed);
        p.umask.store(parent.umask.load(Ordering::Relaxed), Ordering::Relaxed);
        p.uid.store(parent.uid.load(Ordering::Relaxed), Ordering::Relaxed);
        p.gid.store(parent.gid.load(Ordering::Relaxed), Ordering::Relaxed);
        p.exit_signal.store(a.exit_signal, Ordering::Relaxed);
        if flags & CLONE_PARENT != 0 {
            if let Some(gp) = parent.parent() {
                gp.add_child(&p);
            } else {
                parent.add_child(&p);
            }
        } else {
            parent.add_child(&p);
        }
        (p, false)
    };

    let name = parent.comm();
    let task = Task::new_user(proc.clone(), &child_frame, &name);
    task.copy_fpu_from(&cur);
    {
        let ps = cur.sig.lock();
        let mut cs = task.sig.lock();
        cs.mask = ps.mask;
        cs.altstack_sp = ps.altstack_sp;
        cs.altstack_size = ps.altstack_size;
        cs.altstack_flags = ps.altstack_flags;
    }
    if flags & CLONE_SETTLS != 0 {
        task.fs_base.store(a.tls, Ordering::Relaxed);
    } else {
        task.fs_base.store(cur.fs_base.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    if flags & CLONE_CHILD_CLEARTID != 0 {
        task.clear_child_tid.store(a.child_tid, Ordering::Relaxed);
    }
    if flags & CLONE_CHILD_SETTID != 0 {
        if flags & CLONE_VM != 0 {
            // shared memory: write now
            let _ = crate::mm::user::write_user::<u32>(a.child_tid as usize, task.tid);
        } else {
            task.set_child_tid.store(a.child_tid, Ordering::Relaxed);
        }
    }
    if flags & CLONE_PARENT_SETTID != 0 {
        crate::mm::user::write_user::<u32>(a.parent_tid as usize, task.tid)?;
    }
    proc.add_thread(task.clone());
    let tid = task.tid;
    let _ = is_thread;
    crate::sched::enqueue(task);

    if flags & CLONE_VFORK != 0 {
        // wait until the child execs or exits
        let child = proc.clone();
        let _ = child.vfork_wq.wait_until_uninterruptible(|| child.vfork_done.load(Ordering::Acquire));
    }
    Ok(if is_thread { tid } else { proc.pid })
}
