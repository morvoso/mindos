//! User address spaces: virtual memory areas, demand paging, copy-on-write.

use super::errno::*;
use super::pmm;
use super::vmm::{self, PageMapper, ADDR_MASK};
use super::{PAGE_SIZE, USER_TOP};
use crate::arch::x86_64::interrupts::TrapFrame;
use crate::fs::vfs::Inode;
use crate::sched::mutex::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub const PROT_READ: u32 = 1;
pub const PROT_WRITE: u32 = 2;
pub const PROT_EXEC: u32 = 4;

pub const MAP_SHARED: u32 = 0x01;
pub const MAP_PRIVATE: u32 = 0x02;
pub const MAP_FIXED: u32 = 0x10;
pub const MAP_ANONYMOUS: u32 = 0x20;
pub const MAP_GROWSDOWN: u32 = 0x100;
pub const MAP_NORESERVE: u32 = 0x4000;
pub const MAP_POPULATE: u32 = 0x8000;
pub const MAP_STACK: u32 = 0x20000;
pub const MAP_FIXED_NOREPLACE: u32 = 0x100000;

/// Lowest address user mappings may occupy.
pub const MMAP_MIN: usize = 0x1_0000;
/// Top of the mmap region (grows down from here).
pub const MMAP_TOP: usize = 0x7f00_0000_0000;
/// User stack top and reserved size.
pub const STACK_TOP: usize = 0x7fff_ffff_f000;
pub const STACK_SIZE: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub enum Backing {
    Anon,
    /// Page-cache backed file mapping; `offset` is the file offset of `start`.
    File { inode: Arc<dyn Inode>, offset: u64 },
    /// Direct physical memory (framebuffers, MMIO). `base` maps to `start`.
    Phys { base: u64, wc: bool },
}

#[derive(Clone)]
pub struct Vma {
    pub start: usize,
    pub end: usize,
    pub prot: u32,
    pub shared: bool,
    pub backing: Backing,
    pub name: &'static str,
}

impl Vma {
    pub fn contains(&self, a: usize) -> bool {
        a >= self.start && a < self.end
    }
    fn pte_flags(&self) -> u64 {
        let mut f = vmm::PRESENT | vmm::USER;
        if self.prot & PROT_WRITE != 0 {
            f |= vmm::WRITABLE;
        }
        if self.prot & PROT_EXEC == 0 {
            f |= vmm::NX;
        }
        f
    }
}

pub struct AsInner {
    pub vmas: BTreeMap<usize, Vma>,
    pub brk_start: usize,
    pub brk: usize,
    pub mmap_hint: usize,
    pub rss_pages: usize,
}

pub struct AddressSpace {
    pub mapper: PageMapper,
    pub inner: Mutex<AsInner>,
}

impl AddressSpace {
    pub fn new() -> Arc<AddressSpace> {
        Arc::new(AddressSpace {
            mapper: PageMapper::new_user(),
            inner: Mutex::new(AsInner { vmas: BTreeMap::new(), brk_start: 0, brk: 0, mmap_hint: MMAP_TOP, rss_pages: 0 }),
        })
    }

    pub fn pml4(&self) -> u64 {
        self.mapper.pml4
    }

    pub fn activate(&self) {
        self.mapper.activate();
    }

    fn is_current(&self) -> bool {
        self.mapper.is_current()
    }

    #[inline]
    fn flush(&self, va: usize) {
        if self.is_current() {
            crate::arch::x86_64::cpu::invlpg(va);
        }
    }

    // ---- VMA management ----------------------------------------------------

    pub fn find_vma(inner: &AsInner, addr: usize) -> Option<&Vma> {
        inner.vmas.range(..=addr).next_back().map(|(_, v)| v).filter(|v| v.contains(addr))
    }

