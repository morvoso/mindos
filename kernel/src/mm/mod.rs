//! Memory management: physical frames, kernel heap, page tables, address spaces.

pub mod addrspace;
pub mod heap;
pub mod pmm;
pub mod vmm;
pub mod user;

use crate::boot::limine::BootInfo;

pub const PAGE_SIZE: usize = 4096;
pub const PAGE_SHIFT: usize = 12;
/// Lowest kernel-half virtual address.
pub const KERNEL_BASE: usize = 0xffff_8000_0000_0000;
/// One past the highest user-space virtual address.
pub const USER_TOP: usize = 0x0000_8000_0000_0000;
/// Kernel stack size for tasks (pages).
pub const KSTACK_PAGES: usize = 16;

#[inline(always)]
pub const fn page_align_down(x: usize) -> usize {
    x & !(PAGE_SIZE - 1)
}
#[inline(always)]
pub const fn page_align_up(x: usize) -> usize {
    (x + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

pub fn init(boot: &BootInfo) {
    pmm::init(boot);
    vmm::init(boot);
    let (total, free) = pmm::stats();
    klog!("mm", "physical memory: {} MiB total, {} MiB free; hhdm at {:#x}", total * 4 / 1024, free * 4 / 1024, boot.hhdm);
}

/// Allocate a contiguous kernel stack; returns the address of its top.
pub fn alloc_kernel_stack(pages: usize) -> usize {
    let pa = pmm::alloc_frames(pages).expect("out of memory allocating kernel stack");
    vmm::p2v(pa) + pages * PAGE_SIZE
}

pub fn free_kernel_stack(top: usize, pages: usize) {
    let base = top - pages * PAGE_SIZE;
    pmm::free_frames(vmm::v2p_hhdm(base), pages);
}
