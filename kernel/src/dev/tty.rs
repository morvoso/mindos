//! TTY layer: line discipline, the system console, and pseudo-terminals.

use crate::fs::vfs::{self, File, FileOps, Kind, POLLHUP, POLLIN, POLLOUT};
use crate::mm::errno::*;
use crate::mm::user::{copy_from_user, copy_to_user, read_user, write_user};
use crate::sched::wait::WaitQueue;
use crate::sync::SpinLock;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

// c_iflag
pub const IGNBRK: u32 = 0o1;
pub const BRKINT: u32 = 0o2;
pub const IGNCR: u32 = 0o200;
pub const ICRNL: u32 = 0o400;
pub const INLCR: u32 = 0o100;
pub const IXON: u32 = 0o2000;
// c_oflag
pub const OPOST: u32 = 0o1;
pub const ONLCR: u32 = 0o4;
pub const OCRNL: u32 = 0o10;
pub const ONLRET: u32 = 0o40;
// c_lflag
pub const ISIG: u32 = 0o1;
pub const ICANON: u32 = 0o2;
pub const ECHO: u32 = 0o10;
pub const ECHOE: u32 = 0o20;
pub const ECHOK: u32 = 0o40;
pub const ECHONL: u32 = 0o100;
pub const NOFLSH: u32 = 0o200;
pub const TOSTOP: u32 = 0o400;
pub const ECHOCTL: u32 = 0o1000;
pub const IEXTEN: u32 = 0o100000;
// c_cc indices
pub const VINTR: usize = 0;
pub const VQUIT: usize = 1;
pub const VERASE: usize = 2;
pub const VKILL: usize = 3;
pub const VEOF: usize = 4;
pub const VTIME: usize = 5;
pub const VMIN: usize = 6;
pub const VSTART: usize = 8;
pub const VSTOP: usize = 9;
pub const VSUSP: usize = 10;
pub const VEOL: usize = 11;
pub const VREPRINT: usize = 12;
pub const VWERASE: usize = 14;
pub const VLNEXT: usize = 15;
pub const VEOL2: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Termios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_line: u8,
    pub c_cc: [u8; 19],
}

