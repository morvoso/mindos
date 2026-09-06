//! Kernel heap: size-class free lists backed by physical frames through the
//! direct map. Large allocations (> 4 KiB) get contiguous frames directly.

use super::pmm;
use super::vmm::{p2v, v2p_hhdm};
use crate::sync::SpinLock;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

const CLASSES: [usize; 16] = [16, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096];

struct FreeBlock {
    next: *mut FreeBlock,
}

struct Heap {
    heads: [*mut FreeBlock; 16],
    allocated: usize,
    pages: usize,
}
unsafe impl Send for Heap {}

static HEAP: SpinLock<Heap> = SpinLock::new(Heap { heads: [ptr::null_mut(); 16], allocated: 0, pages: 0 });

fn class_for(size: usize, align: usize) -> Option<usize> {
    let need = size.max(align).max(16);
    if align > 16 {
        // power-of-two classes keep natural alignment
        let mut s = 16;
        while s < need {
            s *= 2;
        }
        return CLASSES.iter().position(|&c| c == s);
    }
    CLASSES.iter().position(|&c| c >= need)
}

impl Heap {
    fn refill(&mut self, class: usize) -> bool {
        let bsize = CLASSES[class];
        let pages = if bsize <= 1024 { 1 } else { 4 };
        let pa = match pmm::alloc_frames(pages) {
            Some(p) => p,
            None => return false,
        };
        self.pages += pages;
        let base = p2v(pa);
        let total = pages * 4096;
        let count = total / bsize;
        // thread the blocks onto the free list (in reverse so low addresses come first)
        let mut i = count;
        while i > 0 {
            i -= 1;
            let b = (base + i * bsize) as *mut FreeBlock;
            unsafe {
                (*b).next = self.heads[class];
            }
            self.heads[class] = b;
        }
        true
    }

    fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();
        if size > 4096 || align > 4096 {
            if align > 4096 {
                return ptr::null_mut();
            }
            let pages = (size + 4095) / 4096;
            return match pmm::alloc_frames(pages) {
                Some(pa) => {
                    self.pages += pages;
                    self.allocated += pages * 4096;
                    p2v(pa) as *mut u8
                }
                None => ptr::null_mut(),
            };
        }
        let class = match class_for(size, align) {
            Some(c) => c,
            None => return ptr::null_mut(),
        };
        if self.heads[class].is_null() && !self.refill(class) {
            return ptr::null_mut();
        }
        let b = self.heads[class];
        unsafe {
            self.heads[class] = (*b).next;
        }
        self.allocated += CLASSES[class];
        b as *mut u8
    }

    fn dealloc(&mut self, p: *mut u8, layout: Layout) {
        let size = layout.size();
        let align = layout.align();
        if size > 4096 || align > 4096 {
            let pages = (size + 4095) / 4096;
            pmm::free_frames(v2p_hhdm(p as usize), pages);
            self.pages -= pages;
            self.allocated -= pages * 4096;
            return;
        }
        let class = class_for(size, align).unwrap();
        let b = p as *mut FreeBlock;
        unsafe {
            (*b).next = self.heads[class];
        }
        self.heads[class] = b;
        self.allocated -= CLASSES[class];
    }
}

pub struct KernelAllocator;

unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HEAP.lock().alloc(layout)
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        HEAP.lock().dealloc(p, layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = HEAP.lock().alloc(layout);
        if !p.is_null() {
            unsafe { ptr::write_bytes(p, 0, layout.size()) };
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;

/// (bytes allocated, pages held by the heap)
pub fn stats() -> (usize, usize) {
    let h = HEAP.lock();
    (h.allocated, h.pages)
}
