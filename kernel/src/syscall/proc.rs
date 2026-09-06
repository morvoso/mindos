//! Process management system calls.

use super::SysResult;
use crate::arch::x86_64::interrupts::TrapFrame;
use crate::mm::errno::*;
use crate::mm::user::{read_path, read_str_array, read_user, write_user};
use crate::proc::exit::{self, WaitResult};
use crate::proc::fork::{self, CloneArgs};
use crate::proc::process;
use core::sync::atomic::Ordering;

pub fn clone(frame: &TrapFrame, flags: u64, stack: u64, parent_tid: usize, child_tid: usize, tls: u64) -> SysResult {
    let exit_signal = (flags & 0xff) as i32;
    let pid = fork::do_clone(frame, CloneArgs { flags: flags & !0xff, child_stack: stack, parent_tid: parent_tid as u64, child_tid: child_tid as u64, tls, exit_signal })?;
    Ok(pid as u64)
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Clone3Args {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

pub fn clone3(frame: &TrapFrame, uargs: usize, size: usize) -> SysResult {
    if size < 64 || size > 256 {
        return Err(EINVAL);
    }
    let mut a = Clone3Args::default();
    let n = size.min(core::mem::size_of::<Clone3Args>());
    let bytes = unsafe { core::slice::from_raw_parts_mut(&mut a as *mut _ as *mut u8, n) };
    crate::mm::user::copy_from_user(bytes, uargs)?;
    if a.set_tid_size != 0 {
        return Err(EINVAL);
    }
    let stack = if a.stack != 0 { a.stack + a.stack_size } else { 0 };
    let pid = fork::do_clone(frame, CloneArgs { flags: a.flags, child_stack: stack, parent_tid: a.parent_tid, child_tid: a.child_tid, tls: a.tls, exit_signal: a.exit_signal as i32 })?;
    Ok(pid as u64)
}

pub fn fork(frame: &TrapFrame, vfork: bool) -> SysResult {
    let flags = if vfork { fork::CLONE_VFORK | fork::CLONE_VM } else { 0 };
    let pid = fork::do_clone(frame, CloneArgs { flags, child_stack: 0, parent_tid: 0, child_tid: 0, tls: 0, exit_signal: 17 })?;
    Ok(pid as u64)
}

pub fn execve(frame: &mut TrapFrame, path: usize, argv: usize, envp: usize) -> SysResult {
    let p = read_path(path)?;
    let args = read_str_array(argv, 65536, 2 * 1024 * 1024)?;
    let env = read_str_array(envp, 65536, 2 * 1024 * 1024)?;
    crate::proc::exec::do_execve(frame, &p, args, env)?;
    Ok(0)
}

pub fn execveat(frame: &mut TrapFrame, dirfd: i32, path: usize, argv: usize, envp: usize, flags: u32) -> SysResult {
    let p = read_path(path)?;
    let full = if p.starts_with('/') || dirfd == crate::fs::vfs::AT_FDCWD {
        p
    } else {
        let f = super::fs::files().get(dirfd)?;
        if p.is_empty() && flags & crate::fs::vfs::AT_EMPTY_PATH != 0 {
            f.path()
        } else {
            crate::fs::path::join(&f.path(), &p)
        }
    };
    let args = read_str_array(argv, 65536, 2 * 1024 * 1024)?;
    let env = read_str_array(envp, 65536, 2 * 1024 * 1024)?;
    crate::proc::exec::do_execve(frame, &full, args, env)?;
    Ok(0)
}

pub fn exit(code: i32) -> SysResult {
    let t = crate::sched::current();
    if t.proc.thread_count() <= 1 {
        drop(t);
        exit::exit_group(code)
    }
    drop(t);
    crate::sched::exit_current_task(code)
}

pub fn exit_group(code: i32) -> SysResult {
    exit::exit_group(code)
}

#[repr(C)]
#[derive(Default)]
struct RUsage {
    utime: [i64; 2],
    stime: [i64; 2],
    rest: [i64; 14],
}

fn write_rusage(addr: usize, r: &WaitResult) -> Result<()> {
    let mut ru = RUsage::default();
    ru.utime = [(r.utime_ns / 1_000_000_000) as i64, ((r.utime_ns % 1_000_000_000) / 1000) as i64];
    ru.stime = [(r.stime_ns / 1_000_000_000) as i64, ((r.stime_ns % 1_000_000_000) / 1000) as i64];
    let bytes = unsafe { core::slice::from_raw_parts(&ru as *const _ as *const u8, core::mem::size_of::<RUsage>()) };
    crate::mm::user::copy_to_user(addr, bytes)
}

pub fn wait4(pid: i64, status: usize, options: u32, rusage: usize) -> SysResult {
    match exit::wait(pid, options)? {
        Some(r) => {
            if status != 0 {
                write_user::<i32>(status, r.status)?;
            }
            if rusage != 0 {
                write_rusage(rusage, &r)?;
            }
            Ok(r.pid as u64)
        }
        None => Ok(0),
    }
}

pub fn waitid(idtype: i32, id: i64, infop: usize, options: u32, rusage: usize) -> SysResult {
    // P_ALL=0, P_PID=1, P_PGID=2
    let which = match idtype {
        0 => -1,
        1 => id,
        2 => -id,
        _ => return Err(EINVAL),
    };
    let mut opts = options & (exit::WNOHANG | exit::WUNTRACED | exit::WCONTINUED | exit::WNOWAIT);
    if options & exit::WEXITED == 0 && options & (exit::WSTOPPED | exit::WCONTINUED) == 0 {
        return Err(EINVAL);
    }
    if options & exit::WSTOPPED != 0 {
        opts |= exit::WUNTRACED;
    }
    match exit::wait(which, opts)? {
        Some(r) => {
            if infop != 0 {
                // siginfo_t: signo=SIGCHLD, code, pid, uid, status
                let (code, status) = if r.status & 0x7f == 0 {
                    (1, r.status >> 8) // CLD_EXITED
                } else if r.status & 0xff == 0x7f {
                    (5, (r.status >> 8) & 0xff) // CLD_STOPPED
                } else if r.status == 0xffff {
                    (6, 18) // CLD_CONTINUED
                } else {
                    (2, r.status & 0x7f) // CLD_KILLED
                };
                let mut info = [0u8; 128];
                info[0..4].copy_from_slice(&17i32.to_le_bytes());
                info[8..12].copy_from_slice(&code.to_le_bytes());
                info[16..20].copy_from_slice(&(r.pid as i32).to_le_bytes());
                info[24..28].copy_from_slice(&status.to_le_bytes());
                crate::mm::user::copy_to_user(infop, &info)?;
            }
            if rusage != 0 {
                write_rusage(rusage, &r)?;
            }
            Ok(0)
        }
        None => {
            if infop != 0 {
                let info = [0u8; 128];
                crate::mm::user::copy_to_user(infop, &info)?;
            }
            Ok(0)
        }
    }
}

pub fn setpgid(pid: u32, pgid: u32) -> SysResult {
    let me = crate::sched::current_ref().proc.clone();
    let target = if pid == 0 { me.clone() } else { process::lookup(pid).ok_or(ESRCH)? };
    let pgid = if pgid == 0 { target.pid } else { pgid };
    if target.pid != me.pid && target.ppid() != me.pid {
        return Err(ESRCH);
    }
    if target.sid() == target.pid {
        return Err(EPERM);
    }
    target.pgid.store(pgid, Ordering::Relaxed);
    Ok(0)
}

pub fn getpgid(pid: u32) -> SysResult {
    let p = if pid == 0 { crate::sched::current_ref().proc.clone() } else { process::lookup(pid).ok_or(ESRCH)? };
    Ok(p.pgid() as u64)
}

pub fn getsid(pid: u32) -> SysResult {
    let p = if pid == 0 { crate::sched::current_ref().proc.clone() } else { process::lookup(pid).ok_or(ESRCH)? };
    Ok(p.sid() as u64)
}

pub fn setsid() -> SysResult {
    let p = crate::sched::current_ref().proc.clone();
    if p.pgid() == p.pid && p.sid() != p.pid {
        // already a group leader of someone else's session
        if process::all().iter().any(|x| x.pid != p.pid && x.pgid() == p.pid) {
            return Err(EPERM);
        }
    }
    p.sid.store(p.pid, Ordering::Relaxed);
    p.pgid.store(p.pid, Ordering::Relaxed);
    *p.tty.lock() = None;
    Ok(p.pid as u64)
}

pub fn _unused() {
    let _ = read_user::<u8>;
}