impl Termios {
    pub const fn default_tty() -> Termios {
        let mut cc = [0u8; 19];
        cc[VINTR] = 3;
        cc[VQUIT] = 28;
        cc[VERASE] = 127;
        cc[VKILL] = 21;
        cc[VEOF] = 4;
        cc[VTIME] = 0;
        cc[VMIN] = 1;
        cc[VSTART] = 17;
        cc[VSTOP] = 19;
        cc[VSUSP] = 26;
        cc[VREPRINT] = 18;
        cc[VWERASE] = 23;
        cc[VLNEXT] = 22;
        Termios {
            c_iflag: ICRNL | IXON | BRKINT,
            c_oflag: OPOST | ONLCR,
            c_cflag: 0o17 | 0o60 | 0o200 | 0o2000, // B38400 CS8 CREAD HUPCL
            c_lflag: ISIG | ICANON | ECHO | ECHOE | ECHOK | ECHOCTL | IEXTEN,
            c_line: 0,
            c_cc: cc,
        }
    }
    fn canon(&self) -> bool {
        self.c_lflag & ICANON != 0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Winsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

struct TtyInput {
    /// Line being edited (canonical mode).
    line: Vec<u8>,
    /// Completed lines (canonical mode); an empty line means EOF.
    lines: VecDeque<Vec<u8>>,
    /// Raw bytes (non-canonical mode).
    raw: VecDeque<u8>,
    lnext: bool,
}

pub type OutputSink = dyn Fn(&[u8]) -> bool + Send + Sync;

pub struct Tty {
    pub name: String,
    pub index: u32,
    pub termios: SpinLock<Termios>,
    pub winsize: SpinLock<Winsize>,
    input: SpinLock<TtyInput>,
    pub read_wq: WaitQueue,
    pub write_wq: WaitQueue,
    pub fg_pgrp: AtomicU32,
    pub session: AtomicU32,
    pub hangup: AtomicBool,
    sink: SpinLock<Option<Arc<OutputSink>>>,
    pub openers: AtomicUsize,
    pub is_pty_slave: bool,
}

impl Tty {
    pub fn new(name: &str, index: u32, is_pty_slave: bool) -> Arc<Tty> {
        Arc::new(Tty {
            name: String::from(name),
            index,
            termios: SpinLock::new(Termios::default_tty()),
            winsize: SpinLock::new(Winsize { ws_row: 25, ws_col: 80, ws_xpixel: 0, ws_ypixel: 0 }),
            input: SpinLock::new(TtyInput { line: Vec::new(), lines: VecDeque::new(), raw: VecDeque::new(), lnext: false }),
            read_wq: WaitQueue::new(),
            write_wq: WaitQueue::new(),
            fg_pgrp: AtomicU32::new(0),
            session: AtomicU32::new(0),
            hangup: AtomicBool::new(false),
            sink: SpinLock::new(None),
            openers: AtomicUsize::new(0),
            is_pty_slave,
        })
    }

    pub fn set_sink(&self, s: Arc<OutputSink>) {
        *self.sink.lock() = Some(s);
    }

    fn sink(&self) -> Option<Arc<OutputSink>> {
        self.sink.lock().clone()
    }

    /// Output processing (OPOST) then hand to the sink. Returns false if the sink is full.
    fn output_raw(&self, data: &[u8]) -> bool {
        match self.sink() {
            Some(s) => s(data),
            None => true,
        }
    }

    fn output_processed(&self, data: &[u8], t: &Termios) -> bool {
        if t.c_oflag & OPOST == 0 {
            return self.output_raw(data);
        }
        let mut buf: Vec<u8> = Vec::with_capacity(data.len() + 16);
        for &b in data {
            match b {
                b'\n' if t.c_oflag & ONLCR != 0 => {
                    buf.push(b'\r');
                    buf.push(b'\n');
                }
                b'\r' if t.c_oflag & OCRNL != 0 => buf.push(b'\n'),
                _ => buf.push(b),
            }
        }
        self.output_raw(&buf)
    }

    fn echo(&self, bytes: &[u8], t: &Termios) {
        let _ = self.output_processed(bytes, t);
    }

    fn echo_char(&self, b: u8, t: &Termios) {
        if t.c_lflag & ECHO == 0 {
            if b == b'\n' && t.c_lflag & ECHONL != 0 {
                self.echo(b"\n", t);
            }
            return;
        }
        if b < 0x20 && b != b'\n' && b != b'\t' && t.c_lflag & ECHOCTL != 0 {
            self.echo(&[b'^', b + 0x40], t);
        } else if b == 0x7f && t.c_lflag & ECHOCTL != 0 {
            self.echo(b"^?", t);
        } else {
            self.echo(&[b], t);
        }
    }

    /// Feed input bytes (from an interrupt handler or a pty master).
    pub fn input_bytes(&self, data: &[u8]) {
        let t = *self.termios.lock();
        let mut wake = false;
        for &raw in data {
            let mut b = raw;
            // input translation
            if t.c_iflag & ICRNL != 0 && b == b'\r' {
                b = b'\n';
            } else if t.c_iflag & IGNCR != 0 && b == b'\r' {
                continue;
            } else if t.c_iflag & INLCR != 0 && b == b'\n' {
                b = b'\r';
            }
            let mut inp = self.input.lock();
            let lnext = inp.lnext;
            inp.lnext = false;
            if !lnext {
                if t.c_lflag & ISIG != 0 {
                    let signo = if b == t.c_cc[VINTR] {
                        2
                    } else if b == t.c_cc[VQUIT] {
                        3
                    } else if b == t.c_cc[VSUSP] {
                        20
                    } else {
                        0
                    };
                    if signo != 0 && b != 0 {
                        drop(inp);
                        self.echo_char(b, &t);
                        if t.c_lflag & NOFLSH == 0 {
                            let mut inp = self.input.lock();
                            inp.line.clear();
                            inp.raw.clear();
                        }
                        let pg = self.fg_pgrp.load(Ordering::Relaxed);
                        if pg != 0 {
                            crate::proc::signal::send_to_pgrp(pg, signo);
                        }
                        continue;
                    }
                }
                if t.c_iflag & IXON != 0 && (b == t.c_cc[VSTART] || b == t.c_cc[VSTOP]) && b != 0 {
                    continue; // flow control ignored
                }
                if t.c_lflag & IEXTEN != 0 && b == t.c_cc[VLNEXT] && b != 0 {
                    inp.lnext = true;
                    continue;
                }
            }
            if t.canon() {
                if !lnext {
                    if b == t.c_cc[VERASE] && b != 0 {
                        if let Some(c) = inp.line.pop() {
                            drop(inp);
                            if t.c_lflag & ECHOE != 0 && t.c_lflag & ECHO != 0 {
                                if c < 0x20 && t.c_lflag & ECHOCTL != 0 {
                                    self.echo(b"\x08 \x08\x08 \x08", &t);
                                } else {
                                    self.echo(b"\x08 \x08", &t);
                                }
                            }
                        }
                        continue;
                    }
                    if b == t.c_cc[VWERASE] && b != 0 && t.c_lflag & IEXTEN != 0 {
                        let mut n = 0;
                        while let Some(&c) = inp.line.last() {
                            if c == b' ' && n == 0 || c != b' ' {
                                inp.line.pop();
                                n += 1;
                                if c == b' ' {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        drop(inp);
                        if t.c_lflag & ECHOE != 0 && t.c_lflag & ECHO != 0 {
                            for _ in 0..n {
                                self.echo(b"\x08 \x08", &t);
                            }
                        }
                        continue;
                    }
                    if b == t.c_cc[VKILL] && b != 0 {
                        let n = inp.line.len();
                        inp.line.clear();
                        drop(inp);
                        if t.c_lflag & ECHOK != 0 && t.c_lflag & ECHO != 0 {
                            for _ in 0..n {
                                self.echo(b"\x08 \x08", &t);
                            }
                        }
                        continue;
                    }
                    if b == t.c_cc[VEOF] && b != 0 {
                        let line = core::mem::take(&mut inp.line);
                        inp.lines.push_back(line);
                        wake = true;
                        continue;
                    }
                    if b == t.c_cc[VREPRINT] && b != 0 {
                        let line = inp.line.clone();
                        drop(inp);
                        self.echo(b"^R\n", &t);
                        self.echo(&line, &t);
                        continue;
                    }
                }
                let eol = b == b'\n' || (b != 0 && (b == t.c_cc[VEOL] || b == t.c_cc[VEOL2]));
                inp.line.push(b);
                if eol {
                    let line = core::mem::take(&mut inp.line);
                    inp.lines.push_back(line);
                    wake = true;
                } else if inp.line.len() >= 4095 {
                    // line too long: flush as-is
                    let line = core::mem::take(&mut inp.line);
                    inp.lines.push_back(line);
                    wake = true;
                }
                drop(inp);
                self.echo_char(b, &t);
            } else {
                if inp.raw.len() < 65536 {
                    inp.raw.push_back(b);
                }
                wake = true;
                drop(inp);
                self.echo_char(b, &t);
            }
        }
        if wake {
            self.read_wq.wake_all();
        }
    }

    fn readable_now(&self) -> bool {
        if self.hangup.load(Ordering::Relaxed) {
            return true;
        }
        let t = self.termios.lock();
        let inp = self.input.lock();
        if t.canon() {
            !inp.lines.is_empty()
        } else {
            let min = t.c_cc[VMIN] as usize;
            inp.raw.len() >= min.max(1) || (min == 0 && !inp.raw.is_empty())
        }
    }

    pub fn read(&self, file: &File, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.hangup.load(Ordering::Relaxed) {
                return Ok(0);
            }
            let t = *self.termios.lock();
            {
                let mut inp = self.input.lock();
                if t.canon() {
                    if let Some(mut line) = inp.lines.pop_front() {
                        let n = line.len().min(buf.len());
                        buf[..n].copy_from_slice(&line[..n]);
                        if n < line.len() {
                            line.drain(..n);
                            inp.lines.push_front(line);
                        }
                        return Ok(n);
                    }
                } else {
                    let min = t.c_cc[VMIN] as usize;
                    let time = t.c_cc[VTIME] as u64;
                    if !inp.raw.is_empty() && (inp.raw.len() >= min || min == 0) {
                        let n = inp.raw.len().min(buf.len());
                        for i in 0..n {
                            buf[i] = inp.raw.pop_front().unwrap();
                        }
                        return Ok(n);
                    }
                    if min == 0 && time == 0 {
                        return Ok(0);
                    }
                    if min == 0 && time > 0 {
                        drop(inp);
                        let deadline = crate::arch::x86_64::tsc::uptime_ns() + time * 100_000_000;
                        match self.read_wq.wait_until_deadline(|| self.readable_now() || !self.input.lock().raw.is_empty(), deadline) {
                            Ok(_) => continue,
                            Err(_) => return Err(EINTR),
                        }
                    }
                }
            }
            if file.nonblock() {
                return Err(EAGAIN);
            }
            self.read_wq.wait_until(|| self.readable_now()).map_err(|_| EINTR)?;
        }
    }

    pub fn write(&self, file: &File, buf: &[u8]) -> Result<usize> {
        if self.hangup.load(Ordering::Relaxed) {
            return Err(EIO);
        }
        let t = *self.termios.lock();
        // write in chunks; the sink may apply back-pressure (pty)
        let mut off = 0;
        while off < buf.len() {
            let end = (off + 4096).min(buf.len());
            if self.output_processed(&buf[off..end], &t) {
                off = end;
            } else {
                if file.nonblock() {
                    return if off > 0 { Ok(off) } else { Err(EAGAIN) };
                }
                self.write_wq.wait_until(|| self.hangup.load(Ordering::Relaxed) || self.sink_has_room()).map_err(|_| EINTR)?;
                if self.hangup.load(Ordering::Relaxed) {
                    return Err(EIO);
                }
            }
        }
        Ok(buf.len())
    }

    fn sink_has_room(&self) -> bool {
        match self.sink() {
            Some(s) => s(&[]),
            None => true,
        }
    }

    pub fn poll(&self) -> u32 {
        let mut m = 0;
        if self.readable_now() {
            m |= POLLIN;
        }
        if self.sink_has_room() {
            m |= POLLOUT;
        }
        if self.hangup.load(Ordering::Relaxed) {
            m |= POLLHUP;
        }
        m
    }

    pub fn flush_input(&self) {
        let mut inp = self.input.lock();
        inp.line.clear();
        inp.lines.clear();
        inp.raw.clear();
    }

    pub fn do_hangup(&self) {
        self.hangup.store(true, Ordering::Relaxed);
        self.read_wq.wake_all();
        self.write_wq.wake_all();
        let pg = self.fg_pgrp.load(Ordering::Relaxed);
        if pg != 0 {
            crate::proc::signal::send_to_pgrp(pg, 1); // SIGHUP
        }
    }

    pub fn ioctl(self: &Arc<Tty>, _file: &File, cmd: u32, arg: usize) -> Result<usize> {
        match cmd {
            0x5401 => {
                // TCGETS
                let t = *self.termios.lock();
                let bytes = unsafe { core::slice::from_raw_parts(&t as *const _ as *const u8, 36) };
                copy_to_user(arg, bytes)?;
                Ok(0)
            }
            0x5402 | 0x5403 | 0x5404 => {
                // TCSETS / TCSETSW / TCSETSF
                let mut t = Termios::default_tty();
                let bytes = unsafe { core::slice::from_raw_parts_mut(&mut t as *mut _ as *mut u8, 36) };
                copy_from_user(bytes, arg)?;
                let was_canon = self.termios.lock().canon();
                *self.termios.lock() = t;
                if cmd == 0x5404 {
                    self.flush_input();
                } else if was_canon && !t.canon() {
                    // move any partial line to the raw queue
                    let mut inp = self.input.lock();
                    let line = core::mem::take(&mut inp.line);
                    let lines: Vec<Vec<u8>> = inp.lines.drain(..).collect();
                    for l in lines {
                        inp.raw.extend(l);
                    }
                    inp.raw.extend(line);
                }
                self.read_wq.wake_all();
                Ok(0)
            }
            0x540B => {
                // TCFLSH
                self.flush_input();
                Ok(0)
            }
            0x5409 | 0x540A => Ok(0), // TCSBRK / TCXONC
            0x540E => {
                // TIOCSCTTY
                crate::proc::process::set_controlling_tty(self.clone(), true);
                Ok(0)
            }
            0x540F => {
                // TIOCGPGRP
                write_user::<u32>(arg, self.fg_pgrp.load(Ordering::Relaxed))?;
                Ok(0)
            }
            0x5410 => {
                // TIOCSPGRP
                let pg: u32 = read_user(arg)?;
                self.fg_pgrp.store(pg, Ordering::Relaxed);
                Ok(0)
            }
            0x5413 => {
                // TIOCGWINSZ
                let w = *self.winsize.lock();
                let bytes = unsafe { core::slice::from_raw_parts(&w as *const _ as *const u8, 8) };
                copy_to_user(arg, bytes)?;
                Ok(0)
            }
            0x5414 => {
                // TIOCSWINSZ
                let mut w = Winsize::default();
                let bytes = unsafe { core::slice::from_raw_parts_mut(&mut w as *mut _ as *mut u8, 8) };
                copy_from_user(bytes, arg)?;
                *self.winsize.lock() = w;
                let pg = self.fg_pgrp.load(Ordering::Relaxed);
                if pg != 0 {
                    crate::proc::signal::send_to_pgrp(pg, 28); // SIGWINCH
                }
                Ok(0)
            }
            0x541B => {
                // FIONREAD
                let n = {
                    let inp = self.input.lock();
                    inp.raw.len() + inp.lines.iter().map(|l| l.len()).sum::<usize>()
                };
                write_user::<i32>(arg, n as i32)?;
                Ok(0)
            }
            0x5421 => {
                // FIONBIO
                let on: i32 = read_user(arg)?;
                let f = _file.flags();
                _file.set_flags(if on != 0 { f | vfs::O_NONBLOCK } else { f & !vfs::O_NONBLOCK });
                Ok(0)
            }
            0x5422 => {
                // TIOCNOTTY
                crate::proc::process::drop_controlling_tty();
                Ok(0)
            }
            0x5429 => {
                // TIOCGSID
                write_user::<u32>(arg, self.session.load(Ordering::Relaxed))?;
                Ok(0)
            }
            0x5423 | 0x5424 => Ok(0), // TIOCSETD / TIOCGETD
            0x80045430 => {
                // TIOCGPTN
                if !self.is_pty_slave {
                    return Err(ENOTTY);
                }
                write_user::<u32>(arg, self.index)?;
                Ok(0)
            }
            _ => Err(ENOTTY),
        }
    }
}

/// A file open on a tty (console or pty slave).
pub struct TtyFile {
    pub tty: Arc<Tty>,
}

impl FileOps for TtyFile {
    fn read(&self, f: &File, b: &mut [u8]) -> Result<usize> {
        self.tty.read(f, b)
    }
    fn write(&self, f: &File, b: &[u8]) -> Result<usize> {
        self.tty.write(f, b)
    }
    fn poll(&self, _f: &File) -> u32 {
        self.tty.poll()
    }
    fn poll_wait(&self) -> Option<&WaitQueue> {
        Some(&self.tty.read_wq)
    }
    fn ioctl(&self, f: &File, cmd: u32, arg: usize) -> Result<usize> {
        self.tty.ioctl(f, cmd, arg)
    }
    fn release(&self, _f: &File) {
        self.tty.openers.fetch_sub(1, Ordering::AcqRel);
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

fn open_tty(tty: Arc<Tty>, flags: u32) -> Result<Arc<dyn FileOps>> {
    tty.openers.fetch_add(1, Ordering::AcqRel);
    if flags & vfs::O_NOCTTY == 0 {
        crate::proc::process::set_controlling_tty(tty.clone(), false);
    }
    Ok(Arc::new(TtyFile { tty }))
}

// ---- the system console -------------------------------------------------------

static CONSOLE_TTY: crate::sync::Once<Arc<Tty>> = crate::sync::Once::new();

pub fn console() -> Arc<Tty> {
    CONSOLE_TTY.get().expect("console tty not initialised").clone()
}

pub fn open_console(flags: u32) -> Result<Arc<dyn FileOps>> {
    open_tty(console(), flags)
}

pub fn open_controlling(flags: u32) -> Result<Arc<dyn FileOps>> {
    let tty = crate::proc::process::controlling_tty().ok_or(ENXIO)?;
    open_tty(tty, flags | vfs::O_NOCTTY)
}

struct KeyState {
    shift: bool,
    ctrl: bool,
    alt: bool,
    caps: bool,
}
static KEYS: SpinLock<KeyState> = SpinLock::new(KeyState { shift: false, ctrl: false, alt: false, caps: false });

fn console_key_hook(code: u16, pressed: bool) {
    let mut ks = KEYS.lock();
    match code {
        42 | 54 => {
            ks.shift = pressed;
            return;
        }
        29 | 97 => {
            ks.ctrl = pressed;
            return;
        }
        56 | 100 => {
            ks.alt = pressed;
            return;
        }
        58 => {
            if pressed {
                ks.caps = !ks.caps;
            }
            return;
        }
        _ => {}
    }
    if !pressed || super::evdev::grabbed() {
        return;
    }
    let (shift, ctrl, alt, caps) = (ks.shift, ks.ctrl, ks.alt, ks.caps);
    drop(ks);
    let tty = match CONSOLE_TTY.get() {
        Some(t) => t,
        None => return,
    };
    let seq: &[u8] = match code {
        103 => b"\x1b[A",
        108 => b"\x1b[B",
        106 => b"\x1b[C",
        105 => b"\x1b[D",
        102 => b"\x1b[H",
        107 => b"\x1b[F",
        104 => b"\x1b[5~",
        109 => b"\x1b[6~",
        110 => b"\x1b[2~",
        111 => b"\x1b[3~",
        59 => b"\x1bOP",
        60 => b"\x1bOQ",
        61 => b"\x1bOR",
        62 => b"\x1bOS",
        63 => b"\x1b[15~",
        64 => b"\x1b[17~",
        65 => b"\x1b[18~",
        66 => b"\x1b[19~",
        67 => b"\x1b[20~",
        68 => b"\x1b[21~",
        87 => b"\x1b[23~",
        88 => b"\x1b[24~",
        _ => &[],
    };
    if !seq.is_empty() {
        tty.input_bytes(seq);
        return;
    }
    if let Some(mut b) = super::input::keycode_to_ascii(code, shift, ctrl, caps) {
        if code == 28 || code == 96 {
            b = b'\r';
        }
        if alt {
            tty.input_bytes(&[0x1b, b]);
        } else {
            tty.input_bytes(&[b]);
        }
    }
}

fn serial_irq(_f: &mut crate::arch::x86_64::interrupts::TrapFrame) {
    let mut buf = [0u8; 16];
    let mut n = 0;
    while n < buf.len() {
        match super::serial::try_read() {
            Some(b) => {
                buf[n] = b;
                n += 1;
            }
            None => break,
        }
    }
    if n > 0 {
        if let Some(t) = CONSOLE_TTY.get() {
            t.input_bytes(&buf[..n]);
        }
    }
}

pub fn init() {
    let tty = Tty::new("console", 1, false);
    tty.set_sink(Arc::new(|data: &[u8]| {
        crate::console::write_output(data);
        true
    }));
    let (cols, rows) = super::fbcon::size();
    if cols > 0 {
        *tty.winsize.lock() = Winsize { ws_row: rows as u16, ws_col: cols as u16, ws_xpixel: 0, ws_ypixel: 0 };
    }
    CONSOLE_TTY.call_once(|| tty);
    super::input::set_console_key_hook(console_key_hook);
    super::evdev::init();
    // serial receive interrupts (COM1 = IRQ 4)
    use crate::arch::x86_64::{interrupts, ioapic, lapic, VEC_ISA_BASE};
    interrupts::register_handler(VEC_ISA_BASE + 4, serial_irq);
    ioapic::route_isa(4, VEC_ISA_BASE + 4, lapic::id());
    super::serial::enable_rx_irq();
    klog!("tty", "console tty ready (serial + keyboard)");
}

// ---- pseudo-terminals ----------------------------------------------------------

const MAX_PTYS: usize = 64;

pub struct PtyPair {
    pub slave: Arc<Tty>,
    /// Bytes written by the slave, waiting for the master to read.
    to_master: SpinLock<VecDeque<u8>>,
    master_wq: WaitQueue,
    locked: AtomicBool,
    master_open: AtomicBool,
}

static PTYS: SpinLock<Vec<Option<Arc<PtyPair>>>> = SpinLock::new(Vec::new());

pub struct PtyMaster {
    pair: Arc<PtyPair>,
}

pub fn open_ptmx(_flags: u32) -> Result<Arc<dyn FileOps>> {
    let mut ptys = PTYS.lock();
    if ptys.is_empty() {
        ptys.resize(MAX_PTYS, None);
    }
    let idx = ptys.iter().position(|p| p.is_none()).ok_or(EAGAIN)?;
    let slave = Tty::new(&alloc::format!("pts/{}", idx), idx as u32, true);
    let pair = Arc::new(PtyPair {
        slave: slave.clone(),
        to_master: SpinLock::new(VecDeque::new()),
        master_wq: WaitQueue::new(),
        locked: AtomicBool::new(true),
        master_open: AtomicBool::new(true),
    });
    let weak = Arc::downgrade(&pair);
    slave.set_sink(Arc::new(move |data: &[u8]| {
        let pair = match weak.upgrade() {
            Some(p) => p,
            None => return true,
        };
        let mut q = pair.to_master.lock();
        if data.is_empty() {
            return q.len() < 65536;
        }
        if q.len() + data.len() > 65536 {
            return false;
        }
        q.extend(data.iter().copied());
        drop(q);
        pair.master_wq.wake_all();
        true
    }));
    ptys[idx] = Some(pair.clone());
    drop(ptys);
    // create /dev/pts/N
    if let Ok(pts) = crate::fs::lookup_abs("/dev/pts") {
        let _ = pts.create(&alloc::format!("{}", idx), Kind::CharDev, 0o620, vfs::makedev(super::chardev::MAJOR_PTS, idx as u32));
    }
    Ok(Arc::new(PtyMaster { pair }))
}

pub fn open_pts(n: u32, flags: u32) -> Result<Arc<dyn FileOps>> {
    let pair = {
        let ptys = PTYS.lock();
        ptys.get(n as usize).and_then(|p| p.clone()).ok_or(ENXIO)?
    };
    if pair.locked.load(Ordering::Relaxed) {
        return Err(EIO);
    }
    if !pair.master_open.load(Ordering::Relaxed) {
        return Err(EIO);
    }
    open_tty(pair.slave.clone(), flags)
}

impl FileOps for PtyMaster {
    fn read(&self, f: &File, b: &mut [u8]) -> Result<usize> {
        loop {
            {
                let mut q = self.pair.to_master.lock();
                if !q.is_empty() {
                    let n = q.len().min(b.len());
                    for i in 0..n {
                        b[i] = q.pop_front().unwrap();
                    }
                    drop(q);
                    self.pair.slave.write_wq.wake_all();
                    return Ok(n);
                }
            }
            if self.pair.slave.openers.load(Ordering::Acquire) == 0 && self.pair.slave.hangup.load(Ordering::Relaxed) {
                return Err(EIO);
            }
            if f.nonblock() {
                return Err(EAGAIN);
            }
            self.pair.master_wq.wait_until(|| !self.pair.to_master.lock().is_empty()).map_err(|_| EINTR)?;
        }
    }
    fn write(&self, _f: &File, b: &[u8]) -> Result<usize> {
        self.pair.slave.input_bytes(b);
        Ok(b.len())
    }
    fn poll(&self, _f: &File) -> u32 {
        let mut m = POLLOUT;
        if !self.pair.to_master.lock().is_empty() {
            m |= POLLIN;
        }
        m
    }
    fn poll_wait(&self) -> Option<&WaitQueue> {
        Some(&self.pair.master_wq)
    }
    fn ioctl(&self, f: &File, cmd: u32, arg: usize) -> Result<usize> {
        match cmd {
            0x80045430 => {
                write_user::<u32>(arg, self.pair.slave.index)?;
                Ok(0)
            }
            0x40045431 => {
                // TIOCSPTLCK
                let v: i32 = read_user(arg)?;
                self.pair.locked.store(v != 0, Ordering::Relaxed);
                Ok(0)
            }
            0x5441 => Err(ENOTTY), // TIOCGPTPEER
            0x5413 | 0x5414 | 0x5401 | 0x5402 | 0x5403 | 0x5404 | 0x540F | 0x5410 | 0x541B => self.pair.slave.ioctl(f, cmd, arg),
            0x5421 => {
                let on: i32 = read_user(arg)?;
                let fl = f.flags();
                f.set_flags(if on != 0 { fl | vfs::O_NONBLOCK } else { fl & !vfs::O_NONBLOCK });
                Ok(0)
            }
            _ => Err(ENOTTY),
        }
    }
    fn release(&self, _f: &File) {
        self.pair.master_open.store(false, Ordering::Relaxed);
        self.pair.slave.do_hangup();
        let idx = self.pair.slave.index as usize;
        PTYS.lock()[idx] = None;
        if let Ok(pts) = crate::fs::lookup_abs("/dev/pts") {
            let _ = pts.unlink(&alloc::format!("{}", idx));
        }
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
