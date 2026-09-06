//! Read-only filesystem over the MINDINIT initrd image (zero-copy).

use super::vfs::{DirEntry, Inode, Kind, Stat};
use crate::boot::limine::Module;
use crate::mm::errno::*;
use crate::mm::vmm;
use crate::sync::SpinLock;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

struct Node {
    ino: u64,
    fs_id: u64,
    kind: Kind,
    mode: u32,
    data: usize, // virtual address of file data (page aligned)
    phys: u64,
    size: u64,
    parent: SpinLock<Weak<Node>>,
    children: SpinLock<Vec<(String, Arc<Node>)>>,
    link: String,
    self_ref: SpinLock<Weak<Node>>,
}

impl Node {
    fn new(fs_id: u64, kind: Kind, mode: u32, data: usize, phys: u64, size: u64, link: String) -> Arc<Node> {
        let n = Arc::new(Node {
            ino: super::vfs::next_ino(),
            fs_id,
            kind,
            mode,
            data,
            phys,
            size,
            parent: SpinLock::new(Weak::new()),
            children: SpinLock::new(Vec::new()),
            link,
            self_ref: SpinLock::new(Weak::new()),
        });
        *n.self_ref.lock() = Arc::downgrade(&n);
        n
    }
    fn child(&self, name: &str) -> Option<Arc<Node>> {
        self.children.lock().iter().find(|(n, _)| n == name).map(|(_, c)| c.clone())
    }
    fn ensure_dir(self: &Arc<Node>, name: &str) -> Arc<Node> {
        if let Some(c) = self.child(name) {
            return c;
        }
        let d = Node::new(self.fs_id, Kind::Dir, 0o755, 0, 0, 0, String::new());
        *d.parent.lock() = Arc::downgrade(self);
        self.children.lock().push((String::from(name), d.clone()));
        d
    }
}

impl Inode for Node {
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
        "initrd"
    }
    fn stat(&self) -> Stat {
        let nlink = if self.kind == Kind::Dir { 2 + self.children.lock().iter().filter(|(_, c)| c.kind == Kind::Dir).count() as u64 } else { 1 };
        let size = if self.kind == Kind::Symlink { self.link.len() as u64 } else { self.size };
        Stat::simple(self.fs_id, self.ino, self.kind, self.mode, size, nlink)
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
        self.child(name).map(|c| c as Arc<dyn Inode>).ok_or(ENOENT)
    }
    fn create(&self, name: &str, kind: Kind, mode: u32, _rdev: u64) -> Result<Arc<dyn Inode>> {
        // Only directories may be created (for mount points); everything else is read-only.
        if self.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        if kind != Kind::Dir {
            return Err(EROFS);
        }
        if self.child(name).is_some() {
            return Err(EEXIST);
        }
        let me = self.self_ref.lock().upgrade().ok_or(ENOENT)?;
        let d = Node::new(self.fs_id, Kind::Dir, mode, 0, 0, 0, String::new());
        *d.parent.lock() = Arc::downgrade(&me);
        self.children.lock().push((String::from(name), d.clone()));
        Ok(d)
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
        let ch = self.children.lock();
        Ok(ch.get(pos - 2).map(|(n, c)| DirEntry { name: n.clone(), ino: c.ino, kind: c.kind }))
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize> {
        if self.kind == Kind::Dir {
            return Err(EISDIR);
        }
        if off >= self.size {
            return Ok(0);
        }
        let n = ((self.size - off) as usize).min(buf.len());
        unsafe {
            core::ptr::copy_nonoverlapping((self.data + off as usize) as *const u8, buf.as_mut_ptr(), n);
        }
        Ok(n)
    }
    fn readlink(&self) -> Result<String> {
        if self.kind != Kind::Symlink {
            return Err(EINVAL);
        }
        Ok(self.link.clone())
    }
    fn get_page(&self, index: u64) -> Result<u64> {
        if self.kind != Kind::File {
            return Err(ENODEV);
        }
        let pages = (self.size + 4095) / 4096;
        if index >= pages {
            // beyond EOF: hand out a fresh zero page
            return crate::mm::pmm::alloc_zeroed_ref().ok_or(ENOMEM);
        }
        let pa = self.phys + index * 4096;
        // last partial page: the image zero-pads files to a page boundary
        crate::mm::pmm::ref_inc(pa);
        Ok(pa)
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn rd_u64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

pub fn load(m: &Module) -> Result<Arc<dyn Inode>> {
    let img = unsafe { core::slice::from_raw_parts(m.virt as *const u8, m.size) };
    if img.len() < 32 || &img[0..8] != b"MINDINIT" {
        return Err(EINVAL);
    }
    let count = rd_u32(img, 12) as usize;
    let strtab_off = rd_u32(img, 16) as usize;
    let strtab_size = rd_u32(img, 20) as usize;
    let entries_off = rd_u32(img, 24) as usize;
    let strtab = &img[strtab_off..strtab_off + strtab_size];
    let fs_id = super::vfs::next_fs_id();
    let root = Node::new(fs_id, Kind::Dir, 0o755, 0, 0, 0, String::new());
    let mut files = 0usize;
    let mut bytes = 0u64;
    for i in 0..count {
        let e = entries_off + i * 32;
        let path_off = rd_u32(img, e) as usize;
        let path_len = rd_u32(img, e + 4) as usize;
        let data_off = rd_u64(img, e + 8) as usize;
        let size = rd_u64(img, e + 16);
        let mode = rd_u32(img, e + 24);
        let kind = rd_u32(img, e + 28);
        let path = core::str::from_utf8(&strtab[path_off..path_off + path_len]).map_err(|_| EINVAL)?;
        let comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
        if comps.is_empty() {
            continue;
        }
        let mut dir = root.clone();
        for c in &comps[..comps.len() - 1] {
            dir = dir.ensure_dir(c);
        }
        let name = comps[comps.len() - 1];
        match kind {
            1 => {
                let d = dir.ensure_dir(name);
                let _ = d;
            }
            2 => {
                let target = core::str::from_utf8(&img[data_off..data_off + size as usize]).map_err(|_| EINVAL)?;
                let n = Node::new(fs_id, Kind::Symlink, 0o777, 0, 0, size, String::from(target));
                *n.parent.lock() = Arc::downgrade(&dir);
                dir.children.lock().push((String::from(name), n));
            }
            _ => {
                let vaddr = m.virt + data_off;
                let paddr = m.phys + data_off as u64;
                let n = Node::new(fs_id, Kind::File, mode & 0o7777, vaddr, paddr, size, String::new());
                *n.parent.lock() = Arc::downgrade(&dir);
                dir.children.lock().push((String::from(name), n));
                files += 1;
                bytes += size;
            }
        }
    }
    let _ = vmm::hhdm;
    klog!("initrd", "{} entries, {} files, {} KiB", count, files, bytes / 1024);
    Ok(root)
}
