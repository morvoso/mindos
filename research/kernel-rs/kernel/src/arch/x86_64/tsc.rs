//! Time-stamp counter based clocks.

use super::cpu;
use core::sync::atomic::{AtomicU64, Ordering};

static TSC_HZ: AtomicU64 = AtomicU64::new(0);
static BOOT_TSC: AtomicU64 = AtomicU64::new(0);

pub fn set_frequency(hz: u64) {
    if BOOT_TSC.load(Ordering::Relaxed) == 0 {
        BOOT_TSC.store(cpu::rdtsc(), Ordering::Relaxed);
    }
    TSC_HZ.store(hz, Ordering::Relaxed);
}

pub fn frequency() -> u64 {
    TSC_HZ.load(Ordering::Relaxed)
}

/// Nanoseconds since the TSC clock was started (0 before calibration).
pub fn uptime_ns() -> u64 {
    let hz = TSC_HZ.load(Ordering::Relaxed);
    if hz == 0 {
        return 0;
    }
    let d = cpu::rdtsc().wrapping_sub(BOOT_TSC.load(Ordering::Relaxed));
    ((d as u128) * 1_000_000_000u128 / hz as u128) as u64
}

pub fn ns_to_ticks(ns: u64) -> u64 {
    let hz = TSC_HZ.load(Ordering::Relaxed);
    ((ns as u128) * hz as u128 / 1_000_000_000u128) as u64
}

/// Busy wait.
pub fn spin_ns(ns: u64) {
    let hz = TSC_HZ.load(Ordering::Relaxed);
    if hz == 0 {
        for _ in 0..ns / 10 {
            core::hint::spin_loop();
        }
        return;
    }
    let end = cpu::rdtsc() + ns_to_ticks(ns);
    while cpu::rdtsc() < end {
        core::hint::spin_loop();
    }
}
