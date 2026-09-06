//! Framebuffer text console for boot messages and the kernel console tty.

use super::fb;
use crate::boot::limine::FbInfo;
use crate::util::font::{self, Font};
use crate::sync::SpinLock;
use alloc::vec::Vec;

struct FbCon {
    fb: FbInfo,
    font: Font,
    scale: usize,
    cols: usize,
    rows: usize,
    cx: usize,
    cy: usize,
    cells: Vec<u8>,
    fg: u32,
    bg: u32,
    esc: u8, // ANSI escape state
    escbuf: [u8; 16],
    esclen: usize,
}

static CON: SpinLock<Option<FbCon>> = SpinLock::new(None);

const PAD: usize = 4;

pub fn init() {
    let Some(fbi) = fb::info() else { return };
    if fbi.bpp != 32 {
        klog!("fbcon", "unsupported framebuffer depth {}", fbi.bpp);
        return;
    }
    let font = font::load();
    let scale = if fbi.width >= 2400 { 2 } else { 1 };
    let cols = (fbi.width as usize - 2 * PAD) / (font.width * scale);
    let rows = (fbi.height as usize - 2 * PAD) / (font.height * scale);
    let bg = fbi.pixel(0x8c, 0x10, 0x10);
    let fg = fbi.pixel(0xff, 0xff, 0xff);
    fbi.fill_rect(0, 0, fbi.width, fbi.height, bg);
    let mut c = FbCon { fb: fbi, font, scale, cols, rows, cx: 0, cy: 0, cells: Vec::new(), fg, bg, esc: 0, escbuf: [0; 16], esclen: 0 };
    c.cells.resize(cols * rows, b' ');
    *CON.lock() = Some(c);
    crate::console::set_screen(true);
    klog!("fbcon", "{}x{} @ {} bpp, {} cols x {} rows", fbi.width, fbi.height, fbi.bpp, cols, rows);
}

impl FbCon {
    fn draw_cell(&self, col: usize, row: usize, ch: u8) {
        let glyph = self.font.glyph(ch as char);
        let gw = self.font.width;
        let gh = self.font.height;
        let x0 = PAD + col * gw * self.scale;
        let y0 = PAD + row * gh * self.scale;
        let pitch = self.fb.pitch as usize;
        let base = self.fb.ptr();
        for gy in 0..gh {
            let bits = glyph[gy];
            for sy in 0..self.scale {
                let row_ptr = unsafe { base.add((y0 + gy * self.scale + sy) * pitch) as *mut u32 };
                for gx in 0..gw {
                    let on = bits & (0x80 >> gx) != 0;
                    let color = if on { self.fg } else { self.bg };
                    for sx in 0..self.scale {
                        unsafe { row_ptr.add(x0 + gx * self.scale + sx).write_volatile(color) };
                    }
                }
            }
        }
    }

    fn redraw_all(&self) {
        for r in 0..self.rows {
            for c in 0..self.cols {
                self.draw_cell(c, r, self.cells[r * self.cols + c]);
            }
        }
    }

    fn scroll(&mut self) {
        let cols = self.cols;
        self.cells.copy_within(cols.., 0);
        let len = self.cells.len();
        for c in &mut self.cells[len - cols..] {
            *c = b' ';
        }
        self.redraw_all();
    }

    fn newline(&mut self) {
        self.cx = 0;
        self.cy += 1;
        if self.cy >= self.rows {
            self.cy = self.rows - 1;
            self.scroll();
        }
    }

    fn put(&mut self, b: u8) {
        // minimal ANSI escape handling: swallow CSI sequences
        if self.esc == 1 {
            if b == b'[' {
                self.esc = 2;
                self.esclen = 0;
            } else {
                self.esc = 0;
            }
            return;
        }
        if self.esc == 2 {
            if (0x40..=0x7E).contains(&b) {
                self.esc = 0;
                if b == b'J' {
                    // clear screen
                    for c in &mut self.cells {
                        *c = b' ';
                    }
                    self.cx = 0;
                    self.cy = 0;
                    self.redraw_all();
                }
            } else if self.esclen < 16 {
                self.escbuf[self.esclen] = b;
                self.esclen += 1;
            }
            return;
        }
        match b {
            0x1b => self.esc = 1,
            b'\n' => self.newline(),
            b'\r' => self.cx = 0,
            b'\t' => {
                let n = 8 - (self.cx % 8);
                for _ in 0..n {
                    self.put(b' ');
                }
            }
            0x08 | 0x7f => {
                if self.cx > 0 {
                    self.cx -= 1;
                    let idx = self.cy * self.cols + self.cx;
                    self.cells[idx] = b' ';
                    self.draw_cell(self.cx, self.cy, b' ');
                }
            }
            b if b < 0x20 => {}
            _ => {
                if self.cx >= self.cols {
                    self.newline();
                }
                let idx = self.cy * self.cols + self.cx;
                self.cells[idx] = b;
                self.draw_cell(self.cx, self.cy, b);
                self.cx += 1;
            }
        }
    }
}

pub fn write(s: &[u8]) {
    let mut g = CON.lock();
    if let Some(c) = g.as_mut() {
        for &b in s {
            c.put(b);
        }
    }
}

pub fn size() -> (usize, usize) {
    CON.lock().as_ref().map(|c| (c.cols, c.rows)).unwrap_or((80, 25))
}

/// Redraw the text console (e.g. after the compositor hands the screen back).
pub fn redraw() {
    let g = CON.lock();
    if let Some(c) = g.as_ref() {
        c.fb.fill_rect(0, 0, c.fb.width, c.fb.height, c.bg);
        c.redraw_all();
    }
}
