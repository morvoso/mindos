//! Character device registry and the simple memory devices.

use crate::fs::vfs::{self, File, FileOps, Inode, Kind};
use crate::mm::errno::*;
use alloc::sync::Arc;

pub const MAJOR_MEM: u32 = 1;
pub const MAJOR_TTY: u32 = 4;
pub const MAJOR_CONSOLE: u32 = 5;
pub const MAJOR_MISC: u32 = 10;
pub const MAJOR_INPUT: u32 = 13;
pub const MAJOR_FB: u32 = 29;
pub const MAJOR_PTS: u32 = 136;

/// Create the standard device nodes in the devfs root.
pub fn populate_devfs(dev: &Arc<dyn Inode>) {
    let mk = |name: &str, major: u32, minor: u32, mode: u32| {
        let _ = dev.create(name, Kind::CharDev, mode, vfs::makedev(major, minor));
    };
    mk("null", MAJOR_MEM, 3, 0o666);
    mk("zero", MAJOR_MEM, 5, 0o666);
    mk("full", MAJOR_MEM, 7, 0o666);
    mk("random", MAJOR_MEM, 8, 0o666);
    mk("urandom", MAJOR_MEM, 9, 0o666);
    mk("kmsg", MAJOR_MEM, 11, 0o644);
    mk("tty", MAJOR_CONSOLE, 0, 0o666);
    mk("console", MAJOR_CONSOLE, 1, 0o600);
    mk("ptmx", MAJOR_CONSOLE, 2, 0o666);
    mk("tty1", MAJOR_TTY, 1, 0o620);
    mk("ttyS0", MAJOR_TTY, 64, 0o660);
    mk("fb0", MAJOR_FB, 0, 0o660);
    mk("mind", MAJOR_MISC, 200, 0o666);
    if let Ok(input) = dev.lookup("input") {
        let _ = input.create("event0", Kind::CharDev, 0o660, vfs::makedev(MAJOR_INPUT, 64));
        let _ = input.create("mice", Kind::CharDev, 0o660, vfs::makedev(MAJOR_INPUT, 63));
    }
    let _ = dev.symlink("stdin", "/proc/self/fd/0");
    let _ = dev.symlink("stdout", "/proc/self/fd/1");
    let _ = dev.symlink("stderr", "/proc/self/fd/2");
    let _ = dev.symlink("fd", "/proc/self/fd");
}

/// Open a character device by device number.
pub fn open(rdev: u64, flags: u32) -> Result<Arc<dyn FileOps>> {
    let (major, minor) = (vfs::major(rdev), vfs::minor(rdev));
    match (major, minor) {
        (MAJOR_MEM, 3) => Ok(Arc::new(NullDev)),
        (MAJOR_MEM, 5) => Ok(Arc::new(ZeroDev { full: false })),
        (MAJOR_MEM, 7) => Ok(Arc::new(ZeroDev { full: true })),
        (MAJOR_MEM, 8) | (MAJOR_MEM, 9) => Ok(Arc::new(RandomDev)),
        (MAJOR_MEM, 11) => Ok(Arc::new(KmsgDev)),
        (MAJOR_CONSOLE, 0) => super::tty::open_controlling(flags),
        (MAJOR_CONSOLE, 1) | (MAJOR_TTY, 1) | (MAJOR_TTY, 64) => super::tty::open_console(flags),
        (MAJOR_CONSOLE, 2) => super::tty::open_ptmx(flags),
        (MAJOR_PTS, n) => super::tty::open_pts(n, flags),
        (MAJOR_FB, 0) => super::fbdev::open(flags),
        (MAJOR_INPUT, 64) => super::evdev::open(flags),
        (MAJOR_INPUT, 63) => Err(ENODEV),
        (MAJOR_MISC, 200) => crate::proc::mind::open(flags),
        _ => Err(ENXIO),
    }
}

pub struct NullDev;
impl FileOps for NullDev {
    fn read(&self, _f: &File, _b: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
    fn write(&self, _f: &File, b: &[u8]) -> Result<usize> {
        Ok(b.len())
    }
    fn lseek(&self, _f: &File, _o: i64, _w: u32) -> Result<u64> {
        Ok(0)
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

pub struct ZeroDev {
    full: bool,
}
impl FileOps for ZeroDev {
    fn read(&self, _f: &File, b: &mut [u8]) -> Result<usize> {
        b.fill(0);
        Ok(b.len())
    }
    fn write(&self, _f: &File, b: &[u8]) -> Result<usize> {
        if self.full {
            Err(ENOSPC)
        } else {
            Ok(b.len())
        }
    }
    fn lseek(&self, _f: &File, _o: i64, _w: u32) -> Result<u64> {
        Ok(0)
    }
    fn mmap(&self, _file: &File, _prot: u32, shared: bool) -> Result<crate::mm::addrspace::Backing> {
        if shared {
            Ok(crate::mm::addrspace::Backing::File { inode: crate::fs::tmpfs::anonymous_file("dev/zero"), offset: 0 })
        } else {
            Ok(crate::mm::addrspace::Backing::Anon)
        }
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

pub struct RandomDev;
impl FileOps for RandomDev {
    fn read(&self, _f: &File, b: &mut [u8]) -> Result<usize> {
        fill_random(b);
        Ok(b.len())
    }
    fn write(&self, _f: &File, b: &[u8]) -> Result<usize> {
        Ok(b.len())
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// Fill a buffer with random bytes (RDRAND with a xorshift fallback).
pub fn fill_random(b: &mut [u8]) {
    use core::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
    let mut i = 0;
    while i < b.len() {
        let v = match crate::arch::x86_64::cpu::rdrand64() {
            Some(v) => v,
            None => {
                let mut x = STATE.load(Ordering::Relaxed) ^ crate::arch::x86_64::cpu::rdtsc();
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                STATE.store(x, Ordering::Relaxed);
                x
            }
        };
        let bytes = v.to_le_bytes();
        let n = (b.len() - i).min(8);
        b[i..i + n].copy_from_slice(&bytes[..n]);
        i += n;
    }
}

pub struct KmsgDev;
impl FileOps for KmsgDev {
    fn read(&self, f: &File, b: &mut [u8]) -> Result<usize> {
        let pos = f.pos() as usize;
        let (n, newpos) = crate::console::CONSOLE.lock().log.read_from(pos, b);
        f.set_pos(newpos as u64);
        Ok(n)
    }
    fn write(&self, _f: &File, b: &[u8]) -> Result<usize> {
        let s = core::str::from_utf8(b).unwrap_or("<bad utf8>");
        klog!("user", "{}", s.trim_end());
        Ok(b.len())
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
