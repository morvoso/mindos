//! Path resolution and the mount table.

use super::vfs::{Inode, Kind};
use crate::mm::errno::*;
use crate::sync::SpinLock;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub struct Mount {
    /// Identity (fs_id, ino) of the directory the filesystem is mounted on.
    pub on: (u64, u64),
    pub mountpoint: Arc<dyn Inode>,
    pub root: Arc<dyn Inode>,
    pub path: String,
    pub fstype: &'static str,
}

static ROOT: SpinLock<Option<Arc<dyn Inode>>> = SpinLock::new(None);
static MOUNTS: SpinLock<Vec<Mount>> = SpinLock::new(Vec::new());

pub fn set_root(root: Arc<dyn Inode>) {
    *ROOT.lock() = Some(root);
}
pub fn root() -> Arc<dyn Inode> {
    ROOT.lock().clone().expect("no root filesystem")
}

fn ident(i: &Arc<dyn Inode>) -> (u64, u64) {
    (i.fs_id(), i.ino())
}

pub fn mount(path: &str, fs_root: Arc<dyn Inode>) -> Result<()> {
    let r = resolve_path(&root(), path, true)?;
    let mp = r.inode.ok_or(ENOENT)?;
    if mp.kind() != Kind::Dir {
        return Err(ENOTDIR);
    }
    let fstype = fs_root.fs_name();
    MOUNTS.lock().push(Mount { on: ident(&mp), mountpoint: mp, root: fs_root, path: normalize(path), fstype });
    Ok(())
}

pub fn umount(path: &str) -> Result<()> {
    let p = normalize(path);
    let mut m = MOUNTS.lock();
    let idx = m.iter().position(|x| x.path == p).ok_or(EINVAL)?;
    m.remove(idx);
    Ok(())
}

pub fn mounts() -> Vec<(String, &'static str)> {
    let mut v = alloc::vec![(String::from("/"), root().fs_name())];
    for m in MOUNTS.lock().iter() {
        v.push((m.path.clone(), m.fstype));
    }
    v
}

/// If `dir` is a mount point, return the mounted root.
fn cross_mount(dir: Arc<dyn Inode>) -> Arc<dyn Inode> {
    let id = ident(&dir);
    let m = MOUNTS.lock();
    for x in m.iter().rev() {
        if x.on == id {
            return x.root.clone();
        }
    }
    dir
}

/// If `dir` is the root of a mounted fs, return the mount point beneath it.
fn uncross_mount(dir: Arc<dyn Inode>) -> Arc<dyn Inode> {
    let id = ident(&dir);
    let m = MOUNTS.lock();
    for x in m.iter() {
        if ident(&x.root) == id {
            return x.mountpoint.clone();
        }
    }
    dir
}

pub struct Resolved {
    /// Directory containing the last component.
    pub parent: Arc<dyn Inode>,
    /// The target itself, if it exists.
    pub inode: Option<Arc<dyn Inode>>,
    /// Last path component ("" for the root).
    pub name: String,
}

/// Resolve `path` relative to `base`. With `follow`, a trailing symlink is followed.
pub fn resolve_path(base: &Arc<dyn Inode>, path: &str, follow: bool) -> Result<Resolved> {
    resolve_inner(base, path, follow, 0)
}

fn resolve_inner(base: &Arc<dyn Inode>, path: &str, follow: bool, depth: u32) -> Result<Resolved> {
    if depth > 40 {
        return Err(ELOOP);
    }
    if path.len() > 4096 {
        return Err(ENAMETOOLONG);
    }
    let mut cur: Arc<dyn Inode> = if path.starts_with('/') { root() } else { base.clone() };
    let comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    if comps.is_empty() {
        // "/" or "" (relative to base)
        return Ok(Resolved { parent: cur.clone(), inode: Some(cur), name: String::new() });
    }
    let trailing_slash = path.ends_with('/');
    let n = comps.len();
    let mut parent = cur.clone();
    for (i, comp) in comps.iter().enumerate() {
        let last = i == n - 1;
        if comp.len() > 255 {
            return Err(ENAMETOOLONG);
        }
        if cur.kind() != Kind::Dir {
            return Err(ENOTDIR);
        }
        let next = match *comp {
            "." => Some(cur.clone()),
            ".." => {
                let base_dir = uncross_mount(cur.clone());
                if ident(&base_dir) == ident(&root()) {
                    Some(root())
                } else {
                    Some(base_dir.lookup("..")?)
                }
            }
            name => match cur.lookup(name) {
                Ok(i) => Some(cross_mount(i)),
                Err(Errno(2)) => None,
                Err(e) => return Err(e),
            },
        };
        match next {
            None => {
                if last {
                    return Ok(Resolved { parent: cur, inode: None, name: String::from(*comp) });
                }
                return Err(ENOENT);
            }
            Some(next) => {
                if next.kind() == Kind::Symlink && (!last || follow || trailing_slash) {
                    let target = next.readlink()?;
                    if last {
                        let mut r = resolve_inner(&cur, &target, true, depth + 1)?;
                        if trailing_slash {
                            if let Some(i) = &r.inode {
                                if i.kind() != Kind::Dir {
                                    return Err(ENOTDIR);
                                }
                            }
                        }
                        if r.name.is_empty() {
                            r.name = String::from(*comp);
                        }
                        return Ok(r);
                    }
                    let r = resolve_inner(&cur, &target, true, depth + 1)?;
                    cur = r.inode.ok_or(ENOENT)?;
                    parent = r.parent;
                    continue;
                }
                if last {
                    if trailing_slash && next.kind() != Kind::Dir {
                        return Err(ENOTDIR);
                    }
                    return Ok(Resolved { parent: cur, inode: Some(next), name: String::from(*comp) });
                }
                parent = cur;
                cur = next;
            }
        }
    }
    Ok(Resolved { parent, inode: Some(cur), name: String::from(comps[n - 1]) })
}

/// Lexically normalize an absolute path.
pub fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            x => parts.push(x),
        }
    }
    let mut s = String::from("/");
    s.push_str(&parts.join("/"));
    s
}

/// Join `rel` onto absolute `base` lexically.
pub fn join(base: &str, rel: &str) -> String {
    if rel.starts_with('/') {
        normalize(rel)
    } else {
        let mut s = String::from(base);
        if !s.ends_with('/') {
            s.push('/');
        }
        s.push_str(rel);
        normalize(&s)
    }
}