    /// Find a free gap of `len` bytes below `mmap_hint` (top-down).
    fn find_gap(inner: &AsInner, len: usize, hint: usize) -> Option<usize> {
        let mut top = hint.min(MMAP_TOP);
        // walk VMAs downward from `top`
        loop {
            let cand = top.checked_sub(len)?;
            if cand < MMAP_MIN {
                return None;
            }
            // any VMA overlapping [cand, top)?
            let overlap = inner.vmas.range(..top).next_back().filter(|(_, v)| v.end > cand).map(|(_, v)| v.clone());
            match overlap {
                None => return Some(cand),
                Some(v) => top = v.start,
            }
        }
    }

    fn range_free(inner: &AsInner, start: usize, end: usize) -> bool {
        inner.vmas.range(..end).next_back().map(|(_, v)| v.end <= start).unwrap_or(true)
    }

    /// Map a region. `addr` is a hint unless MAP_FIXED is given.
    pub fn mmap(&self, addr: usize, len: usize, prot: u32, flags: u32, backing: Backing, name: &'static str) -> Result<usize> {
        if len == 0 {
            return Err(EINVAL);
        }
        let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let mut inner = self.inner.lock();
        let start = if flags & MAP_FIXED != 0 {
            if addr & (PAGE_SIZE - 1) != 0 || addr + len > USER_TOP {
                return Err(EINVAL);
            }
            if flags & MAP_FIXED_NOREPLACE != 0 && !Self::range_free(&inner, addr, addr + len) {
                return Err(EEXIST);
            }
            self.unmap_locked(&mut inner, addr, len);
            addr
        } else if addr != 0 && addr & (PAGE_SIZE - 1) == 0 && addr + len <= USER_TOP && addr >= MMAP_MIN && Self::range_free(&inner, addr, addr + len) {
            addr
        } else {
            let hint = inner.mmap_hint;
            let s = Self::find_gap(&inner, len, hint).ok_or(ENOMEM)?;
            inner.mmap_hint = s;
            s
        };
        let shared = flags & MAP_SHARED != 0;
        let vma = Vma { start, end: start + len, prot, shared, backing, name };
        inner.vmas.insert(start, vma);
        Ok(start)
    }

    pub fn munmap(&self, addr: usize, len: usize) -> Result<()> {
        if addr & (PAGE_SIZE - 1) != 0 || len == 0 {
            return Err(EINVAL);
        }
        let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let mut inner = self.inner.lock();
        self.unmap_locked(&mut inner, addr, len);
        Ok(())
    }

    /// Remove all mappings within [addr, addr+len), splitting VMAs as needed.
    fn unmap_locked(&self, inner: &mut AsInner, addr: usize, len: usize) {
        let end = addr + len;
        let affected: Vec<usize> = inner.vmas.range(..end).filter(|(_, v)| v.end > addr).map(|(k, _)| *k).collect();
        for k in affected {
            let v = inner.vmas.remove(&k).unwrap();
            // left remainder
            if v.start < addr {
                let mut left = v.clone();
                left.end = addr;
                inner.vmas.insert(left.start, left);
            }
            // right remainder
            if v.end > end {
                let mut right = v.clone();
                right.start = end;
                if let Backing::File { inode, offset } = &v.backing {
                    right.backing = Backing::File { inode: inode.clone(), offset: offset + (end - v.start) as u64 };
                }
                if let Backing::Phys { base, wc } = &v.backing {
                    right.backing = Backing::Phys { base: base + (end - v.start) as u64, wc: *wc };
                }
                inner.vmas.insert(right.start, right);
            }
            let lo = v.start.max(addr);
            let hi = v.end.min(end);
            self.unmap_pages(inner, lo, hi);
        }
    }

    fn unmap_pages(&self, inner: &mut AsInner, lo: usize, hi: usize) {
        let mut va = lo;
        while va < hi {
            if let Some(old) = self.mapper.unmap(va) {
                if old & vmm::PRESENT != 0 {
                    pmm::ref_dec(old & ADDR_MASK);
                    inner.rss_pages = inner.rss_pages.saturating_sub(1);
                }
                self.flush(va);
            }
            va += PAGE_SIZE;
        }
    }

