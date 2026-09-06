//! Model-specific registers.
use core::arch::asm;

pub const IA32_APIC_BASE: u32 = 0x1B;
pub const IA32_PAT: u32 = 0x277;
pub const IA32_TSC_DEADLINE: u32 = 0x6E0;
pub const IA32_EFER: u32 = 0xC000_0080;
pub const IA32_STAR: u32 = 0xC000_0081;
pub const IA32_LSTAR: u32 = 0xC000_0082;
pub const IA32_CSTAR: u32 = 0xC000_0083;
pub const IA32_FMASK: u32 = 0xC000_0084;
pub const IA32_FS_BASE: u32 = 0xC000_0100;
pub const IA32_GS_BASE: u32 = 0xC000_0101;
pub const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;

#[inline]
pub fn read(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    ((hi as u64) << 32) | lo as u64
}

#[inline]
pub fn write(msr: u32, v: u64) {
    let lo = v as u32;
    let hi = (v >> 32) as u32;
    unsafe { asm!("wrmsr", in("ecx") msr, in("eax") lo, in("edx") hi, options(nomem, nostack, preserves_flags)) };
}
