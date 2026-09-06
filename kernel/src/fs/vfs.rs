//! Core VFS types: inodes, files, directory entries.

use crate::mm::errno::*;
use crate::sched::wait::WaitQueue;
use crate::sync::SpinLock;
use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    CharDev,
    BlockDev,
    Fifo,
    Socket,
}

impl Kind {
    pub fn mode_bits(self) -> u32 {
        match self {
            Kind::File => S_IFREG,
            Kind::Dir => S_IFDIR,
            Kind::Symlink => S_IFLNK,
            Kind::CharDev => S_IFCHR,
            Kind::BlockDev => S_IFBLK,
            Kind::Fifo => S_IFIFO,
            Kind::Socket => S_IFSOCK,
        }
    }
    pub fn dtype(self) -> u8 {
        match self {
            Kind::File => DT_REG,
            Kind::Dir => DT_DIR,
            Kind::Symlink => DT_LNK,
            Kind::CharDev => DT_CHR,
            Kind::BlockDev => DT_BLK,
            Kind::Fifo => DT_FIFO,
            Kind::Socket => DT_SOCK,
        }
    }
    pub fn from_mode(mode: u32) -> Kind {
        match mode & S_IFMT {
            S_IFDIR => Kind::Dir,
            S_IFLNK => Kind::Symlink,
            S_IFCHR => Kind::CharDev,
            S_IFBLK => Kind::BlockDev,
            S_IFIFO => Kind::Fifo,
            S_IFSOCK => Kind::Socket,
            _ => Kind::File,
        }
    }
}

pub const S_IFMT: u32 = 0o170000;
pub const S_IFSOCK: u32 = 0o140000;
pub const S_IFLNK: u32 = 0o120000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFBLK: u32 = 0o060000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFIFO: u32 = 0o010000;

pub const DT_UNKNOWN: u8 = 0;
pub const DT_FIFO: u8 = 1;
pub const DT_CHR: u8 = 2;
pub const DT_DIR: u8 = 4;
pub const DT_BLK: u8 = 6;
pub const DT_REG: u8 = 8;
pub const DT_LNK: u8 = 10;
pub const DT_SOCK: u8 = 12;

// open(2) flags
pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_ACCMODE: u32 = 3;
pub const O_CREAT: u32 = 0o100;
pub const O_EXCL: u32 = 0o200;
pub const O_NOCTTY: u32 = 0o400;
pub const O_TRUNC: u32 = 0o1000;
pub const O_APPEND: u32 = 0o2000;
pub const O_NONBLOCK: u32 = 0o4000;
pub const O_DSYNC: u32 = 0o10000;
pub const O_DIRECTORY: u32 = 0o200000;
pub const O_NOFOLLOW: u32 = 0o400000;
pub const O_CLOEXEC: u32 = 0o2000000;
pub const O_PATH: u32 = 0o10000000;
pub const O_TMPFILE: u32 = 0o20200000;

pub const AT_FDCWD: i32 = -100;
pub const AT_SYMLINK_NOFOLLOW: u32 = 0x100;
pub const AT_REMOVEDIR: u32 = 0x200;
pub const AT_SYMLINK_FOLLOW: u32 = 0x400;
pub const AT_EMPTY_PATH: u32 = 0x1000;

// poll events
pub const POLLIN: u32 = 0x1;
pub const POLLPRI: u32 = 0x2;
pub const POLLOUT: u32 = 0x4;
pub const POLLERR: u32 = 0x8;
pub const POLLHUP: u32 = 0x10;
pub const POLLNVAL: u32 = 0x20;
pub const POLLRDNORM: u32 = 0x40;
pub const POLLWRNORM: u32 = 0x100;
pub const POLLRDHUP: u32 = 0x2000;

