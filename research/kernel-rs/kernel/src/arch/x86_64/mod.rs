//! x86_64 architecture support.

pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod interrupts;
pub mod io;
pub mod ioapic;
pub mod lapic;
pub mod msr;
pub mod percpu;
pub mod pit;
pub mod tsc;

use crate::boot::limine::BootInfo;

/// Timer interrupt frequency (scheduler tick).
pub const TIMER_HZ: u64 = 250;
pub const VEC_TIMER: u8 = 0xF0;
pub const VEC_RESCHED_IPI: u8 = 0xF1;
pub const VEC_HALT_IPI: u8 = 0xF2;
pub const VEC_SPURIOUS: u8 = 0xFF;
pub const VEC_ISA_BASE: u8 = 0x20;

/// Bring the bootstrap processor to a fully usable state: CPU features,
/// descriptor tables, per-CPU area, syscall MSRs, FP/SIMD, PAT.
pub fn init_bsp(_boot: &BootInfo) {
    cpu::init_features();
    cpu::init_pat();
    cpu::enable_fp_simd();
    percpu::init_bsp();
    idt::init();
    idt::load();
    interrupts::init_syscall_msrs();
    let f = cpu::features();
    klog!("cpu", "{} ({}), xsave area {} bytes, avx2={} avx512={} x2apic={} invariant_tsc={}",
        f.brand(), f.vendor(), f.xsave_size, f.avx2, f.avx512f, f.x2apic, f.invariant_tsc);
}

/// Initialise APICs and the timer. Must run after ACPI tables are parsed.
pub fn apic_init() {
    lapic::init();
    ioapic::init();
    let (lapic_hz, tsc_hz) = lapic::calibrate();
    tsc::set_frequency(tsc_hz);
    lapic::start_timer(lapic_hz, TIMER_HZ);
    klog!("apic", "lapic timer {} Hz, tsc {} MHz, tick {} Hz", lapic_hz, tsc_hz / 1_000_000, TIMER_HZ);
}

#[inline(always)]
pub fn enable_interrupts() {
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
}
#[inline(always)]
pub fn disable_interrupts() {
    unsafe { core::arch::asm!("cli", options(nomem, nostack)) };
}
#[inline(always)]
pub fn halt() {
    unsafe { core::arch::asm!("hlt", options(nomem, nostack)) };
}
/// Enable interrupts and halt until the next one (atomic w.r.t. sti).
#[inline(always)]
pub fn idle_wait() {
    unsafe { core::arch::asm!("sti; hlt", options(nomem, nostack)) };
}

pub fn reboot() -> ! {
    disable_interrupts();
    // 8042 keyboard controller reset line
    unsafe { io::outb(0x64, 0xFE) };
    // fall back to a triple fault
    idt::load_empty();
    unsafe { core::arch::asm!("int3") };
    loop {
        halt();
    }
}

pub fn power_off() -> ! {
    disable_interrupts();
    unsafe {
        // QEMU / Bochs ACPI PM1a control (works for both i440fx and q35)
        io::outw(0x604, 0x2000);
        io::outw(0xB004, 0x2000);
        // VirtualBox
        io::outw(0x4004, 0x3400);
    }
    kprintln!("power off not supported on this machine; halting");
    loop {
        halt();
    }
}
