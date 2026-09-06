//! Virtual filesystem.

pub mod file;
pub mod initrd;
pub mod path;
pub mod pipe;
pub mod procfs;
pub mod tmpfs;
pub mod vfs;

use crate::mm::errno::*;
use alloc::sync::Arc;
use vfs::Inode;

/// Mount the initrd as root, populate /dev, /tmp, /proc.
pub fn init(boot: &crate::boot::limine::BootInfo) {
    let root: Arc<dyn Inode> = match boot.module("initrd") {
        Some(m) => initrd::load(m).expect("bad initrd image"),
        None => {
            klog!("fs", "no initrd module; using an empty tmpfs root");
            tmpfs::new_fs("rootfs")
        }
    };
    path::set_root(root.clone());
    // mount points inside the initrd root (create if missing)
    for d in ["dev", "tmp", "proc", "run", "sys", "mnt", "data"] {
        let _ = root.create(d, vfs::Kind::Dir, 0o755, 0);
    }
    let dev = tmpfs::new_fs("devtmpfs");
    path::mount("/dev", dev.clone()).expect("mount /dev");
    let _ = dev.create("pts", vfs::Kind::Dir, 0o755, 0);
    let _ = dev.create("input", vfs::Kind::Dir, 0o755, 0);
    let _ = dev.create("shm", vfs::Kind::Dir, 0o1777, 0);
    crate::dev::chardev::populate_devfs(&dev);
    path::mount("/tmp", tmpfs::new_fs("tmpfs")).expect("mount /tmp");
    path::mount("/run", tmpfs::new_fs("tmpfs")).expect("mount /run");
    path::mount("/proc", procfs::new_fs()).expect("mount /proc");
    klog!("fs", "root={} mounts: /dev /tmp /run /proc", initrd_name(&root));
}

fn initrd_name(root: &Arc<dyn Inode>) -> &'static str {
    if root.fs_name() == "initrd" {
        "initrd"
    } else {
        "tmpfs"
    }
}

pub fn lookup_abs(path: &str) -> Result<Arc<dyn Inode>> {
    path::resolve_path(&path::root(), path, true).map(|r| r.inode).and_then(|i| i.ok_or(ENOENT))
}
