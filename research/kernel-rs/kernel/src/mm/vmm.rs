//! Page-table management for the kernel and user address spaces.

use super::pmm;
use super::PAGE_SIZE;
use crate::arch::x86_64::cpu;
use crate::boot::limine::BootInfo;
use crate::sync::SpinLock;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
pub const USER: u64 = 1 << 2;
pub const WRITE_THROUGH: u64 = 1 << 3; // PAT entry 1 = write-combining
pub const NO_CACHE: u64 = 1 << 4;
pub const ACCESSED: u64 = 1 << 5;
pub const DIRTY: u64 = 1 << 6;
pub const HUGE: u64 = 1 << 7;
pub const GLOBAL: u64 = 1 << 8;
/// Software bit: page is copy-on-write (read-only PTE that must be copied on write).
pub const COW: u64 = 1 << 9;
pub const NX: u64 = 1 << 63;
pub const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;
pub const FLAGS_MASK: u64 = !ADDR_MASK;

static HHDM: AtomicUsize = AtomicUsize::new(0);
static KERNEL_PML4: AtomicU64 = AtomicU64::new(0);

/// Virtual range for kernel MMIO mappings (PML4 slot 448).
const VMAP_START: usize = 0xffff_e000_0000_0000;
static VMAP_NEXT: SpinLock<usize> = SpinLock::new(VMAP_START);

#[inline(always)]
pub fn hhdm() -> usize {
    HHDM.load(Ordering::Relaxed)
}
#[inline(always)]
pub fn p2v(pa: u64) -> usize {
    pa as usize + HHDM.load(Ordering::Relaxed)
}
#[inline(always)]
pub fn v2p_hhdm(va: usize) -> u64 {
    (va - HHDM.load(Ordering::Relaxed)) as u64
}
pub fn kernel_pml4() -> u64 {
    KERNEL_PML4.load(Ordering::Relaxed)
}

#[inline]
unsafe fn table(pa: u64) -> &'static mut [u64; 512] {
    unsafe { &mut *(p2v(pa & ADDR_MASK) as *mut [u64; 512]) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageMapper {
    pub pml4: u64,
}

impl PageMapper {
    pub fn kernel() -> Self {
        PageMapper { pml4: kernel_pml4() }
    }
    pub fn current() -> Self {
        PageMapper { pml4: cpu::read_cr3() & ADDR_MASK }
    }
    /// A fresh user address space sharing the kernel half.
    pub fn new_user() -> Option<Self> {
        let pml4 = pmm::alloc_zeroed()?;
        unsafe {
            let src = table(kernel_pml4());
            let dst = table(pml4);
            dst[256..512].copy_from_slice(&src[256..512]);
        }
        Some(PageMapper { pml4 })
    }
    pub fn is_current(&self) -> bool {
        cpu::read_cr3() & ADDR_MASK == self.pml4
    }
    pub fn activate(&self) {
        if !self.is_current() {
            cpu::write_cr3(self.pml4);
        }
    }

    unsafe fn next(entry: &mut u64, create: bool, user: bool) -> Option<&'static mut [u64; 512]> {
        if *entry & PRESENT != 0 {
            if *entry & HUGE != 0 {
                return None;
            }
            return Some(unsafe { table(*entry) });
        }
        if !create {
            return None;
        }
        let f = pmm::alloc_zeroed()?;
        *entry = f | PRESENT | WRITABLE | if user { USER } else { 0 };
        Some(unsafe { table(f) })
    }

    /// Get (optionally creating) the PTE slot for a virtual address.
    pub fn pte(&self, va: usize, create: bool) -> Option<&'static mut u64> {
        let user = va < super::USER_TOP;
        unsafe {
            let pml4 = table(self.pml4);
            let pdpt = Self::next(&mut pml4[(va >> 39) & 511], create, user)?;
            let pd = Self::next(&mut pdpt[(va >> 30) & 511], create, user)?;
            let pt = Self::next(&mut pd[(va >> 21) & 511], create, user)?;
            Some(&mut pt[(va >> 12) & 511])
        }
    }

    pub fn map(&self, va: usize, pa: u64, flags: u64) -> Result<(), ()> {
        let pte = self.pte(va, true).ok_or(())?;
        *pte = (pa & ADDR_MASK) | flags | PRESENT;
        if self.is_current() {
            cpu::invlpg(va);
        }
        Ok(())
    }

    /// Remove a mapping; returns the old PTE (address + flags) if it was present.
    pub fn unmap(&self, va: usize) -> Option<u64> {
        let pte = self.pte(va, false)?;
        let old = *pte;
        if old & PRESENT == 0 {
            return None;
        }
        *pte = 0;
        if self.is_current() {
            cpu::invlpg(va);
        }
        Some(old)
    }

    pub fn translate(&self, va: usize) -> Option<(u64, u64)> {
        unsafe {
            let pml4 = table(self.pml4);
            let e4 = pml4[(va >> 39) & 511];
            if e4 & PRESENT == 0 {
                return None;
            }
            let pdpt = table(e4);
            let e3 = pdpt[(va >> 30) & 511];
            if e3 & PRESENT == 0 {
                return None;
            }
            if e3 & HUGE != 0 {
                return Some(((e3 & ADDR_MASK) + (va as u64 & 0x3FFF_FFFF), e3 & FLAGS_MASK));
            }
            let pd = table(e3);
            let e2 = pd[(va >> 21) & 511];
            if e2 & PRESENT == 0 {
                return None;
            }
            if e2 & HUGE != 0 {
                return Some(((e2 & ADDR_MASK) + (va as u64 & 0x1F_FFFF), e2 & FLAGS_MASK));
            }
            let pt = table(e2);
            let e1 = pt[(va >> 12) & 511];
            if e1 & PRESENT == 0 {
                return None;
            }
            Some(((e1 & ADDR_MASK) + (va as u64 & 0xFFF), e1 & FLAGS_MASK))
        }
    }

    /// Free every page-table page of the user half (not the mapped frames).
    pub fn free_user_tables(&self) {
        unsafe {
            let pml4 = table(self.pml4);
            for e4 in pml4[..256].iter_mut() {
                if *e4 & PRESENT == 0 {
                    continue;
                }
                let pdpt = table(*e4);
                for e3 in pdpt.iter_mut() {
                    if *e3 & PRESENT == 0 || *e3 & HUGE != 0 {
                        continue;
                    }
                    let pd = table(*e3);
                    for e2 in pd.iter_mut() {
                        if *e2 & PRESENT == 0 || *e2 & HUGE != 0 {
                            continue;
                        }
                        pmm::free_frame(*e2 & ADDR_MASK);
                    }
                    pmm::free_frame(*e3 & ADDR_MASK);
                }
                pmm::free_frame(*e4 & ADDR_MASK);
                *e4 = 0;
            }
        }
    }

    /// Release the PML4 itself (after `free_user_tables`).
    pub fn destroy(self) {
        pmm::free_frame(self.pml4);
    }
}

