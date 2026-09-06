//! Physical frame allocator: a bitmap over all RAM (bit set = frame in use).

use crate::boot::limine::{self, BootInfo};
use crate::sync::SpinLock;

const FRAME: u64 = 4096;

struct Pmm {
    bitmap: *mut u64,
    words: usize,
    frames: usize,
    free: usize,
    total_usable: usize,
    hint: usize,
}
unsafe impl Send for Pmm {}

static PMM: SpinLock<Pmm> = SpinLock::new(Pmm { bitmap: core::ptr::null_mut(), words: 0, frames: 0, free: 0, total_usable: 0, hint: 0 });

impl Pmm {
    #[inline]
    fn word(&self, i: usize) -> &mut u64 {
        unsafe { &mut *self.bitmap.add(i) }
    }
    #[inline]
    fn is_used(&self, f: usize) -> bool {
        *self.word(f / 64) & (1u64 << (f % 64)) != 0
    }
    #[inline]
    fn set_used(&mut self, f: usize) {
        *self.word(f / 64) |= 1u64 << (f % 64);
    }
    #[inline]
    fn set_free(&mut self, f: usize) {
        *self.word(f / 64) &= !(1u64 << (f % 64));
    }

    fn alloc_one(&mut self) -> Option<u64> {
        let start_word = self.hint / 64;
        for pass in 0..2 {
            let (lo, hi) = if pass == 0 { (start_word, self.words) } else { (0, start_word) };
            for w in lo..hi {
                let v = *self.word(w);
                if v != u64::MAX {
                    let bit = (!v).trailing_zeros() as usize;
                    let f = w * 64 + bit;
                    if f >= self.frames {
                        break;
                    }
                    self.set_used(f);
                    self.free -= 1;
                    self.hint = f + 1;
                    return Some(f as u64 * FRAME);
                }
            }
        }
        None
    }

    fn alloc_many(&mut self, n: usize) -> Option<u64> {
        if n == 1 {
            return self.alloc_one();
        }
        let mut run = 0usize;
        let mut f = 256; // never hand out the first 1 MiB
        while f < self.frames {
            // skip whole full words quickly
            if f % 64 == 0 && *self.word(f / 64) == u64::MAX {
                run = 0;
                f += 64;
                continue;
            }
            if self.is_used(f) {
                run = 0;
            } else {
                run += 1;
                if run == n {
                    let start = f + 1 - n;
                    for i in start..start + n {
                        self.set_used(i);
                    }
                    self.free -= n;
                    return Some(start as u64 * FRAME);
                }
            }
            f += 1;
        }
        None
    }
}

pub fn init(boot: &BootInfo) {
    // Highest usable address decides the bitmap size.
    let mut max_addr: u64 = 0;
    let mut total_usable: u64 = 0;
    for e in boot.memmap_iter() {
        if e.typ == limine::MEMMAP_USABLE {
            max_addr = max_addr.max(e.base + e.length);
            total_usable += e.length;
        }
    }
    let frames = (max_addr / FRAME) as usize;
    let words = (frames + 63) / 64;
    let bitmap_bytes = words * 8;

    // Find a usable region large enough for the bitmap.
    let mut bitmap_phys = 0u64;
    for e in boot.memmap_iter() {
        if e.typ == limine::MEMMAP_USABLE && e.length >= bitmap_bytes as u64 && e.base >= 0x100000 {
            bitmap_phys = e.base;
            break;
        }
    }
    assert!(bitmap_phys != 0, "no memory for the frame bitmap");
    let bitmap = (bitmap_phys as usize + boot.hhdm) as *mut u64;
    unsafe { core::ptr::write_bytes(bitmap as *mut u8, 0xFF, bitmap_bytes) };

    let mut p = PMM.lock();
    p.bitmap = bitmap;
    p.words = words;
    p.frames = frames;
    p.free = 0;
    p.total_usable = (total_usable / FRAME) as usize;
    for e in boot.memmap_iter() {
        if e.typ == limine::MEMMAP_USABLE {
            let first = (e.base / FRAME) as usize;
            let count = (e.length / FRAME) as usize;
            for f in first..first + count {
                if f < 256 {
                    continue; // keep the first 1 MiB reserved (SMP trampolines, legacy)
                }
                p.set_free(f);
                p.free += 1;
            }
        }
    }
    // the bitmap itself
    let bfirst = (bitmap_phys / FRAME) as usize;
    let bcount = (bitmap_bytes + 4095) / 4096;
    for f in bfirst..bfirst + bcount {
        if !p.is_used(f) {
            p.set_used(f);
            p.free -= 1;
        }
    }
    p.hint = bfirst + bcount;
}

pub fn alloc_frame() -> Option<u64> {
    PMM.lock().alloc_one()
}

/// Allocate a zero-filled frame.
pub fn alloc_zeroed() -> Option<u64> {
    let pa = alloc_frame()?;
    unsafe { core::ptr::write_bytes(super::vmm::p2v(pa) as *mut u8, 0, 4096) };
    Some(pa)
}

pub fn alloc_frames(n: usize) -> Option<u64> {
    PMM.lock().alloc_many(n)
}

pub fn free_frame(pa: u64) {
    let f = (pa / FRAME) as usize;
    let mut p = PMM.lock();
    if f < p.frames && p.is_used(f) {
        p.set_free(f);
        p.free += 1;
        if f < p.hint {
            p.hint = f;
        }
    }
}

pub fn free_frames(pa: u64, n: usize) {
    for i in 0..n {
        free_frame(pa + i as u64 * FRAME);
    }
}

/// (total usable frames, free frames)
pub fn stats() -> (usize, usize) {
    let p = PMM.lock();
    (p.total_usable, p.free)
}
