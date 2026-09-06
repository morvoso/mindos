//! Per-process file descriptor table.

use super::vfs::File;
use crate::mm::errno::*;
use crate::sync::SpinLock;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub const MAX_FDS: usize = 4096;

#[derive(Clone)]
pub struct FdEntry {
    pub file: Arc<File>,
    pub cloexec: bool,
}

pub struct FdTable {
    fds: SpinLock<Vec<Option<FdEntry>>>,
}

impl FdTable {
    pub fn new() -> Arc<FdTable> {
        Arc::new(FdTable { fds: SpinLock::new(Vec::new()) })
    }

    pub fn get(&self, fd: i32) -> Result<Arc<File>> {
        if fd < 0 {
            return Err(EBADF);
        }
        let fds = self.fds.lock();
        fds.get(fd as usize).and_then(|e| e.as_ref()).map(|e| e.file.clone()).ok_or(EBADF)
    }

    pub fn get_entry(&self, fd: i32) -> Result<FdEntry> {
        if fd < 0 {
            return Err(EBADF);
        }
        let fds = self.fds.lock();
        fds.get(fd as usize).and_then(|e| e.clone()).ok_or(EBADF)
    }

    /// Install `file` at the lowest free descriptor >= `min`.
    pub fn alloc(&self, file: Arc<File>, cloexec: bool, min: usize) -> Result<i32> {
        let mut fds = self.fds.lock();
        let mut i = min;
        while i < fds.len() {
            if fds[i].is_none() {
                fds[i] = Some(FdEntry { file, cloexec });
                return Ok(i as i32);
            }
            i += 1;
        }
        if i >= MAX_FDS {
            return Err(EMFILE);
        }
        while fds.len() < i {
            fds.push(None);
        }
        fds.push(Some(FdEntry { file, cloexec }));
        Ok(i as i32)
    }

    /// Install at a specific descriptor, replacing any existing one.
    pub fn set(&self, fd: i32, file: Arc<File>, cloexec: bool) -> Result<Option<Arc<File>>> {
        if fd < 0 || fd as usize >= MAX_FDS {
            return Err(EBADF);
        }
        let mut fds = self.fds.lock();
        let i = fd as usize;
        while fds.len() <= i {
            fds.push(None);
        }
        let old = fds[i].replace(FdEntry { file, cloexec }).map(|e| e.file);
        Ok(old)
    }

    pub fn close(&self, fd: i32) -> Result<Arc<File>> {
        if fd < 0 {
            return Err(EBADF);
        }
        let mut fds = self.fds.lock();
        let e = fds.get_mut(fd as usize).and_then(|e| e.take()).ok_or(EBADF)?;
        Ok(e.file)
    }

    pub fn set_cloexec(&self, fd: i32, on: bool) -> Result<()> {
        let mut fds = self.fds.lock();
        let e = fds.get_mut(fd as usize).and_then(|e| e.as_mut()).ok_or(EBADF)?;
        e.cloexec = on;
        Ok(())
    }

    pub fn close_on_exec(&self) -> Vec<Arc<File>> {
        let mut fds = self.fds.lock();
        let mut dropped = Vec::new();
        for e in fds.iter_mut() {
            if e.as_ref().map(|x| x.cloexec).unwrap_or(false) {
                if let Some(x) = e.take() {
                    dropped.push(x.file);
                }
            }
        }
        dropped
    }

    /// Copy for fork.
    pub fn duplicate(&self) -> Arc<FdTable> {
        let fds = self.fds.lock();
        Arc::new(FdTable { fds: SpinLock::new(fds.clone()) })
    }

    /// Drop all descriptors (returns them so the caller can release outside the lock).
    pub fn clear(&self) -> Vec<Arc<File>> {
        let mut fds = self.fds.lock();
        let v: Vec<Arc<File>> = fds.drain(..).flatten().map(|e| e.file).collect();
        v
    }

    pub fn iter(&self) -> Vec<(i32, FdEntry)> {
        let fds = self.fds.lock();
        fds.iter().enumerate().filter_map(|(i, e)| e.clone().map(|e| (i as i32, e))).collect()
    }

    pub fn close_range(&self, lo: usize, hi: usize) -> Vec<Arc<File>> {
        let mut fds = self.fds.lock();
        let mut out = Vec::new();
        for i in lo..=hi.min(fds.len().saturating_sub(1)) {
            if let Some(e) = fds[i].take() {
                out.push(e.file);
            }
        }
        out
    }
}
