//! 16550 UART driver for COM1 (the kernel log and a serial tty).

use crate::arch::x86_64::io::{inb, outb};
use core::sync::atomic::{AtomicBool, Ordering};

pub const COM1: u16 = 0x3F8;
static PRESENT: AtomicBool = AtomicBool::new(false);

pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00); // disable interrupts
        outb(COM1 + 3, 0x80); // DLAB on
        outb(COM1 + 0, 0x01); // 115200 baud
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8N1
        outb(COM1 + 2, 0xC7); // FIFO on, clear, 14-byte threshold
        outb(COM1 + 4, 0x0B); // DTR, RTS, OUT2
        // loopback self-test
        outb(COM1 + 4, 0x1E);
        outb(COM1 + 0, 0xAE);
        let ok = inb(COM1 + 0) == 0xAE;
        outb(COM1 + 4, 0x0F);
        PRESENT.store(ok, Ordering::Relaxed);
    }
}

pub fn present() -> bool {
    PRESENT.load(Ordering::Relaxed)
}

#[inline]
fn tx_ready() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

pub fn write_byte(b: u8) {
    if !present() {
        return;
    }
    let mut spins = 0u32;
    while !tx_ready() {
        spins += 1;
        if spins > 1_000_000 {
            return;
        }
        core::hint::spin_loop();
    }
    unsafe { outb(COM1, b) };
}

pub fn write(s: &[u8]) {
    for &b in s {
        if b == b'\n' {
            write_byte(b'\r');
        }
        write_byte(b);
    }
}

pub fn try_read() -> Option<u8> {
    if !present() {
        return None;
    }
    unsafe {
        if inb(COM1 + 5) & 1 != 0 {
            Some(inb(COM1))
        } else {
            None
        }
    }
}

/// Enable the "received data available" interrupt (IRQ4).
pub fn enable_rx_irq() {
    unsafe { outb(COM1 + 1, 0x01) };
}
