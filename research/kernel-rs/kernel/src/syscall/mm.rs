//! Memory management system calls.

use super::SysResult;
use crate::fs::vfs::Kind;
use crate::mm::addrspace::{Backing, AddressSpace, MAP_ANONYMOUS, MAP_FIXED, MAP_FIXED_NOREPLACE, MAP_PRIVATE, MAP_SHARED, PROT_EXEC, PROT_READ, PROT_WRITE};
use crate::mm::errno::*;
use crate::mm::PAGE_SIZE;
use alloc::sync::Arc;

fn aspace() -> Result<Arc<AddressSpace>> {
    crate::sched::current_ref().proc.aspace().ok_or(ENOMEM)
}

pub fn mmap(addr: usize, len: usize, prot: u32, flags: u32, fd: i32, off: u64) -> SysResult {
    if len == 0 {
        return Err(EINVAL);
    }
    if off & (PAGE_SIZE as u64 - 1) != 0 {
        return Err(EINVAL);
    }
    if addr & (PAGE_SIZE - 1) != 0 && flags & MAP_FIXED != 0 {
        return Err(EINVAL);
    }
    let a = aspace()?;
    let prot = prot & (PROT_READ | PROT_WRITE | PROT_EXEC);
    let shared = flags & MAP_SHARED != 0;
    if !shared && flags & MAP_PRIVATE == 0 {
        return Err(EINVAL);
    }
    let (backing, name): (Backing, &'static str) = if flags & MAP_ANONYMOUS != 0 {
        if shared {
            (Backing::File { inode: crate::fs::tmpfs::anonymous_file("shm"), offset: 0 }, "[anon_shmem]")
        } else {
            (Backing::Anon, "[anon]")
        }
    } else {
        let f = super::fs::files().get(fd)?;
        if !f.readable() {
            return Err(EACCES);
        }
        if shared && prot & PROT_WRITE != 0 && !f.writable() {
            return Err(EACCES);
        }
        let b = f.ops.mmap(&f, prot, shared)?;
        let b = match b {
            Backing::File { inode, .. } => {
                if inode.kind() != Kind::File {
                    return Err(ENODEV);
                }
                Backing::File { inode, offset: off }
            }
            Backing::Phys { base, wc } => Backing::Phys { base: base + off, wc },
            other => other,
        };
        let name = alloc::boxed::Box::leak(f.path().into_boxed_str());
        (b, name)
    };
    let fl = flags & (MAP_FIXED | MAP_FIXED_NOREPLACE | MAP_SHARED | MAP_PRIVATE);
    let start = a.mmap(addr, len, prot, fl, backing, name)?;
    if flags & crate::mm::addrspace::MAP_POPULATE != 0 {
        let _ = a.populate_range(start, len, prot & PROT_WRITE != 0);
    }
    Ok(start as u64)
}

pub fn munmap(addr: usize, len: usize) -> SysResult {
    let a = aspace()?;
    a.munmap(addr, len)?;
    Ok(0)
}

pub fn mprotect(addr: usize, len: usize, prot: u32) -> SysResult {
    let a = aspace()?;
    a.mprotect(addr, len, prot & 7)?;
    Ok(0)
}

pub fn brk(addr: usize) -> SysResult {
    let a = aspace()?;
    Ok(a.brk(addr) as u64)
}

pub fn mremap(old_addr: usize, old_len: usize, new_len: usize, flags: u32, new_addr: usize) -> SysResult {
    const MREMAP_MAYMOVE: u32 = 1;
    const MREMAP_FIXED: u32 = 2;
    let a = aspace()?;
    let old_len = (old_len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let new_len = (new_len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    if old_addr & (PAGE_SIZE - 1) != 0 || new_len == 0 {
        return Err(EINVAL);
    }
    if new_len <= old_len {
        if new_len < old_len {
            a.munmap(old_addr + new_len, old_len - new_len)?;
        }
        return Ok(old_addr as u64);
    }
    let (vma, ) = {
        let inner = a.inner.lock();
        (AddressSpace::find_vma(&inner, old_addr).cloned().ok_or(EFAULT)?,)
    };
    // try to grow in place
    let grow_ok = {
        let inner = a.inner.lock();
        inner.vmas.range(old_addr + old_len..old_addr + new_len).next().is_none() && vma.end == old_addr + old_len
    };
    if grow_ok && flags & MREMAP_FIXED == 0 {
        let backing = match &vma.backing {
            Backing::File { inode, offset } => Backing::File { inode: inode.clone(), offset: offset + old_len as u64 },
            Backing::Anon => Backing::Anon,
            Backing::Phys { base, wc } => Backing::Phys { base: base + old_len as u64, wc: *wc },
        };
        let fl = MAP_FIXED | if vma.shared { MAP_SHARED } else { MAP_PRIVATE };
        a.mmap(old_addr + old_len, new_len - old_len, vma.prot, fl, backing, vma.name)?;
        return Ok(old_addr as u64);
    }
    if flags & MREMAP_MAYMOVE == 0 {
        return Err(ENOMEM);
    }
    // move: map a new region, copy the contents, unmap the old one
    let fl = if vma.shared { MAP_SHARED } else { MAP_PRIVATE } | if flags & MREMAP_FIXED != 0 { MAP_FIXED } else { 0 };
    let backing = match &vma.backing {
        Backing::Anon => Backing::Anon,
        Backing::File { inode, offset } => Backing::File { inode: inode.clone(), offset: *offset },
        Backing::Phys { base, wc } => Backing::Phys { base: *base, wc: *wc },
    };
    let dst = a.mmap(if flags & MREMAP_FIXED != 0 { new_addr } else { 0 }, new_len, vma.prot, fl, backing, vma.name)?;
    if matches!(vma.backing, Backing::Anon) {
        // copy page contents
        let mut off = 0;
        let mut buf = alloc::vec![0u8; PAGE_SIZE];
        while off < old_len {
            if a.mapper.translate(old_addr + off).map(|(_, f)| f & crate::mm::vmm::PRESENT != 0).unwrap_or(false) {
                crate::mm::user::copy_from_user(&mut buf, old_addr + off)?;
                crate::mm::user::copy_to_user(dst + off, &buf)?;
            }
            off += PAGE_SIZE;
        }
    }
    a.munmap(old_addr, old_len)?;
    Ok(dst as u64)
}
