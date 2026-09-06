//! Sleep timers checked from the timer tick, plus process interval timers.

use super::task::Task;
use crate::sync::SpinLock;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

struct Timers {
    sleepers: Vec<(u64, Arc<Task>)>,
}

static TIMERS: SpinLock<Timers> = SpinLock::new(Timers { sleepers: Vec::new() });

pub fn init() {}

pub fn arm(t: &Arc<Task>, deadline_ns: u64) {
    t.wake_deadline.store(deadline_ns, Ordering::Relaxed);
    TIMERS.lock().sleepers.push((deadline_ns, t.clone()));
}

pub fn disarm(t: &Arc<Task>) {
    t.wake_deadline.store(0, Ordering::Relaxed);
    TIMERS.lock().sleepers.retain(|(_, x)| !Arc::ptr_eq(x, t));
}

/// Called from the timer interrupt.
pub fn tick() {
    let now = crate::arch::x86_64::tsc::uptime_ns();
    let expired: Vec<Arc<Task>> = {
        let mut tm = TIMERS.lock();
        if tm.sleepers.is_empty() {
            return;
        }
        let mut out = Vec::new();
        tm.sleepers.retain(|(d, t)| {
            if *d <= now {
                out.push(t.clone());
                false
            } else {
                true
            }
        });
        out
    };
    for t in expired {
        t.timed_out.store(true, Ordering::Relaxed);
        super::wake(&t);
    }
    crate::proc::process::tick_itimers(now);
}