/// Linux `struct stat` for x86_64.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Stat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_nlink: u64,
    pub st_mode: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub __pad0: u32,
    pub st_rdev: u64,
    pub st_size: i64,
    pub st_blksize: i64,
    pub st_blocks: i64,
    pub st_atime: i64,
    pub st_atime_nsec: i64,
    pub st_mtime: i64,
    pub st_mtime_nsec: i64,
    pub st_ctime: i64,
    pub st_ctime_nsec: i64,
    pub __unused: [i64; 3],
}

impl Stat {
    pub fn simple(dev: u64, ino: u64, kind: Kind, perm: u32, size: u64, nlink: u64) -> Stat {
        let mut s = Stat::default();
        s.st_dev = dev;
        s.st_ino = ino;
        s.st_nlink = nlink;
        s.st_mode = kind.mode_bits() | (perm & 0o7777);
        s.st_size = size as i64;
        s.st_blksize = 4096;
        s.st_blocks = ((size + 511) / 512) as i64;
        let t = crate::dev::rtc::wall_time_ns() as i64;
        s.st_atime = t / 1_000_000_000;
        s.st_mtime = s.st_atime;
        s.st_ctime = s.st_atime;
        s
    }
}

#[derive(Clone)]
pub struct DirEntry {
    pub name: String,
    pub ino: u64,
    pub kind: Kind,
}

