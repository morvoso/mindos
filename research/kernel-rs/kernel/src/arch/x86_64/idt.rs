//! Interrupt descriptor table (shared by all CPUs).

use core::arch::asm;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_lo: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_hi: u32,
    zero: u32,
}

impl IdtEntry {
    const fn empty() -> Self {
        IdtEntry { offset_lo: 0, selector: 0, ist: 0, type_attr: 0, offset_mid: 0, offset_hi: 0, zero: 0 }
    }
    fn set(&mut self, handler: u64, ist: u8, dpl: u8) {
        self.offset_lo = handler as u16;
        self.offset_mid = (handler >> 16) as u16;
        self.offset_hi = (handler >> 32) as u32;
        self.selector = super::gdt::KERNEL_CS;
        self.ist = ist;
        self.type_attr = 0x8E | ((dpl & 3) << 5); // present, interrupt gate
    }
}

#[repr(C, align(16))]
struct Idt([IdtEntry; 256]);

static mut IDT: Idt = Idt([IdtEntry::empty(); 256]);

#[repr(C, packed)]
struct DescriptorPtr {
    limit: u16,
    base: u64,
}

extern "C" {
    static isr_table: [u64; 256];
}

pub const IST_DOUBLE_FAULT: u8 = 1;
pub const IST_NMI: u8 = 2;

pub fn init() {
    unsafe {
        let idt = &mut *core::ptr::addr_of_mut!(IDT);
        for (i, e) in idt.0.iter_mut().enumerate() {
            let ist = match i {
                8 => IST_DOUBLE_FAULT,
                2 => IST_NMI,
                _ => 0,
            };
            e.set(isr_table[i], ist, 0);
        }
    }
}

pub fn load() {
    let ptr = DescriptorPtr { limit: (core::mem::size_of::<Idt>() - 1) as u16, base: core::ptr::addr_of!(IDT) as u64 };
    unsafe { asm!("lidt [{}]", in(reg) &ptr, options(nostack, preserves_flags)) };
}

/// Load an empty IDT so the next interrupt triple-faults (reboot path).
pub fn load_empty() {
    let ptr = DescriptorPtr { limit: 0, base: 0 };
    unsafe { asm!("lidt [{}]", in(reg) &ptr, options(nostack, preserves_flags)) };
}
