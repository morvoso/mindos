//! Port-mapped I/O.
use core::arch::asm;

#[inline(always)]
pub unsafe fn outb(port: u16, v: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags)) };
}
#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe { asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags)) };
    v
}
#[inline(always)]
pub unsafe fn outw(port: u16, v: u16) {
    unsafe { asm!("out dx, ax", in("dx") port, in("ax") v, options(nomem, nostack, preserves_flags)) };
}
#[inline(always)]
pub unsafe fn inw(port: u16) -> u16 {
    let v: u16;
    unsafe { asm!("in ax, dx", out("ax") v, in("dx") port, options(nomem, nostack, preserves_flags)) };
    v
}
#[inline(always)]
pub unsafe fn outl(port: u16, v: u32) {
    unsafe { asm!("out dx, eax", in("dx") port, in("eax") v, options(nomem, nostack, preserves_flags)) };
}
#[inline(always)]
pub unsafe fn inl(port: u16) -> u32 {
    let v: u32;
    unsafe { asm!("in eax, dx", out("eax") v, in("dx") port, options(nomem, nostack, preserves_flags)) };
    v
}
#[inline(always)]
pub fn io_wait() {
    unsafe { outb(0x80, 0) };
}
/// Read `count` 16-bit words from a port into a buffer (ATA PIO).
pub unsafe fn insw(port: u16, buf: *mut u16, count: usize) {
    unsafe { asm!("rep insw", in("dx") port, inout("rdi") buf => _, inout("rcx") count => _, options(nostack, preserves_flags)) };
}
pub unsafe fn outsw(port: u16, buf: *const u16, count: usize) {
    unsafe { asm!("rep outsw", in("dx") port, inout("rsi") buf => _, inout("rcx") count => _, options(nostack, preserves_flags)) };
}
