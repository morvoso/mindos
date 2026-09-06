//! Scheduler (task model, run queue, wait queues, timers).

pub mod task;

use crate::arch::x86_64::interrupts::TrapFrame;
use crate::arch::x86_64::percpu;
use core::sync::atomic::{AtomicU64, Ordering};

pub static TICKS: AtomicU64 = AtomicU64::new(0);

/// Timer interrupt: accounting and preemption.
pub fn timer_tick(_frame: &mut TrapFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    percpu::this().ticks.fetch_add(1, Ordering::Relaxed);
}

/// Called on every return to user mode.
pub fn return_to_user(_frame: &mut TrapFrame) {}

pub fn panic_dump_current() {}