pub fn makedev(major: u32, minor: u32) -> u64 {
    (((major as u64) & 0xfff) << 8) | ((minor as u64) & 0xff) | (((minor as u64) & !0xff) << 12) | (((major as u64) & !0xfff) << 32)
}
pub fn major(dev: u64) -> u32 {
    (((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff)) as u32
}
pub fn minor(dev: u64) -> u32 {
    ((dev & 0xff) | ((dev >> 12) & !0xff)) as u32
}

/// An inode: a node of a filesystem. All methods are non-blocking with
/// respect to user input (they may sleep on locks).
pub trait Inode: Send + Sync {
    fn kind(&self) -> Kind;
    fn ino(&self) -> u64;
    fn fs_id(&self) -> u64;
    fn fs_name(&self) -> &'static str;
    fn stat(&self) -> Stat;
    fn size(&self) -> u64 {
        self.stat().st_size as u64
    }
    fn mode(&self) -> u32 {
        self.stat().st_mode & 0o7777
    }
    fn rdev(&self) -> u64 {
        0
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>> {
        Err(ENOTDIR)
    }
    fn create(&self, _name: &str, _kind: Kind, _mode: u32, _rdev: u64) -> Result<Arc<dyn Inode>> {
        Err(EROFS)
    }
    fn symlink(&self, _name: &str, _target: &str) -> Result<Arc<dyn Inode>> {
        Err(EROFS)
    }
    fn link(&self, _name: &str, _target: &Arc<dyn Inode>) -> Result<()> {
        Err(EROFS)
    }
    fn unlink(&self, _name: &str) -> Result<()> {
        Err(EROFS)
    }
    fn rmdir(&self, _name: &str) -> Result<()> {
        Err(EROFS)
    }
    /// Move `old_name` in this directory to `new_name` in `new_dir` (same fs).
    fn rename(&self, _old_name: &str, _new_dir: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        Err(EROFS)
    }
    /// The `pos`-th directory entry (0-based), if any.
    fn readdir(&self, _pos: usize) -> Result<Option<DirEntry>> {
        Err(ENOTDIR)
    }

    fn read_at(&self, _off: u64, _buf: &mut [u8]) -> Result<usize> {
        Err(EINVAL)
    }
    fn write_at(&self, _off: u64, _buf: &[u8]) -> Result<usize> {
        Err(EINVAL)
    }
    fn truncate(&self, _size: u64) -> Result<()> {
        Err(EINVAL)
    }
    fn readlink(&self) -> Result<String> {
        Err(EINVAL)
    }
    /// Physical page for file page `index`; a reference is taken for the caller.
    fn get_page(&self, _index: u64) -> Result<u64> {
        Err(ENODEV)
    }
    fn set_mode(&self, _mode: u32) -> Result<()> {
        Ok(())
    }
    fn set_times(&self, _atime_ns: Option<u64>, _mtime_ns: Option<u64>) -> Result<()> {
        Ok(())
    }
    fn sync(&self) -> Result<()> {
        Ok(())
    }
    /// Device inodes supply their own file operations.
    fn open(&self, _flags: u32) -> Result<Option<Arc<dyn FileOps>>> {
        Ok(None)
    }
    /// Object identity for socket binding etc.
    fn as_any(&self) -> &dyn core::any::Any;
}

/// Operations on an open file description.
pub trait FileOps: Send + Sync {
    fn read(&self, file: &File, buf: &mut [u8]) -> Result<usize>;
    fn write(&self, file: &File, buf: &[u8]) -> Result<usize>;
    fn pread(&self, file: &File, buf: &mut [u8], off: u64) -> Result<usize> {
        let _ = (file, buf, off);
        Err(ESPIPE)
    }
    fn pwrite(&self, file: &File, buf: &[u8], off: u64) -> Result<usize> {
        let _ = (file, buf, off);
        Err(ESPIPE)
    }
    fn lseek(&self, _file: &File, _off: i64, _whence: u32) -> Result<u64> {
        Err(ESPIPE)
    }
    fn ioctl(&self, _file: &File, _cmd: u32, _arg: usize) -> Result<usize> {
        Err(ENOTTY)
    }
    /// Current readiness mask (POLL*), without blocking.
    fn poll(&self, _file: &File) -> u32 {
        POLLIN | POLLOUT
    }
    /// Wait queue signalled on readiness changes.
    fn poll_wait(&self) -> Option<&WaitQueue> {
        None
    }
    fn mmap(&self, file: &File, _prot: u32, _shared: bool) -> Result<crate::mm::addrspace::Backing> {
        match &file.inode {
            Some(i) if i.kind() == Kind::File => Ok(crate::mm::addrspace::Backing::File { inode: i.clone(), offset: 0 }),
            _ => Err(ENODEV),
        }
    }
    fn fsync(&self, _file: &File) -> Result<()> {
        Ok(())
    }
    /// Called when the last reference to the open file goes away.
    fn release(&self, _file: &File) {}
    fn readdir(&self, file: &File, pos: usize) -> Result<Option<DirEntry>> {
        match &file.inode {
            Some(i) => i.readdir(pos),
            None => Err(ENOTDIR),
        }
    }
    fn as_any(&self) -> &dyn core::any::Any;
}

/// An open file description (shared by dup'd descriptors).
pub struct File {
    pub inode: Option<Arc<dyn Inode>>,
    pub ops: Arc<dyn FileOps>,
    pub flags: AtomicU32,
    pub pos: AtomicU64,
    pub path: SpinLock<String>,
}

impl File {
    pub fn new(inode: Option<Arc<dyn Inode>>, ops: Arc<dyn FileOps>, flags: u32, path: &str) -> Arc<File> {
        Arc::new(File { inode, ops, flags: AtomicU32::new(flags), pos: AtomicU64::new(0), path: SpinLock::new(String::from(path)) })
    }
    pub fn flags(&self) -> u32 {
        self.flags.load(Ordering::Relaxed)
    }
    pub fn set_flags(&self, f: u32) {
        self.flags.store(f, Ordering::Relaxed);
    }
    pub fn nonblock(&self) -> bool {
        self.flags() & O_NONBLOCK != 0
    }
    pub fn readable(&self) -> bool {
        let m = self.flags() & O_ACCMODE;
        m == O_RDONLY || m == O_RDWR
    }
    pub fn writable(&self) -> bool {
        let m = self.flags() & O_ACCMODE;
        m == O_WRONLY || m == O_RDWR
    }
    pub fn pos(&self) -> u64 {
        self.pos.load(Ordering::Relaxed)
    }
    pub fn set_pos(&self, p: u64) {
        self.pos.store(p, Ordering::Relaxed);
    }
    pub fn path(&self) -> String {
        self.path.lock().clone()
    }
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if !self.readable() {
            return Err(EBADF);
        }
        self.ops.read(self, buf)
    }
    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        if !self.writable() {
            return Err(EBADF);
        }
        self.ops.write(self, buf)
    }
    pub fn kind(&self) -> Option<Kind> {
        self.inode.as_ref().map(|i| i.kind())
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let ops = self.ops.clone();
        ops.release(self);
    }
}

