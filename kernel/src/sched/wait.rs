//! Wait queues: block the current task until a condition holds.

use super::task::{Task, TaskState};
use super::{current, schedule};
use crate::sync::SpinLock;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interrupted;

pub struct WaitQueue {
    waiters: SpinLock<Vec<Arc<Task>>>,
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl WaitQueue {
    pub const fn new() -> Self {
        WaitQueue { waiters: SpinLock::new(Vec::new()) }
    }

    fn remove(&self, t: &Arc<Task>) {
        self.waiters.lock().retain(|w| !Arc::ptr_eq(w, t));
    }

    /// Sleep until `cond()` is true. Interruptible by signals.
    pub fn wait_until<F: FnMut() -> bool>(&self, cond: F) -> Result<(), Interrupted> {
        self.wait_inner(cond, true, 0).map(|_| ())
    }

    /// Sleep until `cond()` is true; signals do not interrupt.
    pub fn wait_until_uninterruptible<F: FnMut() -> bool>(&self, cond: F) {
        let _ = self.wait_inner(cond, false, 0);
    }

    /// Sleep until `cond()` or the absolute deadline (uptime ns, 0 = none).
    /// Ok(true) if the condition became true, Ok(false) on timeout.
    pub fn wait_until_deadline<F: FnMut() -> bool>(&self, cond: F, deadline_ns: u64) -> Result<bool, Interrupted> {
        self.wait_inner(cond, true, deadline_ns)
    }

    fn wait_inner<F: FnMut() -> bool>(&self, mut cond: F, interruptible: bool, deadline_ns: u64) -> Result<bool, Interrupted> {
        let t = current();
        loop {
            if cond() {
                return Ok(true);
            }
            if interruptible && t.has_pending_signal() {
                return Err(Interrupted);
            }
            if deadline_ns != 0 && crate::arch::x86_64::tsc::uptime_ns() >= deadline_ns {
                return Ok(false);
            }
            {
                let mut w = self.waiters.lock();
                if cond() {
                    return Ok(true);
                }
                t.set_state(TaskState::Blocked);
                t.interruptible.store(interruptible, Ordering::Release);
                w.push(t.clone());
            }
            if deadline_ns != 0 {
                super::timer::arm(&t, deadline_ns);
            }
            schedule();
            if deadline_ns != 0 {
                super::timer::disarm(&t);
            }
            t.interruptible.store(false, Ordering::Release);
            self.remove(&t);
        }
    }

    pub fn wake_one(&self) {
        let t = { self.waiters.lock().pop() };
        if let Some(t) = t {
            super::wake(&t);
        }
    }

    pub fn wake_all(&self) {
        let list: Vec<Arc<Task>> = { core::mem::take(&mut *self.waiters.lock()) };
        for t in list {
            super::wake(&t);
        }
    }

    pub fn has_waiters(&self) -> bool {
        !self.waiters.lock().is_empty()
    }
}

/// Sleep the current task until the deadline (uptime ns). Interruptible.
pub fn sleep_until(deadline_ns: u64) -> Result<(), Interrupted> {
    static DUMMY: WaitQueue = WaitQueue::new();
    match DUMMY.wait_until_deadline(|| false, deadline_ns) {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn sleep_ns(ns: u64) -> Result<(), Interrupted> {
    sleep_until(crate::arch::x86_64::tsc::uptime_ns() + ns)
}

// ---- multi-queue waiting (poll/select/epoll) ----------------------------------

impl WaitQueue {
    /// Register `t` as a waiter without blocking.
    pub fn add_waiter(&self, t: &Arc<Task>) {
        let mut w = self.waiters.lock();
        if !w.iter().any(|x| Arc::ptr_eq(x, t)) {
            w.push(t.clone());
        }
    }
    pub fn remove_waiter(&self, t: &Arc<Task>) {
        self.remove(t);
    }
}

/// A set of wait queues the current task is registered on.
pub struct PollTable {
    queues: Vec<&'static WaitQueue>,
    task: Arc<Task>,
}

impl PollTable {
    pub fn new() -> PollTable {
        PollTable { queues: Vec::new(), task: current() }
    }
    /// Register on `wq`. The queue must outlive the poll (files are kept alive by the caller).
    pub fn add(&mut self, wq: &WaitQueue) {
        // SAFETY: the caller holds references to the files owning these queues for the
        // duration of the poll and calls `clear()` before dropping them.
        let wq: &'static WaitQueue = unsafe { &*(wq as *const WaitQueue) };
        wq.add_waiter(&self.task);
        self.queues.push(wq);
    }
    /// Mark the task blocked *before* checking readiness to avoid lost wakeups.
    pub fn prepare(&self) {
        self.task.set_state(TaskState::Blocked);
        self.task.interruptible.store(true, Ordering::Release);
    }
    /// Cancel a prepared block (readiness was found).
    pub fn cancel(&self) {
        self.task.set_state(TaskState::Running);
        self.task.interruptible.store(false, Ordering::Release);
    }
    /// Block until woken or the deadline (0 = none). Ok(false) on timeout.
    pub fn block(&self, deadline_ns: u64) -> Result<bool, Interrupted> {
        if self.task.has_pending_signal() {
            self.cancel();
            return Err(Interrupted);
        }
        if deadline_ns != 0 {
            if crate::arch::x86_64::tsc::uptime_ns() >= deadline_ns {
                self.cancel();
                return Ok(false);
            }
            self.task.timed_out.store(false, Ordering::Relaxed);
            super::timer::arm(&self.task, deadline_ns);
        }
        schedule();
        let timed_out = if deadline_ns != 0 {
            super::timer::disarm(&self.task);
            self.task.timed_out.swap(false, Ordering::Relaxed)
        } else {
            false
        };
        self.task.interruptible.store(false, Ordering::Release);
        if self.task.has_pending_signal() {
            return Err(Interrupted);
        }
        Ok(!timed_out)
    }
    pub fn clear(&mut self) {
        for q in self.queues.drain(..) {
            q.remove_waiter(&self.task);
        }
    }
}

impl Drop for PollTable {
    fn drop(&mut self) {
        self.clear();
    }
}
