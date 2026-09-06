//! i8042 PS/2 controller with keyboard (scancode set 1) and mouse support.

use super::input::{self, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, EV_KEY, EV_REL, REL_WHEEL, REL_X, REL_Y};
use crate::arch::x86_64::interrupts::TrapFrame;
use crate::arch::x86_64::io::{inb, outb};
use crate::sync::SpinLock;

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64;
const CMD: u16 = 0x64;

struct Kbd {
    e0: bool,
}
struct Mouse {
    present: bool,
    wheel: bool,
    packet: [u8; 4],
    idx: usize,
    buttons: u8,
}

static KBD: SpinLock<Kbd> = SpinLock::new(Kbd { e0: false });
static MOUSE: SpinLock<Mouse> = SpinLock::new(Mouse { present: false, wheel: false, packet: [0; 4], idx: 0, buttons: 0 });

fn wait_write() -> bool {
    for _ in 0..100_000 {
        if unsafe { inb(STATUS) } & 2 == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}
fn wait_read() -> bool {
    for _ in 0..100_000 {
        if unsafe { inb(STATUS) } & 1 != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}
fn cmd(c: u8) {
    wait_write();
    unsafe { outb(CMD, c) };
}
fn write_data(d: u8) {
    wait_write();
    unsafe { outb(DATA, d) };
}
fn read_data() -> Option<u8> {
    if wait_read() {
        Some(unsafe { inb(DATA) })
    } else {
        None
    }
}
fn flush() {
    for _ in 0..64 {
        if unsafe { inb(STATUS) } & 1 == 0 {
            break;
        }
        unsafe { inb(DATA) };
    }
}
fn mouse_cmd(c: u8) -> Option<u8> {
    cmd(0xD4);
    write_data(c);
    read_data()
}

pub fn init() {
    // disable devices, flush
    cmd(0xAD);
    cmd(0xA7);
    flush();
    // configuration: disable IRQs and keep scancode translation on
    cmd(0x20);
    let mut cfg = read_data().unwrap_or(0);
    cfg &= !(1 | 2);
    cfg |= 1 << 6;
    cmd(0x60);
    write_data(cfg);
    // controller self test
    cmd(0xAA);
    let st = read_data();
    if st != Some(0x55) {
        klog!("ps2", "controller self-test failed ({:?}); continuing anyway", st);
    }
    // restore config (self-test may reset it)
    cmd(0x60);
    write_data(cfg);
    let dual = cfg & (1 << 5) != 0;
    // enable devices
    cmd(0xAE);
    if dual {
        cmd(0xA8);
    }
    // reset keyboard
    write_data(0xFF);
    let _ = read_data(); // ACK
    let _ = read_data(); // 0xAA
    write_data(0xF4); // enable scanning
    let _ = read_data();
    // mouse
    let mut mouse_ok = false;
    let mut wheel = false;
    if dual {
        if mouse_cmd(0xFF) == Some(0xFA) {
            let _ = read_data(); // 0xAA
            let _ = read_data(); // device id
            mouse_ok = true;
            // try to enable the scroll wheel (IntelliMouse sequence)
            for rate in [200u8, 100, 80] {
                mouse_cmd(0xF3);
                mouse_cmd(rate);
            }
            mouse_cmd(0xF2);
            if let Some(id) = read_data() {
                wheel = id == 3 || id == 4;
            }
            mouse_cmd(0xF3);
            mouse_cmd(100); // sample rate 100
            mouse_cmd(0xE8);
            mouse_cmd(2); // resolution 4 counts/mm
            mouse_cmd(0xF4); // enable reporting
        }
    }
    {
        let mut m = MOUSE.lock();
        m.present = mouse_ok;
        m.wheel = wheel;
    }
    flush();
    // enable interrupts
    cmd(0x20);
    let mut cfg = read_data().unwrap_or(cfg);
    cfg |= 1;
    if mouse_ok {
        cfg |= 2;
    }
    cmd(0x60);
    write_data(cfg);

    use crate::arch::x86_64::{interrupts, ioapic, lapic, VEC_ISA_BASE};
    interrupts::register_handler(VEC_ISA_BASE + 1, kbd_irq);
    ioapic::route_isa(1, VEC_ISA_BASE + 1, lapic::id());
    if mouse_ok {
        interrupts::register_handler(VEC_ISA_BASE + 12, mouse_irq);
        ioapic::route_isa(12, VEC_ISA_BASE + 12, lapic::id());
    }
    flush();
    klog!("ps2", "keyboard ok, mouse {}{}", if mouse_ok { "ok" } else { "absent" }, if wheel { " (wheel)" } else { "" });
}

fn kbd_irq(_f: &mut TrapFrame) {
    let status = unsafe { inb(STATUS) };
    if status & 1 == 0 {
        return;
    }
    let sc = unsafe { inb(DATA) };
    let mut k = KBD.lock();
    if sc == 0xE0 {
        k.e0 = true;
        return;
    }
    if sc == 0xE1 {
        return; // pause key prefix (ignored)
    }
    let pressed = sc & 0x80 == 0;
    let code = sc & 0x7F;
    let key: u16 = if k.e0 {
        k.e0 = false;
        match code {
            0x1C => 96,  // KP enter
            0x1D => 97,  // right ctrl
            0x35 => 98,  // KP slash
            0x37 => 99,  // sysrq
            0x38 => 100, // right alt
            0x47 => 102, // home
            0x48 => 103, // up
            0x49 => 104, // page up
            0x4B => 105, // left
            0x4D => 106, // right
            0x4F => 107, // end
            0x50 => 108, // down
            0x51 => 109, // page down
            0x52 => 110, // insert
            0x53 => 111, // delete
            0x5B => 125, // left meta
            0x5C => 126, // right meta
            0x5D => 127, // menu
            _ => return,
        }
    } else {
        code as u16
    };
    drop(k);
    input::emit(EV_KEY, key, pressed as i32);
    input::sync();
}

fn mouse_irq(_f: &mut TrapFrame) {
    let status = unsafe { inb(STATUS) };
    if status & 1 == 0 {
        return;
    }
    let b = unsafe { inb(DATA) };
    let mut m = MOUSE.lock();
    if m.idx == 0 && b & 0x08 == 0 {
        return; // resync: first byte always has bit 3 set
    }
    let idx = m.idx;
    m.packet[idx] = b;
    m.idx += 1;
    let need = if m.wheel { 4 } else { 3 };
    if m.idx < need {
        return;
    }
    m.idx = 0;
    let p = m.packet;
    let mut dx = p[1] as i32;
    let mut dy = p[2] as i32;
    if p[0] & 0x10 != 0 {
        dx -= 256;
    }
    if p[0] & 0x20 != 0 {
        dy -= 256;
    }
    let buttons = p[0] & 7;
    let old = m.buttons;
    m.buttons = buttons;
    let wheel = if m.wheel { ((p[3] & 0x0F) as i8) << 4 >> 4 } else { 0 };
    drop(m);
    if dx != 0 {
        input::emit(EV_REL, REL_X, dx);
    }
    if dy != 0 {
        input::emit(EV_REL, REL_Y, -dy);
    }
    if wheel != 0 {
        input::emit(EV_REL, REL_WHEEL, -(wheel as i32));
    }
    for (bit, code) in [(1u8, BTN_LEFT), (2, BTN_RIGHT), (4, BTN_MIDDLE)] {
        if (old ^ buttons) & bit != 0 {
            input::emit(EV_KEY, code, ((buttons & bit) != 0) as i32);
        }
    }
    input::sync();
}