/// Take over paging from the bootloader: build our own kernel PML4 with every
/// upper-half PDPT pre-allocated so later kernel mappings are visible in all
/// address spaces.
pub fn init(boot: &BootInfo) {
    HHDM.store(boot.hhdm, Ordering::Relaxed);
    let old = cpu::read_cr3() & ADDR_MASK;
    let new = pmm::alloc_zeroed().expect("no memory for kernel PML4");
    unsafe {
        let src = table(old);
        let dst = table(new);
        for i in 256..512 {
            if src[i] & PRESENT != 0 {
                dst[i] = src[i];
            } else {
                let f = pmm::alloc_zeroed().expect("no memory for kernel PDPT");
                dst[i] = f | PRESENT | WRITABLE;
            }
        }
    }
    KERNEL_PML4.store(new, Ordering::Relaxed);
    cpu::write_cr3(new);
}

/// Map a physical MMIO range into the kernel's vmap area. `wc` selects
/// write-combining (framebuffers) instead of strongly uncached.
pub fn map_mmio(pa: u64, size: usize, wc: bool) -> usize {
    let off = (pa & 0xFFF) as usize;
    let base = pa & !0xFFF;
    let len = super::page_align_up(size + off);
    let va = {
        let mut n = VMAP_NEXT.lock();
        let v = *n;
        *n += len + PAGE_SIZE; // guard page
        v
    };
    let cache = if wc { WRITE_THROUGH } else { NO_CACHE | WRITE_THROUGH };
    let m = PageMapper::kernel();
    for i in (0..len).step_by(PAGE_SIZE) {
        m.map(va + i, base + i as u64, WRITABLE | NX | GLOBAL | cache).expect("mmio map failed");
    }
    va + off
}

/// Map ordinary RAM frames at a kernel virtual address (e.g. non-contiguous buffers).
pub fn map_kernel_pages(frames: &[u64], flags: u64) -> usize {
    let len = frames.len() * PAGE_SIZE;
    let va = {
        let mut n = VMAP_NEXT.lock();
        let v = *n;
        *n += len + PAGE_SIZE;
        v
    };
    let m = PageMapper::kernel();
    for (i, &pa) in frames.iter().enumerate() {
        m.map(va + i * PAGE_SIZE, pa, WRITABLE | NX | GLOBAL | flags).expect("kernel map failed");
    }
    va
}

pub fn flush_tlb_all() {
    cpu::write_cr3(cpu::read_cr3());
}