    pub fn mprotect(&self, addr: usize, len: usize, prot: u32) -> Result<()> {
        if addr & (PAGE_SIZE - 1) != 0 {
            return Err(EINVAL);
        }
        let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let end = addr + len;
        let mut inner = self.inner.lock();
        // split VMAs at addr and end, then update prot of those fully inside
        for split in [addr, end] {
            if let Some(v) = Self::find_vma(&inner, split).cloned() {
                if v.start != split {
                    let mut left = v.clone();
                    left.end = split;
                    let mut right = v.clone();
                    right.start = split;
                    if let Backing::File { inode, offset } = &v.backing {
                        right.backing = Backing::File { inode: inode.clone(), offset: offset + (split - v.start) as u64 };
                    }
                    if let Backing::Phys { base, wc } = &v.backing {
                        right.backing = Backing::Phys { base: base + (split - v.start) as u64, wc: *wc };
                    }
                    inner.vmas.insert(left.start, left);
                    inner.vmas.insert(right.start, right);
                }
            }
        }
        let keys: Vec<usize> = inner.vmas.range(addr..end).map(|(k, _)| *k).collect();
        for k in keys {
            let v = inner.vmas.get_mut(&k).unwrap();
            v.prot = prot;
            let (s, e) = (v.start, v.end);
            let base_flags = v.pte_flags();
            // update existing PTEs: never grant write to COW pages
            let mut va = s;
            while va < e {
                if let Some((pa, f)) = self.mapper.translate(va) {
                    let mut nf = base_flags | (f & (vmm::COW | vmm::ACCESSED | vmm::DIRTY | vmm::WRITE_THROUGH | vmm::NO_CACHE));
                    if f & vmm::COW != 0 {
                        nf &= !vmm::WRITABLE;
                    }
                    self.mapper.map(va, pa, nf);
                    self.flush(va);
                }
                va += PAGE_SIZE;
            }
        }
        Ok(())
    }

