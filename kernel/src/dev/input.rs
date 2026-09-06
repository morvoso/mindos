//! Input event queue (Linux evdev-compatible `input_event` records) fed by the
//! keyboard/mouse drivers and consumed by `/dev/input/event0` and the console tty.

use crate::sync::SpinLock;
use core::sync::atomic::{AtomicUsize, Ordering};

pub const EV_SYN: u16 = 0;
pub const EV_KEY: u16 = 1;
pub const EV_REL: u16 = 2;
pub const REL_X: u16 = 0;
pub const REL_Y: u16 = 1;
pub const REL_WHEEL: u16 = 8;
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct InputEvent {
    pub tv_sec: u64,
    pub tv_usec: u64,
    pub typ: u16,
    pub code: u16,
    pub value: i32,
}

const QUEUE: usize = 512;

pub struct EventQueue {
    buf: [InputEvent; QUEUE],
    head: usize,
    tail: usize,
}

pub static EVENTS: SpinLock<EventQueue> = SpinLock::new(EventQueue { buf: [InputEvent { tv_sec: 0, tv_usec: 0, typ: 0, code: 0, value: 0 }; QUEUE], head: 0, tail: 0 });

/// Hook invoked (in IRQ context) after events are queued, so waiting readers can be woken.
static WAKER: AtomicUsize = AtomicUsize::new(0);
/// Hook for the kernel console tty: receives key events (code, pressed).
static CONSOLE_KEY_HOOK: AtomicUsize = AtomicUsize::new(0);

pub fn set_waker(f: fn()) {
    WAKER.store(f as usize, Ordering::Release);
}
pub fn set_console_key_hook(f: fn(u16, bool)) {
    CONSOLE_KEY_HOOK.store(f as usize, Ordering::Release);
}

impl EventQueue {
    pub fn push(&mut self, ev: InputEvent) {
        if self.head - self.tail >= QUEUE {
            self.tail += 1; // drop oldest
        }
        self.buf[self.head % QUEUE] = ev;
        self.head += 1;
    }
    pub fn pop(&mut self) -> Option<InputEvent> {
        if self.head == self.tail {
            return None;
        }
        let e = self.buf[self.tail % QUEUE];
        self.tail += 1;
        Some(e)
    }
    pub fn len(&self) -> usize {
        self.head - self.tail
    }
}

fn now() -> (u64, u64) {
    let ns = crate::arch::x86_64::tsc::uptime_ns();
    (ns / 1_000_000_000, (ns % 1_000_000_000) / 1000)
}

pub fn emit(typ: u16, code: u16, value: i32) {
    let (s, us) = now();
    EVENTS.lock().push(InputEvent { tv_sec: s, tv_usec: us, typ, code, value });
    if typ == EV_KEY {
        let h = CONSOLE_KEY_HOOK.load(Ordering::Acquire);
        if h != 0 {
            let f: fn(u16, bool) = unsafe { core::mem::transmute(h) };
            f(code, value != 0);
        }
    }
}

pub fn sync() {
    emit(EV_SYN, 0, 0);
    let w = WAKER.load(Ordering::Acquire);
    if w != 0 {
        let f: fn() = unsafe { core::mem::transmute(w) };
        f();
    }
}

pub fn pending() -> usize {
    EVENTS.lock().len()
}

/// US keyboard layout: keycode -> (plain, shifted) ASCII for the console tty.
pub fn keycode_to_ascii(code: u16, shift: bool, ctrl: bool, caps: bool) -> Option<u8> {
    const MAP: [(u16, u8, u8); 58] = [
        (2, b'1', b'!'), (3, b'2', b'@'), (4, b'3', b'#'), (5, b'4', b'$'), (6, b'5', b'%'), (7, b'6', b'^'),
        (8, b'7', b'&'), (9, b'8', b'*'), (10, b'9', b'('), (11, b'0', b')'), (12, b'-', b'_'), (13, b'=', b'+'),
        (14, 0x7f, 0x7f), (15, b'\t', b'\t'), (16, b'q', b'Q'), (17, b'w', b'W'), (18, b'e', b'E'), (19, b'r', b'R'),
        (20, b't', b'T'), (21, b'y', b'Y'), (22, b'u', b'U'), (23, b'i', b'I'), (24, b'o', b'O'), (25, b'p', b'P'),
        (26, b'[', b'{'), (27, b']', b'}'), (28, b'\n', b'\n'), (30, b'a', b'A'), (31, b's', b'S'), (32, b'd', b'D'),
        (33, b'f', b'F'), (34, b'g', b'G'), (35, b'h', b'H'), (36, b'j', b'J'), (37, b'k', b'K'), (38, b'l', b'L'),
        (39, b';', b':'), (40, b'\'', b'"'), (41, b'`', b'~'), (43, b'\\', b'|'), (44, b'z', b'Z'), (45, b'x', b'X'),
        (46, b'c', b'C'), (47, b'v', b'V'), (48, b'b', b'B'), (49, b'n', b'N'), (50, b'm', b'M'), (51, b',', b'<'),
        (52, b'.', b'>'), (53, b'/', b'?'), (57, b' ', b' '), (1, 0x1b, 0x1b), (96, b'\n', b'\n'), (98, b'/', b'/'),
        (55, b'*', b'*'), (74, b'-', b'-'), (78, b'+', b'+'), (111, 0x7f, 0x7f),
    ];
    for &(k, plain, shifted) in MAP.iter() {
        if k == code {
            let mut c = if shift { shifted } else { plain };
            if caps && plain.is_ascii_alphabetic() {
                c = if shift { plain } else { shifted };
            }
            if ctrl && c.is_ascii_alphabetic() {
                return Some(c.to_ascii_uppercase() - b'@');
            }
            if ctrl && c == b'[' {
                return Some(0x1b);
            }
            return Some(c);
        }
    }
    None
}
