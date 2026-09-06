//! Signal system calls.

use super::SysResult;
use crate::mm::errno::*;
use crate::mm::user::{copy_from_user, copy_to_user, read_user, write_user};
use crate::proc::process::{self, SigAction};
use crate::proc::signal::{self, sig_bit, SIGKILL, SIGSTOP};
use core::sync::atomic::Ordering;

const KILL_MASK: u64 = (1 << (SIGKILL - 1)) | (1 << (SIGSTOP - 1));

pub fn rt_sigaction(signo: i32, act: usize, oldact: usize, sigsetsize: usize) -> SysResult {
    if sigsetsize != 8 || signo < 1 || signo > 64 {
        return Err(EINVAL);
    }
    let sh = crate::sched::current_ref().proc.sighand();
    if oldact != 0 {
        let a = sh.get(signo);
        let raw = [a.handler, a.flags, a.restorer, a.mask];
        let bytes = unsafe { core::slice::from_raw_parts(raw.as_ptr() as *const u8, 32) };
        copy_to_user(oldact, bytes)?;
    }
    if act != 0 {
        if signo == SIGKILL || signo == SIGSTOP {
            return Err(EINVAL);
        }
        let mut raw = [0u64; 4];
        let bytes = unsafe { core::slice::from_raw_parts_mut(raw.as_mut_ptr() as *mut u8, 32) };
        copy_from_user(bytes, act)?;
        sh.set(signo, SigAction { handler: raw[0], flags: raw[1], restorer: raw[2], mask: raw[3] });
        // ignoring a pending signal discards it
        if raw[0] == signal::SIG_IGN {
            let p = &crate::sched::current_ref().proc;
            p.shared_pending.fetch_and(!sig_bit(signo), Ordering::Relaxed);
            for t in p.threads.lock().iter() {
                t.sig.lock().pending &= !sig_bit(signo);
            }
        }
    }
    Ok(0)
}

pub fn rt_sigprocmask(how: i32, set: usize, oldset: usize, sigsetsize: usize) -> SysResult {
    if sigsetsize != 8 {
        return Err(EINVAL);
    }
    let t = crate::sched::current_ref();
    let old = t.signal_mask();
    if oldset != 0 {
        write_user::<u64>(oldset, old)?;
    }
    if set != 0 {
        let s: u64 = read_user(set)?;
        let new = match how {
            0 => old | s,          // SIG_BLOCK
            1 => old & !s,         // SIG_UNBLOCK
            2 => s,                // SIG_SETMASK
            _ => return Err(EINVAL),
        };
        t.sig.lock().mask = new & !KILL_MASK;
    }
    Ok(0)
}

pub fn rt_sigpending(set: usize, sigsetsize: usize) -> SysResult {
    if sigsetsize != 8 {
        return Err(EINVAL);
    }
    let t = crate::sched::current_ref();
    let p = t.sig.lock().pending | t.proc.shared_pending();
    write_user::<u64>(set, p)?;
    Ok(0)
}

pub fn rt_sigsuspend(mask: usize, sigsetsize: usize) -> SysResult {
    if sigsetsize != 8 {
        return Err(EINVAL);
    }
    let t = crate::sched::current();
    let newmask: u64 = read_user(mask)?;
    let old = {
        let mut s = t.sig.lock();
        let old = s.mask;
        s.mask = newmask & !KILL_MASK;
        old
    };
    t.saved_mask.store(old, Ordering::Relaxed);
    static WQ: crate::sched::wait::WaitQueue = crate::sched::wait::WaitQueue::new();
    let _ = WQ.wait_until(|| false);
    Err(EINTR)
}

pub fn pause() -> SysResult {
    static WQ: crate::sched::wait::WaitQueue = crate::sched::wait::WaitQueue::new();
    let _ = WQ.wait_until(|| false);
    Err(EINTR)
}

