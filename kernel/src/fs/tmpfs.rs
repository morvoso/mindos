//! In-memory filesystem backed by page frames (also used for /dev and memfd).

use super::vfs::{self, DirEntry, FileOps, Inode, Kind, Stat};
use crate::mm::errno::*;
use crate::mm::pmm;
use crate::mm::vmm;
use crate::sched::mutex::Mutex;
use crate::sync::SpinLock;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub struct TmpInode {
    ino: u64,
    fs_id: u64,
    fs_name: &'static str,
    kind: Kind,
    mode: AtomicU32,
    rdev: u64,
    nlink: AtomicU32,
    mtime_ns: AtomicU64,
    data: Mutex<FileData>,
    dir: SpinLock<DirData>,
    parent: SpinLock<Weak<TmpInode>>,
    self_ref: SpinLock<Weak<TmpInode>>,
    link: SpinLock<String>,
    /// Extra object attached to the inode (e.g. a bound unix socket).
    pub attachment: SpinLock<Option<Arc<dyn core::any::Any + Send + Sync>>>,
}

#[derive(Default)]
struct FileData {
    pages: Vec<Option<u64>>,
    size: u64,
}

#[derive(Default)]
struct DirData {
    entries: BTreeMap<String, Arc<TmpInode>>,
}

impl TmpInode {
    fn new(fs_id: u64, fs_name: &'static str, kind: Kind, mode: u32, rdev: u64) -> Arc<TmpInode> {
        let n = Arc::new(TmpInode {
            ino: vfs::next_ino(),
            fs_id,
            fs_name,
            kind,
            mode: AtomicU32::new(mode & 0o7777),
            rdev,
            nlink: AtomicU32::new(1),
            mtime_ns: AtomicU64::new(crate::dev::rtc::wall_time_ns()),
            data: Mutex::new(FileData::default()),
            dir: SpinLock::new(DirData::default()),
            parent: SpinLock::new(Weak::new()),
            self_ref: SpinLock::new(Weak::new()),
            link: SpinLock::new(String::new()),
            attachment: SpinLock::new(None),
        });
        *n.self_ref.lock() = Arc::downgrade(&n);
        n
    }

    fn touch(&self) {
        self.mtime_ns.store(crate::dev::rtc::wall_time_ns(), Ordering::Relaxed);
    }

    fn page_for(&self, d: &mut FileData, index: usize, alloc: bool) -> Result<Option<u64>> {
        if index >= d.pages.len() {
            if !alloc {
                return Ok(None);
            }
            d.pages.resize(index + 1, None);
        }
        if d.pages[index].is_none() {
            if !alloc {
                return Ok(None);
            }
            let pa = pmm::alloc_zeroed_ref().ok_or(ENOSPC)?;
            d.pages[index] = Some(pa);
        }
        Ok(d.pages[index])
    }

    pub fn is_tmp(i: &Arc<dyn Inode>) -> Option<&TmpInode> {
        i.as_any().downcast_ref::<TmpInode>()
    }
}

impl Drop for TmpInode {
    fn drop(&mut self) {
        let d = unsafe { self.data.get_unchecked() };
        for p in d.pages.iter().flatten() {
            pmm::ref_dec(*p);
        }
    }
}

