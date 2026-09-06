//! Fast user-space mutexes.

use super::SysResult;
use crate::mm::errno::*;
use crate::mm::user::read_user;
use crate::sched::task::{Task, TaskState};
use crate::sync::SpinLock;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

const FUTEX_WAIT: u32 = 0;
const FUTEX_WAKE: u32 = 1;
const FUTEX_REQUEUE: u32 = 3;
const FUTEX_CMP_REQUEUE: u32 = 4;
const FUTEX_WAKE_OP: u32 = 5;
const FUTEX_WAIT_BITSET: u32 = 9;
const FUTEX_WAKE_BITSET: u32 = 10;
const FUTEX_PRIVATE_FLAG: u32 = 128;
const FUTEX_CLOCK_REALTIME: u32 = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key(u64, u64);

struct Waiter {
    key: Key,
    task: Arc<Task>,
    bitset: u32,
    woken: bool,
}

static WAITERS: SpinLock<Vec<Waiter>> = SpinLock::new(Vec::new());

fn key_for(uaddr: usize, private: bool) -> Result<Key> {
    if uaddr & 3 != 0 {
        return Err(EINVAL);
    }
    let a = crate::sched::current_ref().proc.aspace().ok_or(EFAULT)?;
    if private {
        return Ok(Key(a.pml4(), uaddr as u64));
    }
    a.populate_range(uaddr, 4, false)?;
    let (pa, _) = a.mapper.translate(uaddr).ok_or(EFAULT)?;
    Ok(Key(0, pa + (uaddr & 0xfff) as u64))
}

/// Wake up to `n` waiters on `uaddr` (used by exit for CLONE_CHILD_CLEARTID).
pub fn wake(uaddr: usize, n: usize, private: bool) -> usize {
    let key = match key_for(uaddr, private) {
        Ok(k) => k,
        Err(_) => return 0,
    };
    // also try the other key space (a waiter may have used the opposite flag)
    let mut count = wake_key(key, n, u32::MAX);
    if count < n {
        if let Ok(k2) = key_for(uaddr, !private) {
            count += wake_key(k2, n - count, u32::MAX);
        }
    }
    count
}

fn wake_key(key: Key, n: usize, bitset: u32) -> usize {
    let mut to_wake = Vec::new();
    {
        let mut ws = WAITERS.lock();
        for w in ws.iter_mut() {
            if to_wake.len() >= n {
                break;
            }
            if w.key == key && !w.woken && w.bitset & bitset != 0 {
                w.woken = true;
                to_wake.push(w.task.clone());
            }
        }
    }
    let count = to_wake.len();
    for t in to_wake {
        crate::sched::wake(&t);
    }
    count
}

fn wait(uaddr: usize, val: u32, deadline: u64, bitset: u32, private: bool) -> SysResult {
    let key = key_for(uaddr, private)?;
    let t = crate::sched::current();
    // register first so a wake between the value check and the sleep is not lost
    t.set_state(TaskState::Blocked);
    t.interruptible.store(true, Ordering::Release);
    WAITERS.lock().push(Waiter { key, task: t.clone(), bitset, woken: false });
    let cur: u32 = match read_user(uaddr) {
        Ok(v) => v,
        Err(e) => {
            remove_waiter(&t);
            t.set_state(TaskState::Running);
            t.interruptible.store(false, Ordering::Release);
            return Err(e);
        }
    };
    if cur != val {
        remove_waiter(&t);
        t.set_state(TaskState::Running);
        t.interruptible.store(false, Ordering::Release);
        return Err(EAGAIN);
    }
    if t.has_pending_signal() {
        remove_waiter(&t);
        t.set_state(TaskState::Running);
        t.interruptible.store(false, Ordering::Release);
        return Err(ERESTARTSYS);
    }
    if deadline != 0 {
        t.timed_out.store(false, Ordering::Relaxed);
        crate::sched::timer::arm(&t, deadline);
    }
    crate::sched::schedule();
    if deadline != 0 {
        crate::sched::timer::disarm(&t);
    }
    t.interruptible.store(false, Ordering::Release);
    let woken = remove_waiter(&t);
    if woken {
        return Ok(0);
    }
    if deadline != 0 && t.timed_out.swap(false, Ordering::Relaxed) {
        return Err(ETIMEDOUT);
    }
    if t.has_pending_signal() {
        return Err(ERESTARTSYS);
    }
    Ok(0)
}