pub fn sigaltstack(ss: usize, old: usize) -> SysResult {
    let t = crate::sched::current_ref();
    if old != 0 {
        let s = t.sig.lock();
        let raw: [u64; 3] = [s.altstack_sp, s.altstack_flags as u64, s.altstack_size];
        let bytes = unsafe { core::slice::from_raw_parts(raw.as_ptr() as *const u8, 24) };
        copy_to_user(old, bytes)?;
    }
    if ss != 0 {
        let mut raw = [0u64; 3];
        let bytes = unsafe { core::slice::from_raw_parts_mut(raw.as_mut_ptr() as *mut u8, 24) };
        copy_from_user(bytes, ss)?;
        let flags = raw[1] as u32;
        let mut s = t.sig.lock();
        if flags & 2 != 0 {
            // SS_DISABLE
            s.altstack_sp = 0;
            s.altstack_size = 0;
            s.altstack_flags = 2;
        } else {
            if raw[2] < 2048 {
                return Err(ENOMEM);
            }
            s.altstack_sp = raw[0];
            s.altstack_size = raw[2];
            s.altstack_flags = flags & 0x8000_0000; // SS_AUTODISARM only
        }
    }
    Ok(0)
}

pub fn kill(pid: i64, signo: i32) -> SysResult {
    if signo < 0 || signo > 64 {
        return Err(EINVAL);
    }
    let me = crate::sched::current_ref().proc.clone();
    if pid > 0 {
        let p = process::lookup(pid as u32).ok_or(ESRCH)?;
        if signo != 0 {
            signal::send(&p, signo);
        }
        Ok(0)
    } else if pid == 0 {
        if signo != 0 {
            signal::send_to_pgrp(me.pgid(), signo);
        }
        Ok(0)
    } else if pid == -1 {
        let mut n = 0;
        for p in process::all() {
            if p.pid > 1 && p.pid != me.pid && !p.is_zombie() {
                if signo != 0 {
                    signal::send(&p, signo);
                }
                n += 1;
            }
        }
        if n == 0 {
            return Err(ESRCH);
        }
        Ok(0)
    } else {
        let pg = (-pid) as u32;
        let n = if signo != 0 { signal::send_to_pgrp(pg, signo) } else { process::all().iter().filter(|p| p.pgid() == pg).count() };
        if n == 0 {
            return Err(ESRCH);
        }
        Ok(0)
    }
}

pub fn tgkill(tgid: u32, tid: u32, signo: i32) -> SysResult {
    if signo < 0 || signo > 64 {
        return Err(EINVAL);
    }
    let t = process::find_task(tid).ok_or(ESRCH)?;
    if tgid != 0 && t.proc.pid != tgid {
        return Err(ESRCH);
    }
    if signo != 0 {
        signal::send_to_task(&t, signo);
    }
    Ok(0)
}

pub fn rt_sigtimedwait(set: usize, info: usize, timeout: usize, sigsetsize: usize) -> SysResult {
    if sigsetsize != 8 {
        return Err(EINVAL);
    }
    let want: u64 = read_user(set)?;
    let t = crate::sched::current();
    let deadline = if timeout != 0 {
        let ts = super::time::read_timespec(timeout)?;
        crate::arch::x86_64::tsc::uptime_ns() + ts
    } else {
        0
    };
    static WQ: crate::sched::wait::WaitQueue = crate::sched::wait::WaitQueue::new();
    loop {
        // take a matching pending signal
        {
            let mut s = t.sig.lock();
            let avail = (s.pending | t.proc.shared_pending()) & want;
            if avail != 0 {
                let signo = avail.trailing_zeros() as i32 + 1;
                if s.pending & sig_bit(signo) != 0 {
                    s.pending &= !sig_bit(signo);
                } else {
                    t.proc.shared_pending.fetch_and(!sig_bit(signo), Ordering::Relaxed);
                }
                drop(s);
                if info != 0 {
                    let mut buf = [0u8; 128];
                    buf[0..4].copy_from_slice(&signo.to_le_bytes());
                    copy_to_user(info, &buf)?;
                }
                return Ok(signo as u64);
            }
        }
        if timeout != 0 && deadline <= crate::arch::x86_64::tsc::uptime_ns() {
            return Err(EAGAIN);
        }
        // block until any signal in `want` arrives (temporarily unblock them)
        let saved = {
            let mut s = t.sig.lock();
            let saved = s.mask;
            s.mask &= !want;
            saved
        };
        let r = WQ.wait_until_deadline(|| (t.sig.lock().pending | t.proc.shared_pending()) & want != 0, deadline);
        t.sig.lock().mask = saved;
        match r {
            Ok(true) => continue,
            Ok(false) => return Err(EAGAIN),
            Err(_) => {
                if (t.sig.lock().pending | t.proc.shared_pending()) & want != 0 {
                    continue;
                }
                return Err(EINTR);
            }
        }
    }
}