impl Inode for TmpInode {
    fn kind(&self) -> Kind {
        self.kind
    }
    fn ino(&self) -> u64 {
        self.ino
    }
    fn fs_id(&self) -> u64 {
        self.fs_id
    }
    fn fs_name(&self) -> &'static str {
        self.fs_name
    }
    fn rdev(&self) -> u64 {
        self.rdev
    }
    fn stat(&self) -> Stat {
        let size = match self.kind {
            Kind::Dir => 4096,
            Kind::Symlink => self.link.lock().len() as u64,
            _ => self.data.lock().size,
        };
        let nlink = if self.kind == Kind::Dir { 2 + self.dir.lock().entries.values().filter(|c| c.kind == Kind::Dir).count() as u64 } else { self.nlink.load(Ordering::Relaxed) as u64 };
        let mut s = Stat::simple(self.fs_id, self.ino, self.kind, self.mode.load(Ordering::Relaxed), size, nlink);
        s.st_rdev = self.rdev;
        let t = self.mtime_ns.load(Ordering::Relaxed);
        s.st_mtime = (t / 1_000_000_000) as i64;
        s.st_mtime_nsec = (t % 1_000_000_000) as i64;
        s.st_ctime = s.st_mtime;
        s.st_ctime_nsec = s.st_mtime_nsec;
        s.st_atime = s.st_mtime;
        s.st_atime_nsec = s.st_mtime_nsec;
        if self.kind == Kind::File {
            let d = self.data.lock();
            s.st_blocks = d.pages.iter().flatten().count() as i64 * 8;
        }
        s
    }
    fn size(&self) -> u64 {
        match self.kind {
            Kind::Symlink => self.link.lock().len() as u64,
            Kind::Dir => 4096,
            _ => self.data.lock().size,
        }
    }
    fn mode(&self) -> u32 {
        self.mode.load(Ordering::Relaxed)
    }
    fn set_mode(&self, mode: u32) -> Result<()> {
        self.mode.store(mode & 0o7777, Ordering::Relaxed);
        Ok(())
    }
    fn set_times(&self, _a: Option<u64>, m: Option<u64>) -> Result<()> {
        if let Some(m) = m {
            self.mtime_ns.store(m, Ordering::Relaxed);
        }
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if self.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        if name == "." {
            return self.self_ref.lock().upgrade().map(|n| n as Arc<dyn Inode>).ok_or(ENOENT);
        }
        if name == ".." {
            return match self.parent.lock().upgrade() {
                Some(p) => Ok(p as Arc<dyn Inode>),
                None => self.self_ref.lock().upgrade().map(|n| n as Arc<dyn Inode>).ok_or(ENOENT),
            };
        }
        self.dir.lock().entries.get(name).cloned().map(|c| c as Arc<dyn Inode>).ok_or(ENOENT)
    }
    fn create(&self, name: &str, kind: Kind, mode: u32, rdev: u64) -> Result<Arc<dyn Inode>> {
        if self.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        if name.is_empty() || name == "." || name == ".." {
            return Err(EINVAL);
        }
        let me = self.self_ref.lock().upgrade().ok_or(ENOENT)?;
        let mut d = self.dir.lock();
        if d.entries.contains_key(name) {
            return Err(EEXIST);
        }
        let n = TmpInode::new(self.fs_id, self.fs_name, kind, mode, rdev);
        *n.parent.lock() = Arc::downgrade(&me);
        d.entries.insert(String::from(name), n.clone());
        self.touch();
        Ok(n)
    }
    fn symlink(&self, name: &str, target: &str) -> Result<Arc<dyn Inode>> {
        let n = self.create(name, Kind::Symlink, 0o777, 0)?;
        let t = n.as_any().downcast_ref::<TmpInode>().unwrap();
        *t.link.lock() = String::from(target);
        Ok(n)
    }
    fn link(&self, name: &str, target: &Arc<dyn Inode>) -> Result<()> {
        if self.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        let t = target.as_any().downcast_ref::<TmpInode>().ok_or(EXDEV)?;
        if t.fs_id != self.fs_id {
            return Err(EXDEV);
        }
        if t.kind == Kind::Dir {
            return Err(EPERM);
        }
        let tarc = t.self_ref.lock().upgrade().ok_or(ENOENT)?;
        let mut d = self.dir.lock();
        if d.entries.contains_key(name) {
            return Err(EEXIST);
        }
        t.nlink.fetch_add(1, Ordering::Relaxed);
        d.entries.insert(String::from(name), tarc);
        Ok(())
    }
    fn unlink(&self, name: &str) -> Result<()> {
        if self.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        let mut d = self.dir.lock();
        let e = d.entries.get(name).ok_or(ENOENT)?;
        if e.kind == Kind::Dir {
            return Err(EISDIR);
        }
        let e = d.entries.remove(name).unwrap();
        e.nlink.fetch_sub(1, Ordering::Relaxed);
        self.touch();
        Ok(())
    }
    fn rmdir(&self, name: &str) -> Result<()> {
        if self.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        let mut d = self.dir.lock();
        let e = d.entries.get(name).ok_or(ENOENT)?;
        if e.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        if !e.dir.lock().entries.is_empty() {
            return Err(ENOTEMPTY);
        }
        d.entries.remove(name);
        self.touch();
        Ok(())
    }
    fn rename(&self, old_name: &str, new_dir: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        let nd = new_dir.as_any().downcast_ref::<TmpInode>().ok_or(EXDEV)?;
        if nd.fs_id != self.fs_id {
            return Err(EXDEV);
        }
        let same = core::ptr::eq(nd, self);
        let entry = {
            let mut d = self.dir.lock();
            d.entries.remove(old_name).ok_or(ENOENT)?
        };
        let nd_arc = nd.self_ref.lock().upgrade().ok_or(ENOENT)?;
        {
            let mut d = nd.dir.lock();
            if let Some(existing) = d.entries.get(new_name) {
                if existing.kind == Kind::Dir && !existing.dir.lock().entries.is_empty() {
                    let _ = same;
                    self.dir.lock().entries.insert(String::from(old_name), entry);
                    return Err(ENOTEMPTY);
                }
            }
            *entry.parent.lock() = Arc::downgrade(&nd_arc);
            d.entries.insert(String::from(new_name), entry);
        }
        self.touch();
        nd.touch();
        Ok(())
    }
    fn readdir(&self, pos: usize) -> Result<Option<DirEntry>> {
        if self.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        if pos == 0 {
            return Ok(Some(DirEntry { name: String::from("."), ino: self.ino, kind: Kind::Dir }));
        }
        if pos == 1 {
            let ino = self.parent.lock().upgrade().map(|p| p.ino).unwrap_or(self.ino);
            return Ok(Some(DirEntry { name: String::from(".."), ino, kind: Kind::Dir }));
        }
        let d = self.dir.lock();
        Ok(d.entries.iter().nth(pos - 2).map(|(n, c)| DirEntry { name: n.clone(), ino: c.ino, kind: c.kind }))
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize> {
        if self.kind == Kind::Dir {
            return Err(EISDIR);
        }
        let mut d = self.data.lock();
        if off >= d.size {
            return Ok(0);
        }
        let n = ((d.size - off) as usize).min(buf.len());
        let mut done = 0;
        while done < n {
            let pos = off as usize + done;
            let idx = pos / 4096;
            let po = pos % 4096;
            let chunk = (4096 - po).min(n - done);
            match self.page_for(&mut d, idx, false)? {
                Some(pa) => unsafe {
                    core::ptr::copy_nonoverlapping((vmm::p2v(pa) + po) as *const u8, buf[done..].as_mut_ptr(), chunk);
                },
                None => buf[done..done + chunk].fill(0),
            }
            done += chunk;
        }
        Ok(n)
    }
    fn write_at(&self, off: u64, buf: &[u8]) -> Result<usize> {
        if self.kind == Kind::Dir {
            return Err(EISDIR);
        }
        let mut d = self.data.lock();
        let mut done = 0;
        while done < buf.len() {
            let pos = off as usize + done;
            let idx = pos / 4096;
            let po = pos % 4096;
            let chunk = (4096 - po).min(buf.len() - done);
            let pa = self.page_for(&mut d, idx, true)?.unwrap();
            unsafe {
                core::ptr::copy_nonoverlapping(buf[done..].as_ptr(), (vmm::p2v(pa) + po) as *mut u8, chunk);
            }
            done += chunk;
        }
        let end = off + buf.len() as u64;
        if end > d.size {
            d.size = end;
        }
        self.touch();
        Ok(buf.len())
    }
    fn truncate(&self, size: u64) -> Result<()> {
        if self.kind == Kind::Dir {
            return Err(EISDIR);
        }
        let mut d = self.data.lock();
        let npages = ((size + 4095) / 4096) as usize;
        if npages < d.pages.len() {
            for p in d.pages.drain(npages..).flatten() {
                pmm::ref_dec(p);
            }
        }
        // zero the tail of the last page when shrinking
        if size < d.size && size % 4096 != 0 {
            if let Some(Some(pa)) = d.pages.get((size / 4096) as usize) {
                let off = (size % 4096) as usize;
                unsafe {
                    core::ptr::write_bytes((vmm::p2v(*pa) + off) as *mut u8, 0, 4096 - off);
                }
            }
        }
        d.size = size;
        self.touch();
        Ok(())
    }
    fn readlink(&self) -> Result<String> {
        if self.kind != Kind::Symlink {
            return Err(EINVAL);
        }
        Ok(self.link.lock().clone())
    }
    fn get_page(&self, index: u64) -> Result<u64> {
        if self.kind != Kind::File {
            return Err(ENODEV);
        }
        let mut d = self.data.lock();
        let pa = self.page_for(&mut d, index as usize, true)?.unwrap();
        pmm::ref_inc(pa);
        Ok(pa)
    }
    fn open(&self, flags: u32) -> Result<Option<Arc<dyn FileOps>>> {
        match self.kind {
            Kind::CharDev => crate::dev::chardev::open(self.rdev, flags).map(Some),
            Kind::Fifo => {
                let me = self.self_ref.lock().upgrade().ok_or(ENOENT)?;
                super::pipe::open_fifo(me, flags).map(Some)
            }
            Kind::Socket => Err(ENXIO),
            _ => Ok(None),
        }
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

pub fn new_fs(name: &'static str) -> Arc<dyn Inode> {
    TmpInode::new(vfs::next_fs_id(), name, Kind::Dir, 0o755, 0)
}

/// An unlinked regular file (memfd, shared anonymous memory).
pub fn anonymous_file(name: &'static str) -> Arc<dyn Inode> {
    static FS: crate::sync::Once<u64> = crate::sync::Once::new();
    let id = *FS.call_once(vfs::next_fs_id);
    TmpInode::new(id, name, Kind::File, 0o600, 0)
}