/// Remove the task's waiter entry; returns whether it had been woken by a FUTEX_WAKE.
fn remove_waiter(t: &Arc<Task>) -> bool {
    let mut ws = WAITERS.lock();
    let mut woken = false;
    ws.retain(|w| {
        if Arc::ptr_eq(&w.task, t) {
            woken |= w.woken;
            false
        } else {
            true
        }
    });
    woken
}

fn requeue(uaddr: usize, uaddr2: usize, nwake: usize, nrequeue: usize, private: bool) -> SysResult {
    let k1 = key_for(uaddr, private)?;
    let k2 = key_for(uaddr2, private)?;
    let woken = wake_key(k1, nwake, u32::MAX);
    let mut moved = 0;
    {
        let mut ws = WAITERS.lock();
        for w in ws.iter_mut() {
            if moved >= nrequeue {
                break;
            }
            if w.key == k1 && !w.woken {
                w.key = k2;
                moved += 1;
            }
        }
    }
    Ok((woken + moved) as u64)
}

pub fn futex(uaddr: usize, op: u32, val: u32, timeout_or_val2: usize, uaddr2: usize, val3: u32) -> SysResult {
    let private = op & FUTEX_PRIVATE_FLAG != 0;
    let realtime = op & FUTEX_CLOCK_REALTIME != 0;
    let cmd = op & 0x7f;
    match cmd {
        FUTEX_WAIT => {
            let deadline = if timeout_or_val2 != 0 { crate::arch::x86_64::tsc::uptime_ns() + super::time::read_timespec(timeout_or_val2)? } else { 0 };
            wait(uaddr, val, deadline, u32::MAX, private)
        }
        FUTEX_WAIT_BITSET => {
            if val3 == 0 {
                return Err(EINVAL);
            }
            let deadline = if timeout_or_val2 != 0 {
                let abs = super::time::read_timespec(timeout_or_val2)?;
                let clock_now = if realtime { crate::dev::rtc::wall_time_ns() } else { crate::arch::x86_64::tsc::uptime_ns() };
                let now = crate::arch::x86_64::tsc::uptime_ns();
                let d = now + abs.saturating_sub(clock_now);
                if d <= now {
                    now + 1
                } else {
                    d
                }
            } else {
                0
            };
            wait(uaddr, val, deadline, val3, private)
        }
        FUTEX_WAKE => {
            let key = key_for(uaddr, private)?;
            Ok(wake_key(key, val as usize, u32::MAX) as u64)
        }
        FUTEX_WAKE_BITSET => {
            if val3 == 0 {
                return Err(EINVAL);
            }
            let key = key_for(uaddr, private)?;
            Ok(wake_key(key, val as usize, val3) as u64)
        }
        FUTEX_REQUEUE => requeue(uaddr, uaddr2, val as usize, timeout_or_val2, private),
        FUTEX_CMP_REQUEUE => {
            let cur: u32 = read_user(uaddr)?;
            if cur != val3 {
                return Err(EAGAIN);
            }
            requeue(uaddr, uaddr2, val as usize, timeout_or_val2, private)
        }
        FUTEX_WAKE_OP => {
            // val3 encodes: op(28..31) cmp(24..27) oparg(12..23) cmparg(0..11)
            let opc = (val3 >> 28) & 0xf;
            let cmp = (val3 >> 24) & 0xf;
            let mut oparg = ((val3 >> 12) & 0xfff) as i32;
            let cmparg = (val3 & 0xfff) as i32;
            if opc & 8 != 0 {
                oparg = 1 << (oparg & 31);
            }
            let old: u32 = read_user(uaddr2)?;
            let newv = match opc & 7 {
                0 => oparg as u32,
                1 => old.wrapping_add(oparg as u32),
                2 => old | oparg as u32,
                3 => old & !(oparg as u32),
                4 => old ^ oparg as u32,
                _ => return Err(EINVAL),
            };
            crate::mm::user::write_user::<u32>(uaddr2, newv)?;
            let key1 = key_for(uaddr, private)?;
            let mut n = wake_key(key1, val as usize, u32::MAX);
            let cond = match cmp {
                0 => old == cmparg as u32,
                1 => old != cmparg as u32,
                2 => (old as i32) < cmparg,
                3 => (old as i32) <= cmparg,
                4 => (old as i32) > cmparg,
                5 => (old as i32) >= cmparg,
                _ => return Err(EINVAL),
            };
            if cond {
                let key2 = key_for(uaddr2, private)?;
                n += wake_key(key2, timeout_or_val2, u32::MAX);
            }
            Ok(n as u64)
        }
        _ => Err(ENOSYS),
    }
}
