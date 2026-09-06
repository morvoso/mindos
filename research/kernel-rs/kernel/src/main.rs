//! MindOS kernel entry point.
//!
//! MindOS is a from-scratch x86_64 kernel whose native system-call ABI is the
//! Linux one, so unmodified Linux binaries run on it. The bootloader (Limine)
//! loads us in the higher half with a direct map of physical memory.

#![no_std]
#![no_main]
#![allow(dead_code)]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

#[macro_use]
pub mod console;

pub mod acpi;
pub mod arch;
pub mod boot;
pub mod dev;
pub mod fs;
pub mod util;
pub mod mm;
pub mod net;
mod panic;
pub mod proc;
pub mod sched;
pub mod sync;
pub mod syscall;

use arch::x86_64 as x86;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[no_mangle]
pub extern "C" fn _start() -> ! {
    dev::serial::init();
    console::enable_serial();
    kprintln!();
    kprintln!("MindOS kernel {} — booting", VERSION);

    let boot = boot::limine::init();
    kprintln!("bootloader: {} {} (base revision {}{})", boot.bootloader, boot.bootloader_version,
        boot.loaded_revision.unwrap_or(0), if boot.base_revision_ok { "" } else { ", requested revision unsupported" });
    kprintln!("kernel at phys {:#x} / virt {:#x}, hhdm {:#x}", boot.kernel_phys, boot.kernel_virt, boot.hhdm);
    if !boot.cmdline.is_empty() {
        kprintln!("cmdline: {}", boot.cmdline);
    }
    let mut usable = 0u64;
    for e in boot.memmap_iter() {
        if e.typ == boot::limine::MEMMAP_USABLE {
            usable += e.length;
        }
    }
    kprintln!("memory map: {} entries, {} MiB usable", boot.memmap.len(), usable >> 20);
    for i in 0..boot.module_count {
        if let Some(m) = &boot.modules[i] {
            kprintln!("module '{}' ({}): {} bytes at phys {:#x}", m.name, m.path, m.size, m.phys);
        }
    }

    mm::init(boot);
    x86::init_bsp(boot);
    dev::fb::init(boot.fb);
    dev::fbcon::init();
    if let Some(fb) = boot.fb {
        klog!("boot", "framebuffer {}x{} pitch {} bpp {} at phys {:#x}", fb.width, fb.height, fb.pitch, fb.bpp, fb.phys);
    }
    acpi::init(boot);
    x86::apic_init();
    x86::interrupts::register_handler(x86::VEC_TIMER, sched::timer_tick);
    dev::ps2::init();
    x86::enable_interrupts();
    klog!("boot", "interrupts enabled");

    heap_selftest();

    let (total, free) = mm::pmm::stats();
    let (hbytes, hpages) = mm::heap::stats();
    klog!("boot", "bring-up complete: {} MiB free of {} MiB, heap {} KiB in {} pages", free * 4 / 1024, total * 4 / 1024, hbytes / 1024, hpages);

    let mut last = 0;
    loop {
        x86::idle_wait();
        let t = sched::TICKS.load(core::sync::atomic::Ordering::Relaxed);
        if t / x86::TIMER_HZ != last {
            last = t / x86::TIMER_HZ;
            if last % 10 == 0 {
                klog!("idle", "uptime {}s", last);
            }
        }
        while let Some(ev) = dev::input::EVENTS.lock().pop() {
            if ev.typ != dev::input::EV_SYN {
                klog!("input", "type {} code {} value {}", ev.typ, ev.code, ev.value);
            }
        }
    }
}

fn heap_selftest() {
    use alloc::vec::Vec;
    let mut v: Vec<alloc::boxed::Box<[u8; 100]>> = Vec::new();
    for i in 0..1000 {
        v.push(alloc::boxed::Box::new([i as u8; 100]));
    }
    let mut big: Vec<u64> = Vec::with_capacity(100_000);
    for i in 0..100_000u64 {
        big.push(i * 3);
    }
    let sum: u64 = big.iter().sum();
    assert_eq!(sum, 3 * (99_999u64 * 100_000 / 2));
    for (i, b) in v.iter().enumerate() {
        assert_eq!(b[50], i as u8);
    }
    let s = alloc::format!("{}-{}", "heap", 42);
    assert_eq!(s, "heap-42");
    klog!("boot", "heap self-test passed");
}