/// Generic file operations for regular files and directories on any inode.
pub struct RegularOps;

impl FileOps for RegularOps {
    fn read(&self, file: &File, buf: &mut [u8]) -> Result<usize> {
        let inode = file.inode.as_ref().ok_or(EBADF)?;
        if inode.kind() == Kind::Dir {
            return Err(EISDIR);
        }
        let pos = file.pos();
        let n = inode.read_at(pos, buf)?;
        file.set_pos(pos + n as u64);
        Ok(n)
    }
    fn write(&self, file: &File, buf: &[u8]) -> Result<usize> {
        let inode = file.inode.as_ref().ok_or(EBADF)?;
        if inode.kind() == Kind::Dir {
            return Err(EISDIR);
        }
        let pos = if file.flags() & O_APPEND != 0 { inode.size() } else { file.pos() };
        let n = inode.write_at(pos, buf)?;
        file.set_pos(pos + n as u64);
        Ok(n)
    }
    fn pread(&self, file: &File, buf: &mut [u8], off: u64) -> Result<usize> {
        let inode = file.inode.as_ref().ok_or(EBADF)?;
        inode.read_at(off, buf)
    }
    fn pwrite(&self, file: &File, buf: &[u8], off: u64) -> Result<usize> {
        let inode = file.inode.as_ref().ok_or(EBADF)?;
        inode.write_at(off, buf)
    }
    fn lseek(&self, file: &File, off: i64, whence: u32) -> Result<u64> {
        let inode = file.inode.as_ref().ok_or(EBADF)?;
        let base: i64 = match whence {
            0 => 0,
            1 => file.pos() as i64,
            2 => inode.size() as i64,
            3 | 4 => {
                // SEEK_DATA / SEEK_HOLE
                let size = inode.size() as i64;
                if off >= size {
                    return Err(ENXIO);
                }
                if whence == 3 {
                    file.set_pos(off as u64);
                    return Ok(off as u64);
                }
                file.set_pos(size as u64);
                return Ok(size as u64);
            }
            _ => return Err(EINVAL),
        };
        let np = base.checked_add(off).ok_or(EOVERFLOW)?;
        if np < 0 {
            return Err(EINVAL);
        }
        file.set_pos(np as u64);
        Ok(np as u64)
    }
    fn fsync(&self, file: &File) -> Result<()> {
        if let Some(i) = &file.inode {
            i.sync()?;
        }
        Ok(())
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

pub static REGULAR_OPS: RegularOps = RegularOps;

pub fn regular_ops() -> Arc<dyn FileOps> {
    static CELL: crate::sync::Once<Arc<dyn FileOps>> = crate::sync::Once::new();
    CELL.call_once(|| Arc::new(RegularOps)).clone()
}

/// Convenience: read a whole file into memory.
pub fn read_all(inode: &Arc<dyn Inode>) -> Result<alloc::vec::Vec<u8>> {
    let size = inode.size() as usize;
    let mut v = alloc::vec![0u8; size];
    let mut off = 0;
    while off < size {
        let n = inode.read_at(off as u64, &mut v[off..])?;
        if n == 0 {
            break;
        }
        off += n;
    }
    v.truncate(off);
    Ok(v)
}

/// Global inode number allocator for in-memory filesystems.
pub fn next_ino() -> u64 {
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}
pub fn next_fs_id() -> u64 {
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}
