//! Local APIC (xAPIC MMIO or x2APIC MSR mode).

use super::cpu;
use super::msr;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const REG_ID: u32 = 0x20;
const REG_VERSION: u32 = 0x30;
const REG_TPR: u32 = 0x80;
const REG_EOI: u32 = 0xB0;
const REG_SVR: u32 = 0xF0;
const REG_ESR: u32 = 0x280;
const REG_ICR_LO: u32 = 0x300;
const REG_ICR_HI: u32 = 0x310;
const REG_LVT_TIMER: u32 = 0x320;
const REG_LVT_THERMAL: u32 = 0x330;
const REG_LVT_PERF: u32 = 0x340;
const REG_LVT_LINT0: u32 = 0x350;
const REG_LVT_LINT1: u32 = 0x360;
const REG_LVT_ERROR: u32 = 0x370;
const REG_TIMER_INIT: u32 = 0x380;
const REG_TIMER_CUR: u32 = 0x390;
const REG_TIMER_DIV: u32 = 0x3E0;

static BASE: AtomicUsize = AtomicUsize::new(0);
static X2APIC: AtomicBool = AtomicBool::new(false);
static TIMER_HZ: AtomicU64 = AtomicU64::new(0);
static TICK_HZ: AtomicU64 = AtomicU64::new(0);

#[inline]
fn read(reg: u32) -> u32 {
    if X2APIC.load(Ordering::Relaxed) {
        msr::read(0x800 + reg / 16) as u32
    } else {
        unsafe { core::ptr::read_volatile((BASE.load(Ordering::Relaxed) + reg as usize) as *const u32) }
    }
}
#[inline]
fn write(reg: u32, v: u32) {
    if X2APIC.load(Ordering::Relaxed) {
        msr::write(0x800 + reg / 16, v as u64);
    } else {
        unsafe { core::ptr::write_volatile((BASE.load(Ordering::Relaxed) + reg as usize) as *mut u32, v) };
    }
}

/// Map the LAPIC and initialise it on the calling CPU.
pub fn init() {
    let apic_base = msr::read(msr::IA32_APIC_BASE);
    let x2 = apic_base & (1 << 10) != 0;
    X2APIC.store(x2, Ordering::Relaxed);
    if !x2 && BASE.load(Ordering::Relaxed) == 0 {
        let phys = apic_base & 0xF_FFFF_F000;
        let va = crate::mm::vmm::map_mmio(phys, 0x1000, false);
        BASE.store(va, Ordering::Relaxed);
    }
    // enable (bit 11) — normally already set by firmware/bootloader
    if apic_base & (1 << 11) == 0 {
        msr::write(msr::IA32_APIC_BASE, apic_base | (1 << 11));
    }
    init_this_cpu();
}

/// Per-CPU LAPIC setup (also used by application processors).
pub fn init_this_cpu() {
    write(REG_TPR, 0);
    write(REG_LVT_TIMER, 1 << 16);
    write(REG_LVT_LINT0, 1 << 16);
    write(REG_LVT_LINT1, 1 << 16);
    write(REG_LVT_ERROR, 1 << 16);
    let ver = read(REG_VERSION);
    let max_lvt = (ver >> 16) & 0xFF;
    if max_lvt >= 4 {
        write(REG_LVT_PERF, 1 << 16);
    }
    if max_lvt >= 5 {
        write(REG_LVT_THERMAL, 1 << 16);
    }
    write(REG_SVR, 0x100 | super::VEC_SPURIOUS as u32);
    write(REG_ESR, 0);
    write(REG_ESR, 0);
    eoi();
}

pub fn id() -> u32 {
    if X2APIC.load(Ordering::Relaxed) {
        read(REG_ID)
    } else {
        read(REG_ID) >> 24
    }
}

#[inline]
pub fn eoi() {
    write(REG_EOI, 0);
}

/// Measure the APIC timer and TSC frequencies against the PIT.
/// Returns (apic_timer_hz at divider 16, tsc_hz).
pub fn calibrate() -> (u64, u64) {
    const MS: u32 = 20;
    write(REG_TIMER_DIV, 0x3); // divide by 16
    write(REG_LVT_TIMER, 1 << 16); // masked, one-shot
    write(REG_TIMER_INIT, 0xFFFF_FFFF);
    let t0 = cpu::rdtsc();
    super::pit::wait_ms(MS);
    let t1 = cpu::rdtsc();
    let cur = read(REG_TIMER_CUR);
    write(REG_TIMER_INIT, 0);
    let apic_ticks = (0xFFFF_FFFFu32 - cur) as u64;
    let apic_hz = apic_ticks * 1000 / MS as u64;
    let tsc_hz = (t1 - t0) * 1000 / MS as u64;
    (apic_hz, tsc_hz)
}

pub fn start_timer(apic_hz: u64, tick_hz: u64) {
    TIMER_HZ.store(apic_hz, Ordering::Relaxed);
    TICK_HZ.store(tick_hz, Ordering::Relaxed);
    start_timer_this_cpu();
}

pub fn start_timer_this_cpu() {
    let apic_hz = TIMER_HZ.load(Ordering::Relaxed);
    let tick_hz = TICK_HZ.load(Ordering::Relaxed);
    write(REG_TIMER_DIV, 0x3);
    write(REG_LVT_TIMER, (super::VEC_TIMER as u32) | (1 << 17)); // periodic
    write(REG_TIMER_INIT, (apic_hz / tick_hz).max(1) as u32);
}

/// Send an inter-processor interrupt to a specific LAPIC id.
pub fn send_ipi(lapic_id: u32, vector: u8) {
    if X2APIC.load(Ordering::Relaxed) {
        msr::write(0x830, ((lapic_id as u64) << 32) | vector as u64);
    } else {
        write(REG_ICR_HI, lapic_id << 24);
        write(REG_ICR_LO, vector as u32);
        while read(REG_ICR_LO) & (1 << 12) != 0 {
            core::hint::spin_loop();
        }
    }
}

/// Broadcast an IPI to all other CPUs.
pub fn send_ipi_others(vector: u8) {
    if X2APIC.load(Ordering::Relaxed) {
        msr::write(0x830, (0b11 << 18) | vector as u64);
    } else {
        write(REG_ICR_HI, 0);
        write(REG_ICR_LO, (0b11 << 18) | vector as u32);
        while read(REG_ICR_LO) & (1 << 12) != 0 {
            core::hint::spin_loop();
        }
    }
}
