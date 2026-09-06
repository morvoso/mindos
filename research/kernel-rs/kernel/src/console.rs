//! Kernel console: every message goes to the serial port, the framebuffer
//! console (once it exists) and an in-memory ring buffer (`/dev/kmsg`, dmesg).

use crate::sync::SpinLock;
use core::fmt::{self, Write};

const LOG_SIZE: usize = 128 * 1024;

pub struct LogRing {
    buf: [u8; LOG_SIZE],
    head: usize, // total bytes ever written
}

impl LogRing {
    const fn new() -> Self {
        LogRing { buf: [0; LOG_SIZE], head: 0 }
    }
    fn push(&mut self, s: &[u8]) {
        for &b in s {
            self.buf[self.head % LOG_SIZE] = b;
            self.head += 1;
        }
    }
    /// Copy log contents starting at absolute offset `from` into `out`.
    /// Returns (bytes copied, new absolute offset).
    pub fn read_from(&self, from: usize, out: &mut [u8]) -> (usize, usize) {
        let start = if self.head > LOG_SIZE { self.head - LOG_SIZE } else { 0 };
        let mut pos = from.max(start);
        let mut n = 0;
        while pos < self.head && n < out.len() {
            out[n] = self.buf[pos % LOG_SIZE];
            pos += 1;
            n += 1;
        }
        (n, pos)
    }
    pub fn len(&self) -> usize {
        self.head
    }
}

pub struct Console {
    pub log: LogRing,
    pub serial: bool,
    pub screen: bool,
}

pub static CONSOLE: SpinLock<Console> = SpinLock::new(Console { log: LogRing::new(), serial: false, screen: false });

pub fn write_bytes(s: &[u8]) {
    let mut c = CONSOLE.lock();
    c.log.push(s);
    if c.serial {
        crate::dev::serial::write(s);
    }
    if c.screen {
        crate::dev::fbcon::write(s);
    }
}

/// Write user-visible output to the console devices without logging it.
pub fn write_output(s: &[u8]) {
    let c = CONSOLE.lock();
    if c.serial {
        crate::dev::serial::write(s);
    }
    if c.screen {
        crate::dev::fbcon::write(s);
    }
}

struct Writer;
impl Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    let _ = Writer.write_fmt(args);
}

/// Used by the panic handler: never blocks on the console lock.
pub fn print_unlocked(args: fmt::Arguments) {
    unsafe {
        if CONSOLE.is_locked() {
            CONSOLE.force_unlock();
        }
    }
    print(args);
}

pub fn enable_serial() {
    CONSOLE.lock().serial = true;
}
pub fn set_screen(on: bool) {
    CONSOLE.lock().screen = on;
}
pub fn screen_enabled() -> bool {
    CONSOLE.lock().screen
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::console::print(format_args!($($arg)*)) };
}
#[macro_export]
macro_rules! kprintln {
    () => { $crate::console::print(format_args!("\n")) };
    ($($arg:tt)*) => {{ $crate::console::print(format_args!($($arg)*)); $crate::console::print(format_args!("\n")); }};
}
/// Log with a subsystem tag and the current uptime.
#[macro_export]
macro_rules! klog {
    ($tag:expr, $($arg:tt)*) => {{
        let ns = $crate::arch::x86_64::tsc::uptime_ns();
        $crate::console::print(format_args!("[{:5}.{:06}] {}: ", ns / 1_000_000_000, (ns % 1_000_000_000) / 1000, $tag));
        $crate::console::print(format_args!($($arg)*));
        $crate::console::print(format_args!("\n"));
    }};
}
