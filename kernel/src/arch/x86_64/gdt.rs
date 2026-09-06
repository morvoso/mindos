//! Global descriptor table and task state segment (one per CPU).

use core::arch::asm;

pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const USER_DS: u16 = 0x18 | 3;
pub const USER_CS: u16 = 0x20 | 3;
pub const TSS_SEL: u16 = 0x28;

#[repr(C, packed)]
pub struct Tss {
    reserved0: u32,
    pub rsp: [u64; 3],
    reserved1: u64,
    pub ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    pub iomap_base: u16,
}

impl Tss {
    pub const fn new() -> Self {
        Tss { reserved0: 0, rsp: [0; 3], reserved1: 0, ist: [0; 7], reserved2: 0, reserved3: 0, iomap_base: core::mem::size_of::<Tss>() as u16 }
    }
}

#[repr(C, align(16))]
pub struct Gdt {
    pub entries: [u64; 7],
}

#[repr(C, packed)]
struct DescriptorPtr {
    limit: u16,
    base: u64,
}

impl Gdt {
    pub const fn new() -> Self {
        Gdt {
            entries: [
                0,
                0x00AF_9A00_0000_FFFF, // kernel code, 64-bit
                0x00CF_9200_0000_FFFF, // kernel data
                0x00CF_F200_0000_FFFF, // user data (DPL3)
                0x00AF_FA00_0000_FFFF, // user code, 64-bit (DPL3)
                0,                     // TSS low
                0,                     // TSS high
            ],
        }
    }

    pub fn set_tss(&mut self, tss: &Tss) {
        let base = tss as *const Tss as u64;
        let limit = (core::mem::size_of::<Tss>() - 1) as u64;
        let low = (limit & 0xFFFF)
            | ((base & 0xFF_FFFF) << 16)
            | (0x89u64 << 40) // present, type = available 64-bit TSS
            | (((limit >> 16) & 0xF) << 48)
            | (((base >> 24) & 0xFF) << 56);
        let high = base >> 32;
        self.entries[5] = low;
        self.entries[6] = high;
    }

    /// Load this GDT, reload the segment registers and the task register.
    /// Caller must (re)program GS base afterwards, as loading GS clears it.
    pub unsafe fn load(&'static self) {
        let ptr = DescriptorPtr { limit: (core::mem::size_of::<Gdt>() - 1) as u16, base: self as *const Gdt as u64 };
        unsafe {
            asm!(
                "lgdt [{ptr}]",
                "push {cs}",
                "lea {tmp}, [rip + 2f]",
                "push {tmp}",
                "retfq",
                "2:",
                "mov ds, {ds:x}",
                "mov es, {ds:x}",
                "mov ss, {ds:x}",
                "xor eax, eax",
                "mov fs, ax",
                "mov gs, ax",
                "ltr {tss:x}",
                ptr = in(reg) &ptr,
                cs = const KERNEL_CS as u64,
                ds = in(reg) KERNEL_DS,
                tss = in(reg) TSS_SEL,
                tmp = out(reg) _,
                out("rax") _,
                options(nostack)
            );
        }
    }
}