    pub fn brk(&self, new_brk: usize) -> usize {
        let mut inner = self.inner.lock();
        let cur = inner.brk;
        if new_brk == 0 || new_brk < inner.brk_start {
            return cur;
        }
        let new_end = (new_brk + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let cur_end = (cur + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        if new_end > cur_end {
            if !Self::range_free(&inner, cur_end, new_end) || new_end > MMAP_TOP {
                return cur;
            }
            // extend the heap VMA (or create it)
            let start = inner.brk_start;
            if let Some(v) = inner.vmas.get_mut(&start) {
                if v.end == cur_end {
                    v.end = new_end;
                } else {
                    inner.vmas.insert(cur_end, Vma { start: cur_end, end: new_end, prot: PROT_READ | PROT_WRITE, shared: false, backing: Backing::Anon, name: "[heap]" });
                }
            } else {
                inner.vmas.insert(cur_end, Vma { start: cur_end, end: new_end, prot: PROT_READ | PROT_WRITE, shared: false, backing: Backing::Anon, name: "[heap]" });
            }
        } else if new_end < cur_end {
            self.unmap_locked(&mut inner, new_end, cur_end - new_end);
        }
        inner.brk = new_brk;
        new_brk
    }

    pub fn set_brk_start(&self, addr: usize) {
        let mut inner = self.inner.lock();
        let a = (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        inner.brk_start = a;
        inner.brk = a;
    }

    // ---- page faults --------------------------------------------------------

    /// Resolve a fault at `addr`. Returns Ok(()) if the access is now valid.
    pub fn fault(&self, addr: usize, write: bool, exec: bool) -> Result<()> {
        let mut inner = self.inner.lock();
        let vma = match Self::find_vma(&inner, addr) {
            Some(v) => v.clone(),
            None => return Err(EFAULT),
        };
        if write && vma.prot & PROT_WRITE == 0 {
            return Err(EACCES);
        }
        if exec && vma.prot & PROT_EXEC == 0 {
            return Err(EACCES);
        }
        if !write && !exec && vma.prot & PROT_READ == 0 {
            return Err(EACCES);
        }
        let va = addr & !(PAGE_SIZE - 1);
        self.populate_page(&mut inner, &vma, va, write)
    }

    fn populate_page(&self, inner: &mut AsInner, vma: &Vma, va: usize, write: bool) -> Result<()> {
        let flags = vma.pte_flags();
        if let Some((pa, f)) = self.mapper.translate(va) {
            if f & vmm::PRESENT != 0 {
                if write && f & vmm::WRITABLE == 0 {
                    if f & vmm::COW != 0 {
                        // copy-on-write
                        if pmm::ref_count(pa) == 1 {
                            self.mapper.map(va, pa, (f & !vmm::COW) | vmm::WRITABLE);
                        } else {
                            let np = pmm::alloc_frame_ref().ok_or(ENOMEM)?;
                            unsafe {
                                core::ptr::copy_nonoverlapping(vmm::p2v(pa) as *const u8, vmm::p2v(np) as *mut u8, PAGE_SIZE);
                            }
                            pmm::ref_dec(pa);
                            self.mapper.map(va, np, (f & !vmm::COW) | vmm::WRITABLE);
                        }
                        self.flush(va);
                        return Ok(());
                    }
                    return Err(EACCES);
                }
                // spurious (already mapped, e.g. by another thread)
                return Ok(());
            }
        }
        match &vma.backing {
            Backing::Anon => {
                let pa = pmm::alloc_zeroed_ref().ok_or(ENOMEM)?;
                self.mapper.map(va, pa, flags);
                inner.rss_pages += 1;
            }
            Backing::Phys { base, wc } => {
                let pa = base + (va - vma.start) as u64;
                let mut f = flags;
                if *wc {
                    f |= vmm::WRITE_THROUGH;
                } else {
                    f |= vmm::NO_CACHE | vmm::WRITE_THROUGH;
                }
                self.mapper.map(va, pa, f);
            }
            Backing::File { inode, offset } => {
                let index = (offset + (va - vma.start) as u64) / PAGE_SIZE as u64;
                if vma.shared {
                    let pa = inode.get_page(index)?; // ref taken for us
                    self.mapper.map(va, pa, flags);
                } else {
                    let pa = inode.get_page(index)?;
                    if write {
                        // private write: copy immediately
                        let np = pmm::alloc_frame_ref().ok_or(ENOMEM)?;
                        unsafe {
                            core::ptr::copy_nonoverlapping(vmm::p2v(pa) as *const u8, vmm::p2v(np) as *mut u8, PAGE_SIZE);
                        }
                        pmm::ref_dec(pa);
                        self.mapper.map(va, np, flags);
                    } else {
                        // map the cached page read-only, mark COW if the VMA is writable
                        let mut f = flags & !vmm::WRITABLE;
                        if vma.prot & PROT_WRITE != 0 {
                            f |= vmm::COW;
                        }
                        self.mapper.map(va, pa, f);
                    }
                }
                inner.rss_pages += 1;
            }
        }
        self.flush(va);
        Ok(())
    }

    /// Make sure [addr, addr+len) is mapped (used before kernel accesses user memory).
    pub fn populate_range(&self, addr: usize, len: usize, write: bool) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let end = addr.checked_add(len).ok_or(EFAULT)?;
        if end > USER_TOP {
            return Err(EFAULT);
        }
        let mut inner = self.inner.lock();
        let mut va = addr & !(PAGE_SIZE - 1);
        while va < end {
            let vma = Self::find_vma(&inner, va).cloned().ok_or(EFAULT)?;
            if write && vma.prot & PROT_WRITE == 0 {
                return Err(EFAULT);
            }
            if !write && vma.prot & PROT_READ == 0 {
                return Err(EFAULT);
            }
            // fast path: already mapped with sufficient rights
            let mapped = match self.mapper.translate(va) {
                Some((_, f)) if f & vmm::PRESENT != 0 => !write || f & vmm::WRITABLE != 0,
                _ => false,
            };
            if !mapped {
                self.populate_page(&mut inner, &vma, va, write)?;
            }
            va += PAGE_SIZE;
        }
        Ok(())
    }

    /// Check that [addr, addr+len) lies within VMAs granting the access (no faulting).
    pub fn check_range(&self, addr: usize, len: usize, write: bool) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let end = addr.checked_add(len).ok_or(EFAULT)?;
        if end > USER_TOP {
            return Err(EFAULT);
        }
        let inner = self.inner.lock();
        let mut va = addr;
        while va < end {
            let vma = Self::find_vma(&inner, va).ok_or(EFAULT)?;
            if write && vma.prot & PROT_WRITE == 0 {
                return Err(EFAULT);
            }
            if !write && vma.prot & PROT_READ == 0 {
                return Err(EFAULT);
            }
            va = vma.end;
        }
        Ok(())
    }

    // ---- fork ---------------------------------------------------------------

    /// Duplicate this address space for a child (copy-on-write).
    pub fn fork(&self) -> Result<Arc<AddressSpace>> {
        let child = AddressSpace::new();
        let mut inner = self.inner.lock();
        let mut ci = child.inner.lock();
        ci.brk_start = inner.brk_start;
        ci.brk = inner.brk;
        ci.mmap_hint = inner.mmap_hint;
        for (_, v) in inner.vmas.iter() {
            ci.vmas.insert(v.start, v.clone());
        }
        let vmas: Vec<Vma> = inner.vmas.values().cloned().collect();
        for v in vmas {
            let mut va = v.start;
            while va < v.end {
                if let Some((pa, f)) = self.mapper.translate(va) {
                    if f & vmm::PRESENT != 0 {
                        let nf = if v.shared || matches!(v.backing, Backing::Phys { .. }) {
                            f
                        } else if f & vmm::WRITABLE != 0 {
                            let nf = (f & !vmm::WRITABLE) | vmm::COW;
                            self.mapper.map(va, pa, nf);
                            self.flush(va);
                            nf
                        } else {
                            f
                        };
                        pmm::ref_inc(pa);
                        child.mapper.map(va, pa, nf);
                        ci.rss_pages += 1;
                    }
                }
                va += PAGE_SIZE;
            }
        }
        let _ = &mut inner;
        drop(ci);
        Ok(child)
    }

    /// Tear down every mapping (called on exec/exit).
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        let vmas: Vec<Vma> = inner.vmas.values().cloned().collect();
        for v in vmas {
            self.unmap_pages(&mut inner, v.start, v.end);
        }
        inner.vmas.clear();
        inner.brk = 0;
        inner.brk_start = 0;
        inner.mmap_hint = MMAP_TOP;
    }

    pub fn rss(&self) -> usize {
        self.inner.lock().rss_pages
    }
    pub fn vsize(&self) -> usize {
        self.inner.lock().vmas.values().map(|v| v.end - v.start).sum()
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        self.clear();
        self.mapper.destroy();
    }
}

/// Entry from the #PF handler. Returns true if the fault was resolved.
pub fn handle_page_fault(addr: usize, error: u64, _frame: &mut TrapFrame) -> bool {
    let write = error & 2 != 0;
    let exec = error & 0x10 != 0;
    let task = match crate::sched::try_current() {
        Some(t) => t,
        None => return false,
    };
    let aspace = match task.proc.aspace() {
        Some(a) => a,
        None => return false,
    };
    aspace.fault(addr, write, exec).is_ok()
}
