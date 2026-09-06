//! I/O APIC: routes ISA/GSI interrupts to CPU vectors.

use crate::acpi;
use crate::sync::SpinLock;
use alloc::vec::Vec;

struct IoApic {
    base: usize,
    gsi_base: u32,
    count: u32,
}

impl IoApic {
    fn read(&self, reg: u32) -> u32 {
        unsafe {
            core::ptr::write_volatile(self.base as *mut u32, reg);
            core::ptr::read_volatile((self.base + 0x10) as *const u32)
        }
    }
    fn write(&self, reg: u32, v: u32) {
        unsafe {
            core::ptr::write_volatile(self.base as *mut u32, reg);
            core::ptr::write_volatile((self.base + 0x10) as *mut u32, v);
        }
    }
    fn set_entry(&self, idx: u32, lo: u32, hi: u32) {
        self.write(0x10 + idx * 2, lo | (1 << 16)); // mask while changing
        self.write(0x11 + idx * 2, hi);
        self.write(0x10 + idx * 2, lo);
    }
}

static IOAPICS: SpinLock<Vec<IoApic>> = SpinLock::new(Vec::new());

pub fn init() {
    let info = acpi::info();
    let mut list = IOAPICS.lock();
    for io in info.ioapics.iter() {
        let va = crate::mm::vmm::map_mmio(io.address as u64, 0x1000, false);
        let a = IoApic { base: va, gsi_base: io.gsi_base, count: 0 };
        let count = ((a.read(1) >> 16) & 0xFF) + 1;
        let a = IoApic { count, ..a };
        for i in 0..count {
            a.write(0x10 + i * 2, 1 << 16);
        }
        klog!("ioapic", "id {} at {:#x}: gsi {}..{}", io.id, io.address, io.gsi_base, io.gsi_base + count);
        list.push(a);
    }
}

/// Route an ISA IRQ (0-15) to `vector` on the CPU with `lapic_id`, applying
/// any ACPI interrupt source override.
pub fn route_isa(irq: u8, vector: u8, lapic_id: u32) {
    let info = acpi::info();
    let mut gsi = irq as u32;
    let mut active_low = false;
    let mut level = false;
    for o in info.overrides.iter() {
        if o.source == irq {
            gsi = o.gsi;
            active_low = o.flags & 3 == 3;
            level = (o.flags >> 2) & 3 == 3;
        }
    }
    route_gsi(gsi, vector, lapic_id, active_low, level);
}

pub fn route_gsi(gsi: u32, vector: u8, lapic_id: u32, active_low: bool, level: bool) {
    let list = IOAPICS.lock();
    for a in list.iter() {
        if gsi >= a.gsi_base && gsi < a.gsi_base + a.count {
            let idx = gsi - a.gsi_base;
            let lo = vector as u32 | ((active_low as u32) << 13) | ((level as u32) << 15);
            a.set_entry(idx, lo, lapic_id << 24);
            return;
        }
    }
    klog!("ioapic", "no IOAPIC handles gsi {}", gsi);
}

pub fn mask_gsi(gsi: u32, masked: bool) {
    let list = IOAPICS.lock();
    for a in list.iter() {
        if gsi >= a.gsi_base && gsi < a.gsi_base + a.count {
            let idx = gsi - a.gsi_base;
            let lo = a.read(0x10 + idx * 2);
            a.write(0x10 + idx * 2, if masked { lo | (1 << 16) } else { lo & !(1 << 16) });
        }
    }
}
