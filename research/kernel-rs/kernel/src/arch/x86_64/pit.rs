//! 8254 programmable interval timer, used only to calibrate the APIC timer and TSC.

use super::io::{inb, outb};

pub const PIT_HZ: u64 = 1_193_182;

/// Busy-wait `ms` milliseconds (max ~50) using PIT channel 2 in one-shot mode.
pub fn wait_ms(ms: u32) {
    let count = (PIT_HZ * ms as u64 / 1000) as u16;
    unsafe {
        // gate channel 2 on, speaker off
        outb(0x61, (inb(0x61) & 0xFD) | 0x01);
        outb(0x43, 0xB0); // channel 2, lo/hi byte, mode 0
        outb(0x42, count as u8);
        outb(0x42, (count >> 8) as u8);
        // restart counting: pulse the gate
        let g = inb(0x61) & 0xFE;
        outb(0x61, g);
        outb(0x61, g | 1);
        while inb(0x61) & 0x20 == 0 {
            core::hint::spin_loop();
        }
    }
}
