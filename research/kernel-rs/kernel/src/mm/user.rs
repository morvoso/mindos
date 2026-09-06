//! Safe-ish access to user memory from the kernel.

use super::errno::*;
use super::USER_TOP;
use alloc::string::String;
use alloc::vec::Vec;

fn aspace() -> Result<alloc::sync::Arc<super::addrspace::AddressSpace>> {
    crate::sched::current_ref().proc.aspace().ok_or(EFAULT)
}

/// Verify and fault in a user range.
pub fn access_ok(addr: usize, len: usize, write: bool) -> Result<()> {
    if len == 0 {
        return Ok(());
    }
    if addr.checked_add(len).map(|e| e > USER_TOP).unwrap_or(true) {
        return Err(EFAULT);
    }
    aspace()?.populate_range(addr, len, write)
}

pub fn copy_from_user(dst: &mut [u8], src: usize) -> Result<()> {
    access_ok(src, dst.len(), false)?;
    unsafe { core::ptr::copy_nonoverlapping(src as *const u8, dst.as_mut_ptr(), dst.len()) };
    Ok(())
}

pub fn copy_to_user(dst: usize, src: &[u8]) -> Result<()> {
    access_ok(dst, src.len(), true)?;
    unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, src.len()) };
    Ok(())
}

pub fn read_user<T: Copy>(addr: usize) -> Result<T> {
    access_ok(addr, core::mem::size_of::<T>(), false)?;
    Ok(unsafe { core::ptr::read_unaligned(addr as *const T) })
}

pub fn write_user<T: Copy>(addr: usize, v: T) -> Result<()> {
    access_ok(addr, core::mem::size_of::<T>(), true)?;
    unsafe { core::ptr::write_unaligned(addr as *mut T, v) };
    Ok(())
}

pub fn read_bytes(addr: usize, len: usize) -> Result<Vec<u8>> {
    let mut v = alloc::vec![0u8; len];
    copy_from_user(&mut v, addr)?;
    Ok(v)
}

/// Read a NUL-terminated string (at most `max` bytes).
pub fn read_cstr(addr: usize, max: usize) -> Result<String> {
    let mut out = Vec::new();
    let mut a = addr;
    loop {
        if out.len() >= max {
            return Err(ENAMETOOLONG);
        }
        // read up to the end of the current page in one go
        let page_rem = 4096 - (a & 4095);
        let chunk = page_rem.min(max - out.len());
        access_ok(a, chunk, false)?;
        let s = unsafe { core::slice::from_raw_parts(a as *const u8, chunk) };
        if let Some(i) = s.iter().position(|&b| b == 0) {
            out.extend_from_slice(&s[..i]);
            return String::from_utf8(out).map_err(|_| EINVAL);
        }
        out.extend_from_slice(s);
        a += chunk;
    }
}

pub fn read_path(addr: usize) -> Result<String> {
    read_cstr(addr, 4096)
}

/// Read a NULL-terminated array of user string pointers (argv/envp).
pub fn read_str_array(addr: usize, max_count: usize, max_total: usize) -> Result<Vec<String>> {
    let mut v = Vec::new();
    if addr == 0 {
        return Ok(v);
    }
    let mut total = 0usize;
    for i in 0..max_count {
        let p: u64 = read_user(addr + i * 8)?;
        if p == 0 {
            return Ok(v);
        }
        let s = read_cstr(p as usize, 128 * 1024)?;
        total += s.len() + 1;
        if total > max_total {
            return Err(E2BIG);
        }
        v.push(s);
    }
    Err(E2BIG)
}

/// A user buffer the kernel may read from or write to directly after validation.
pub struct UserBuf {
    pub addr: usize,
    pub len: usize,
}

impl UserBuf {
    pub fn new(addr: usize, len: usize) -> UserBuf {
        UserBuf { addr, len }
    }
    /// Get a slice for reading (the range is faulted in first).
    pub fn as_slice(&self) -> Result<&[u8]> {
        access_ok(self.addr, self.len, false)?;
        Ok(unsafe { core::slice::from_raw_parts(self.addr as *const u8, self.len) })
    }
    /// Get a mutable slice for writing.
    pub fn as_mut_slice(&self) -> Result<&mut [u8]> {
        access_ok(self.addr, self.len, true)?;
        Ok(unsafe { core::slice::from_raw_parts_mut(self.addr as *mut u8, self.len) })
    }
}

/// iovec as laid out by Linux.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IoVec {
    pub base: u64,
    pub len: u64,
}

pub fn read_iovecs(addr: usize, count: usize) -> Result<Vec<IoVec>> {
    if count > 1024 {
        return Err(EINVAL);
    }
    let mut v = Vec::with_capacity(count);
    for i in 0..count {
        let iov: IoVec = read_user(addr + i * 16)?;
        if iov.len > isize::MAX as u64 {
            return Err(EINVAL);
        }
        v.push(iov);
    }
    Ok(v)
}
