//! File and directory system calls.

use super::SysResult;
use crate::fs::file::FdTable;
use crate::fs::path::{self, resolve_path, Resolved};
use crate::fs::vfs::{self, File, Inode, Kind, Stat, AT_EMPTY_PATH, AT_FDCWD, AT_REMOVEDIR, AT_SYMLINK_NOFOLLOW, O_ACCMODE, O_APPEND, O_CLOEXEC, O_CREAT, O_DIRECTORY, O_EXCL, O_NOFOLLOW, O_NONBLOCK, O_PATH, O_RDONLY, O_TRUNC, O_WRONLY};
use crate::mm::errno::*;
use crate::mm::user::{copy_from_user, copy_to_user, read_iovecs, read_path, read_user, write_user, UserBuf};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

pub fn files() -> Arc<FdTable> {
    crate::sched::current_ref().proc.files()
}

fn cur_file(fd: i32) -> Result<Arc<File>> {
    files().get(fd)
}

/// Base directory (and its absolute path) for *at() calls.
fn base_dir(dirfd: i32, path: &str) -> Result<(Arc<dyn Inode>, String)> {
    if path.starts_with('/') {
        return Ok((path::root(), String::from("/")));
    }
    if dirfd == AT_FDCWD {
        let (cwd, p) = crate::sched::current_ref().proc.cwd();
        return Ok((cwd, p));
    }
    let f = cur_file(dirfd)?;
    let inode = f.inode.clone().ok_or(ENOTDIR)?;
    if inode.kind() != Kind::Dir {
        return Err(ENOTDIR);
    }
    Ok((inode, f.path()))
}

/// Resolve a user path relative to dirfd. Returns the resolution and the absolute path string.
fn resolve_at(dirfd: i32, path: &str, follow: bool) -> Result<(Resolved, String)> {
    let (base, base_path) = base_dir(dirfd, path)?;
    let r = resolve_path(&base, path, follow)?;
    let abs = path::join(&base_path, path);
    Ok((r, abs))
}

fn resolve_at_empty(dirfd: i32, path: &str, flags: u32, follow: bool) -> Result<(Arc<dyn Inode>, String)> {
    if path.is_empty() && flags & AT_EMPTY_PATH != 0 {
        if dirfd == AT_FDCWD {
            let (cwd, p) = crate::sched::current_ref().proc.cwd();
            return Ok((cwd, p));
        }
        let f = cur_file(dirfd)?;
        let inode = f.inode.clone().ok_or(EBADF)?;
        return Ok((inode, f.path()));
    }
    let (r, abs) = resolve_at(dirfd, path, follow)?;
    Ok((r.inode.ok_or(ENOENT)?, abs))
}

pub fn open_file(dirfd: i32, path: &str, flags: u32, mode: u32) -> Result<Arc<File>> {
    let follow = flags & O_NOFOLLOW == 0;
    let (r, abs) = resolve_at(dirfd, path, follow)?;
    let inode = match r.inode {
        Some(i) => {
            if flags & O_CREAT != 0 && flags & O_EXCL != 0 {
                return Err(EEXIST);
            }
            if i.kind() == Kind::Symlink && !follow && flags & O_PATH == 0 {
                return Err(ELOOP);
            }
            i
        }
        None => {
            if flags & O_CREAT == 0 {
                return Err(ENOENT);
            }
            let umask = crate::sched::current_ref().proc.umask.load(Ordering::Relaxed);
            r.parent.create(&r.name, Kind::File, mode & !umask & 0o7777, 0)?
        }
    };
    let kind = inode.kind();
    if flags & O_DIRECTORY != 0 && kind != Kind::Dir {
        return Err(ENOTDIR);
    }
    let acc = flags & O_ACCMODE;
    if kind == Kind::Dir && (acc == O_WRONLY || acc == vfs::O_RDWR) {
        return Err(EISDIR);
    }
    if flags & O_PATH != 0 {
        let ops: Arc<dyn vfs::FileOps> = Arc::new(PathOps);
        return Ok(File::new(Some(inode), ops, flags & (O_PATH | O_CLOEXEC | O_DIRECTORY | O_NOFOLLOW), &abs));
    }
    if flags & O_TRUNC != 0 && kind == Kind::File && acc != O_RDONLY {
        inode.truncate(0)?;
    }
    let ops = match inode.open(flags)? {
        Some(o) => o,
        None => vfs::regular_ops(),
    };
    let f = File::new(Some(inode), ops, flags & !(O_CREAT | O_EXCL | O_TRUNC | O_CLOEXEC), &abs);
    Ok(f)
}

/// Placeholder ops for O_PATH descriptors.
struct PathOps;
impl vfs::FileOps for PathOps {
    fn read(&self, _f: &File, _b: &mut [u8]) -> Result<usize> {
        Err(EBADF)
    }
    fn write(&self, _f: &File, _b: &[u8]) -> Result<usize> {
        Err(EBADF)
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

pub fn openat(dirfd: i32, path: usize, flags: u32, mode: u32) -> SysResult {
    let p = read_path(path)?;
    let f = open_file(dirfd, &p, flags, mode)?;
    let fd = files().alloc(f, flags & O_CLOEXEC != 0, 0)?;
    Ok(fd as u64)
}

pub fn close(fd: i32) -> SysResult {
    let f = files().close(fd)?;
    drop(f);
    Ok(0)
}

pub fn close_range(lo: u32, hi: u32, flags: u32) -> SysResult {
    if flags & !0x6 != 0 {
        return Err(EINVAL);
    }
    let dropped = files().close_range(lo as usize, hi as usize);
    drop(dropped);
    Ok(0)
}

pub fn read(fd: i32, buf: usize, len: usize) -> SysResult {
    let f = cur_file(fd)?;
    let len = len.min(0x7fff_f000);
    let ub = UserBuf::new(buf, len);
    let s = ub.as_mut_slice()?;
    let n = f.read(s)?;
    Ok(n as u64)
}

pub fn write(fd: i32, buf: usize, len: usize) -> SysResult {
    let f = cur_file(fd)?;
    let len = len.min(0x7fff_f000);
    let ub = UserBuf::new(buf, len);
    let s = ub.as_slice()?;
    let n = f.write(s)?;
    Ok(n as u64)
}

pub fn pread(fd: i32, buf: usize, len: usize, off: u64) -> SysResult {
    let f = cur_file(fd)?;
    if !f.readable() {
        return Err(EBADF);
    }
    let ub = UserBuf::new(buf, len);
    let s = ub.as_mut_slice()?;
    Ok(f.ops.pread(&f, s, off)? as u64)
}

pub fn pwrite(fd: i32, buf: usize, len: usize, off: u64) -> SysResult {
    let f = cur_file(fd)?;
    if !f.writable() {
        return Err(EBADF);
    }
    let ub = UserBuf::new(buf, len);
    let s = ub.as_slice()?;
    Ok(f.ops.pwrite(&f, s, off)? as u64)
}

pub fn readv(fd: i32, iov: usize, cnt: usize) -> SysResult {
    let f = cur_file(fd)?;
    let iovs = read_iovecs(iov, cnt)?;
    let mut total = 0u64;
    for v in iovs {
        if v.len == 0 {
            continue;
        }
        let ub = UserBuf::new(v.base as usize, v.len as usize);
        let s = ub.as_mut_slice()?;
        let n = match f.read(s) {
            Ok(n) => n,
            Err(e) => {
                if total > 0 {
                    return Ok(total);
                }
                return Err(e);
            }
        };
        total += n as u64;
        if n < s.len() {
            break;
        }
    }
    Ok(total)
}

pub fn writev(fd: i32, iov: usize, cnt: usize) -> SysResult {
    let f = cur_file(fd)?;
    let iovs = read_iovecs(iov, cnt)?;
    let mut total = 0u64;
    for v in iovs {
        if v.len == 0 {
            continue;
        }
        let ub = UserBuf::new(v.base as usize, v.len as usize);
        let s = ub.as_slice()?;
        let n = match f.write(s) {
            Ok(n) => n,
            Err(e) => {
                if total > 0 {
                    return Ok(total);
                }
                return Err(e);
            }
        };
        total += n as u64;
        if n < s.len() {
            break;
        }
    }
    Ok(total)
}

pub fn preadv(fd: i32, iov: usize, cnt: usize, off: u64) -> SysResult {
    let f = cur_file(fd)?;
    let iovs = read_iovecs(iov, cnt)?;
    let mut total = 0u64;
    let mut o = off;
    for v in iovs {
        let ub = UserBuf::new(v.base as usize, v.len as usize);
        let s = ub.as_mut_slice()?;
        let n = f.ops.pread(&f, s, o)?;
        total += n as u64;
        o += n as u64;
        if n < s.len() {
            break;
        }
    }
    Ok(total)
}

pub fn pwritev(fd: i32, iov: usize, cnt: usize, off: u64) -> SysResult {
    let f = cur_file(fd)?;
    let iovs = read_iovecs(iov, cnt)?;
    let mut total = 0u64;
    let mut o = off;
    for v in iovs {
        let ub = UserBuf::new(v.base as usize, v.len as usize);
        let s = ub.as_slice()?;
        let n = f.ops.pwrite(&f, s, o)?;
        total += n as u64;
        o += n as u64;
        if n < s.len() {
            break;
        }
    }
    Ok(total)
}

pub fn lseek(fd: i32, off: i64, whence: u32) -> SysResult {
    let f = cur_file(fd)?;
    Ok(f.ops.lseek(&f, off, whence)?)
}

pub fn ioctl(fd: i32, cmd: u32, arg: usize) -> SysResult {
    let f = cur_file(fd)?;
    match cmd {
        0x5421 => {
            // FIONBIO works on every file
            let on: i32 = read_user(arg)?;
            let fl = f.flags();
            f.set_flags(if on != 0 { fl | O_NONBLOCK } else { fl & !O_NONBLOCK });
            Ok(0)
        }
        0x5451 | 0x5450 => {
            // FIOCLEX / FIONCLEX
            files().set_cloexec(fd, cmd == 0x5451)?;
            Ok(0)
        }
        _ => Ok(f.ops.ioctl(&f, cmd, arg)? as u64),
    }
}

fn write_stat(st: &Stat, addr: usize) -> Result<()> {
    let bytes = unsafe { core::slice::from_raw_parts(st as *const _ as *const u8, core::mem::size_of::<Stat>()) };
    copy_to_user(addr, bytes)
}

fn file_stat(f: &File) -> Stat {
    match &f.inode {
        Some(i) => i.stat(),
        None => {
            // pipes, sockets and other anonymous files
            let kind = f.ops.as_any().downcast_ref::<crate::fs::pipe::PipeReader>().map(|_| Kind::Fifo).or_else(|| f.ops.as_any().downcast_ref::<crate::fs::pipe::PipeWriter>().map(|_| Kind::Fifo)).unwrap_or(Kind::Socket);
            let mut s = Stat::simple(0, 0, kind, 0o600, 0, 1);
            s.st_blksize = 4096;
            s
        }
    }
}

pub fn fstat(fd: i32, buf: usize) -> SysResult {
    let f = cur_file(fd)?;
    write_stat(&file_stat(&f), buf)?;
    Ok(0)
}

pub fn stat_path(dirfd: i32, path: usize, buf: usize, flags: u32) -> SysResult {
    let p = read_path(path)?;
    let (inode, _) = resolve_at_empty(dirfd, &p, flags, flags & AT_SYMLINK_NOFOLLOW == 0)?;
    write_stat(&inode.stat(), buf)?;
    Ok(0)
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct StatxTimestamp {
    sec: i64,
    nsec: u32,
    pad: i32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Statx {
    mask: u32,
    blksize: u32,
    attributes: u64,
    nlink: u32,
    uid: u32,
    gid: u32,
    mode: u16,
    pad1: u16,
    ino: u64,
    size: u64,
    blocks: u64,
    attributes_mask: u64,
    atime: StatxTimestamp,
    btime: StatxTimestamp,
    ctime: StatxTimestamp,
    mtime: StatxTimestamp,
    rdev_major: u32,
    rdev_minor: u32,
    dev_major: u32,
    dev_minor: u32,
    mnt_id: u64,
    dio_mem_align: u32,
    dio_offset_align: u32,
    spare: [u64; 12],
}

pub fn statx(dirfd: i32, path: usize, flags: u32, _mask: u32, buf: usize) -> SysResult {
    let p = read_path(path)?;
    let st = if p.is_empty() && flags & AT_EMPTY_PATH != 0 {
        if dirfd == AT_FDCWD {
            crate::sched::current_ref().proc.cwd().0.stat()
        } else {
            file_stat(&cur_file(dirfd)?)
        }
    } else {
        let (inode, _) = resolve_at_empty(dirfd, &p, flags, flags & AT_SYMLINK_NOFOLLOW == 0)?;
        inode.stat()
    };
    let mut x = Statx::default();
    x.mask = 0x7ff; // STATX_BASIC_STATS
    x.blksize = st.st_blksize as u32;
    x.nlink = st.st_nlink as u32;
    x.uid = st.st_uid;
    x.gid = st.st_gid;
    x.mode = st.st_mode as u16;
    x.ino = st.st_ino;
    x.size = st.st_size as u64;
    x.blocks = st.st_blocks as u64;
    x.atime = StatxTimestamp { sec: st.st_atime, nsec: st.st_atime_nsec as u32, pad: 0 };
    x.mtime = StatxTimestamp { sec: st.st_mtime, nsec: st.st_mtime_nsec as u32, pad: 0 };
    x.ctime = StatxTimestamp { sec: st.st_ctime, nsec: st.st_ctime_nsec as u32, pad: 0 };
    x.btime = x.ctime;
    x.rdev_major = vfs::major(st.st_rdev);
    x.rdev_minor = vfs::minor(st.st_rdev);
    x.dev_major = vfs::major(st.st_dev);
    x.dev_minor = vfs::minor(st.st_dev);
    let bytes = unsafe { core::slice::from_raw_parts(&x as *const _ as *const u8, core::mem::size_of::<Statx>()) };
    copy_to_user(buf, bytes)?;
    Ok(0)
}

pub fn faccessat(dirfd: i32, path: usize, _mode: u32, flags: u32) -> SysResult {
    let p = read_path(path)?;
    let (inode, _) = resolve_at_empty(dirfd, &p, flags, flags & AT_SYMLINK_NOFOLLOW == 0)?;
    let _ = inode;
    Ok(0)
}

pub fn getdents64(fd: i32, buf: usize, len: usize) -> SysResult {
    let f = cur_file(fd)?;
    let inode = f.inode.clone().ok_or(ENOTDIR)?;
    if inode.kind() != Kind::Dir {
        return Err(ENOTDIR);
    }
    let mut out: Vec<u8> = Vec::with_capacity(len.min(65536));
    let mut pos = f.pos() as usize;
    loop {
        let e = match f.ops.readdir(&f, pos)? {
            Some(e) => e,
            None => break,
        };
        let name = e.name.as_bytes();
        let reclen = (19 + name.len() + 1 + 7) & !7;
        if out.len() + reclen > len {
            if out.is_empty() {
                return Err(EINVAL);
            }
            break;
        }
        out.extend_from_slice(&e.ino.to_le_bytes());
        out.extend_from_slice(&((pos + 1) as u64).to_le_bytes());
        out.extend_from_slice(&(reclen as u16).to_le_bytes());
        out.push(e.kind.dtype());
        out.extend_from_slice(name);
        out.push(0);
        while out.len() % 8 != 0 {
            out.push(0);
        }
        pos += 1;
    }
    copy_to_user(buf, &out)?;
    f.set_pos(pos as u64);
    Ok(out.len() as u64)
}

pub fn getcwd(buf: usize, len: usize) -> SysResult {
    let (_, p) = crate::sched::current_ref().proc.cwd();
    let bytes = p.as_bytes();
    if bytes.len() + 1 > len {
        return Err(ERANGE);
    }
    copy_to_user(buf, bytes)?;
    copy_to_user(buf + bytes.len(), &[0u8])?;
    Ok((bytes.len() + 1) as u64)
}

pub fn chdir(path: usize) -> SysResult {
    let p = read_path(path)?;
    let (r, abs) = resolve_at(AT_FDCWD, &p, true)?;
    let inode = r.inode.ok_or(ENOENT)?;
    if inode.kind() != Kind::Dir {
        return Err(ENOTDIR);
    }
    crate::sched::current_ref().proc.set_cwd(inode, abs);
    Ok(0)
}

pub fn fchdir(fd: i32) -> SysResult {
    let f = cur_file(fd)?;
    let inode = f.inode.clone().ok_or(ENOTDIR)?;
    if inode.kind() != Kind::Dir {
        return Err(ENOTDIR);
    }
    crate::sched::current_ref().proc.set_cwd(inode, f.path());
    Ok(0)
}

pub fn mkdirat(dirfd: i32, path: usize, mode: u32) -> SysResult {
    let p = read_path(path)?;
    let (r, _) = resolve_at(dirfd, &p, true)?;
    if r.inode.is_some() {
        return Err(EEXIST);
    }
    let umask = crate::sched::current_ref().proc.umask.load(Ordering::Relaxed);
    r.parent.create(&r.name, Kind::Dir, mode & !umask & 0o7777, 0)?;
    Ok(0)
}

pub fn mknodat(dirfd: i32, path: usize, mode: u32, dev: u64) -> SysResult {
    let p = read_path(path)?;
    let (r, _) = resolve_at(dirfd, &p, true)?;
    if r.inode.is_some() {
        return Err(EEXIST);
    }
    let kind = match mode & vfs::S_IFMT {
        0 | vfs::S_IFREG => Kind::File,
        vfs::S_IFCHR => Kind::CharDev,
        vfs::S_IFBLK => Kind::BlockDev,
        vfs::S_IFIFO => Kind::Fifo,
        vfs::S_IFSOCK => Kind::Socket,
        _ => return Err(EINVAL),
    };
    let umask = crate::sched::current_ref().proc.umask.load(Ordering::Relaxed);
    r.parent.create(&r.name, kind, mode & !umask & 0o7777, dev)?;
    Ok(0)
}

pub fn unlinkat(dirfd: i32, path: usize, flags: u32) -> SysResult {
    let p = read_path(path)?;
    let (r, _) = resolve_at(dirfd, &p, false)?;
    let inode = r.inode.ok_or(ENOENT)?;
    if flags & AT_REMOVEDIR != 0 {
        if inode.kind() != Kind::Dir {
            return Err(ENOTDIR);
        }
        r.parent.rmdir(&r.name)?;
    } else {
        if inode.kind() == Kind::Dir {
            return Err(EISDIR);
        }
        r.parent.unlink(&r.name)?;
    }
    Ok(0)
}

pub fn renameat(olddirfd: i32, oldpath: usize, newdirfd: i32, newpath: usize) -> SysResult {
    let op = read_path(oldpath)?;
    let np = read_path(newpath)?;
    let (ro, _) = resolve_at(olddirfd, &op, false)?;
    ro.inode.as_ref().ok_or(ENOENT)?;
    let (rn, _) = resolve_at(newdirfd, &np, false)?;
    if rn.name.is_empty() {
        return Err(EINVAL);
    }
    ro.parent.rename(&ro.name, &rn.parent, &rn.name)?;
    Ok(0)
}

pub fn linkat(olddirfd: i32, oldpath: usize, newdirfd: i32, newpath: usize, flags: u32) -> SysResult {
    let op = read_path(oldpath)?;
    let np = read_path(newpath)?;
    let (target, _) = resolve_at_empty(olddirfd, &op, flags, flags & vfs::AT_SYMLINK_FOLLOW != 0)?;
    let (rn, _) = resolve_at(newdirfd, &np, false)?;
    if rn.inode.is_some() {
        return Err(EEXIST);
    }
    rn.parent.link(&rn.name, &target)?;
    Ok(0)
}

pub fn symlinkat(target: usize, newdirfd: i32, linkpath: usize) -> SysResult {
    let t = read_path(target)?;
    let lp = read_path(linkpath)?;
    let (r, _) = resolve_at(newdirfd, &lp, false)?;
    if r.inode.is_some() {
        return Err(EEXIST);
    }
    r.parent.symlink(&r.name, &t)?;
    Ok(0)
}

pub fn readlinkat(dirfd: i32, path: usize, buf: usize, len: usize) -> SysResult {
    let p = read_path(path)?;
    let (r, _) = resolve_at(dirfd, &p, false)?;
    let inode = r.inode.ok_or(ENOENT)?;
    if inode.kind() != Kind::Symlink {
        return Err(EINVAL);
    }
    let target = inode.readlink()?;
    let b = target.as_bytes();
    let n = b.len().min(len);
    copy_to_user(buf, &b[..n])?;
    Ok(n as u64)
}

pub fn fchmodat(dirfd: i32, path: usize, mode: u32) -> SysResult {
    let p = read_path(path)?;
    let (inode, _) = resolve_at_empty(dirfd, &p, 0, true)?;
    inode.set_mode(mode)?;
    Ok(0)
}

pub fn fchmod(fd: i32, mode: u32) -> SysResult {
    let f = cur_file(fd)?;
    if let Some(i) = &f.inode {
        i.set_mode(mode)?;
    }
    Ok(0)
}

pub fn truncate(path: usize, len: u64) -> SysResult {
    let p = read_path(path)?;
    let (inode, _) = resolve_at_empty(AT_FDCWD, &p, 0, true)?;
    if inode.kind() == Kind::Dir {
        return Err(EISDIR);
    }
    inode.truncate(len)?;
    Ok(0)
}

pub fn ftruncate(fd: i32, len: u64) -> SysResult {
    let f = cur_file(fd)?;
    if !f.writable() {
        return Err(EINVAL);
    }
    let inode = f.inode.clone().ok_or(EINVAL)?;
    inode.truncate(len)?;
    Ok(0)
}

pub fn fallocate(fd: i32, _mode: u32, off: u64, len: u64) -> SysResult {
    let f = cur_file(fd)?;
    let inode = f.inode.clone().ok_or(EBADF)?;
    if inode.size() < off + len {
        inode.truncate(off + len)?;
    }
    Ok(0)
}

pub fn fsync(fd: i32) -> SysResult {
    let f = cur_file(fd)?;
    f.ops.fsync(&f)?;
    Ok(0)
}

pub fn dup(fd: i32) -> SysResult {
    let f = cur_file(fd)?;
    Ok(files().alloc(f, false, 0)? as u64)
}

pub fn dup3(oldfd: i32, newfd: i32, flags: u32, is_dup2: bool) -> SysResult {
    let f = cur_file(oldfd)?;
    if oldfd == newfd {
        if is_dup2 {
            return Ok(newfd as u64);
        }
        return Err(EINVAL);
    }
    let old = files().set(newfd, f, flags & O_CLOEXEC != 0)?;
    drop(old);
    Ok(newfd as u64)
}

pub fn fcntl(fd: i32, cmd: u32, arg: u64) -> SysResult {
    const F_DUPFD: u32 = 0;
    const F_GETFD: u32 = 1;
    const F_SETFD: u32 = 2;
    const F_GETFL: u32 = 3;
    const F_SETFL: u32 = 4;
    const F_GETLK: u32 = 5;
    const F_SETLK: u32 = 6;
    const F_SETLKW: u32 = 7;
    const F_SETOWN: u32 = 8;
    const F_GETOWN: u32 = 9;
    const F_DUPFD_CLOEXEC: u32 = 1030;
    const F_GETPIPE_SZ: u32 = 1032;
    const F_SETPIPE_SZ: u32 = 1031;
    const F_ADD_SEALS: u32 = 1033;
    const F_GET_SEALS: u32 = 1034;
    let table = files();
    let entry = table.get_entry(fd)?;
    match cmd {
        F_DUPFD => Ok(table.alloc(entry.file, false, arg as usize)? as u64),
        F_DUPFD_CLOEXEC => Ok(table.alloc(entry.file, true, arg as usize)? as u64),
        F_GETFD => Ok(if entry.cloexec { 1 } else { 0 }),
        F_SETFD => {
            table.set_cloexec(fd, arg & 1 != 0)?;
            Ok(0)
        }
        F_GETFL => Ok(entry.file.flags() as u64),
        F_SETFL => {
            let keep = entry.file.flags() & (O_ACCMODE | O_PATH);
            let new = (arg as u32) & (O_APPEND | O_NONBLOCK | vfs::O_DSYNC | 0o40000 | 0o1000000);
            entry.file.set_flags(keep | new);
            Ok(0)
        }
        F_GETLK => {
            // report unlocked: set l_type = F_UNLCK (2) at offset 0
            write_user::<u16>(arg as usize, 2)?;
            Ok(0)
        }
        F_SETLK | F_SETLKW => Ok(0),
        F_SETOWN => Ok(0),
        F_GETOWN => Ok(0),
        F_GETPIPE_SZ => Ok(crate::fs::pipe::PIPE_BUF_SIZE as u64),
        F_SETPIPE_SZ => Ok(crate::fs::pipe::PIPE_BUF_SIZE as u64),
        F_ADD_SEALS => Ok(0),
        F_GET_SEALS => Ok(0),
        _ => Err(EINVAL),
    }
}

pub fn pipe2(fds: usize, flags: u32) -> SysResult {
    if flags & !(O_CLOEXEC | O_NONBLOCK | 0o40000) != 0 {
        return Err(EINVAL);
    }
    let (r, w) = crate::fs::pipe::create(flags);
    let table = files();
    let cloexec = flags & O_CLOEXEC != 0;
    let rfd = table.alloc(r, cloexec, 0)?;
    let wfd = match table.alloc(w, cloexec, 0) {
        Ok(fd) => fd,
        Err(e) => {
            let _ = table.close(rfd);
            return Err(e);
        }
    };
    write_user::<[i32; 2]>(fds, [rfd, wfd])?;
    Ok(0)
}

pub fn sendfile(out_fd: i32, in_fd: i32, off_ptr: usize, count: usize) -> SysResult {
    let inf = cur_file(in_fd)?;
    let outf = cur_file(out_fd)?;
    let mut off = if off_ptr != 0 { Some(read_user::<u64>(off_ptr)?) } else { None };
    let mut buf = alloc::vec![0u8; 65536];
    let mut total = 0usize;
    while total < count {
        let chunk = (count - total).min(buf.len());
        let n = match off {
            Some(o) => inf.ops.pread(&inf, &mut buf[..chunk], o)?,
            None => inf.read(&mut buf[..chunk])?,
        };
        if n == 0 {
            break;
        }
        let mut w = 0;
        while w < n {
            let m = outf.write(&buf[w..n])?;
            if m == 0 {
                break;
            }
            w += m;
        }
        if let Some(o) = off.as_mut() {
            *o += n as u64;
        }
        total += n;
        if n < chunk {
            break;
        }
    }
    if let Some(o) = off {
        write_user::<u64>(off_ptr, o)?;
    }
    Ok(total as u64)
}

pub fn memfd_create(name: usize, flags: u32) -> SysResult {
    let _ = read_path(name)?;
    let inode = crate::fs::tmpfs::anonymous_file("memfd");
    let f = File::new(Some(inode), vfs::regular_ops(), vfs::O_RDWR, "/memfd:");
    Ok(files().alloc(f, flags & 1 != 0, 0)? as u64)
}

#[repr(C)]
#[derive(Default)]
struct StatFs {
    f_type: i64,
    f_bsize: i64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_fsid: [i32; 2],
    f_namelen: i64,
    f_frsize: i64,
    f_flags: i64,
    f_spare: [i64; 4],
}

fn statfs_for(inode: &Arc<dyn Inode>, buf: usize) -> Result<()> {
    let (total, free) = crate::mm::pmm::stats();
    let mut s = StatFs::default();
    s.f_type = match inode.fs_name() {
        "proc" => 0x9fa0,
        "tmpfs" | "devtmpfs" => 0x01021994,
        "ext2" => 0xEF53,
        _ => 0x858458f6, // ramfs
    };
    s.f_bsize = 4096;
    s.f_frsize = 4096;
    s.f_blocks = total as u64;
    s.f_bfree = free as u64;
    s.f_bavail = free as u64;
    s.f_files = 1_000_000;
    s.f_ffree = 900_000;
    s.f_namelen = 255;
    let bytes = unsafe { core::slice::from_raw_parts(&s as *const _ as *const u8, core::mem::size_of::<StatFs>()) };
    copy_to_user(buf, bytes)
}

pub fn statfs(path: usize, buf: usize) -> SysResult {
    let p = read_path(path)?;
    let (inode, _) = resolve_at_empty(AT_FDCWD, &p, 0, true)?;
    statfs_for(&inode, buf)?;
    Ok(0)
}

pub fn fstatfs(fd: i32, buf: usize) -> SysResult {
    let f = cur_file(fd)?;
    let inode = f.inode.clone().ok_or(EBADF)?;
    statfs_for(&inode, buf)?;
    Ok(0)
}

pub fn mount(source: usize, target: usize, fstype: usize, _flags: u64, _data: usize) -> SysResult {
    let src = if source != 0 { read_path(source).unwrap_or_default() } else { String::new() };
    let tgt = read_path(target)?;
    let ty = if fstype != 0 { read_path(fstype)? } else { String::new() };
    match ty.as_str() {
        "tmpfs" | "ramfs" => path::mount(&tgt, crate::fs::tmpfs::new_fs("tmpfs"))?,
        "proc" => path::mount(&tgt, crate::fs::procfs::new_fs())?,
        "devtmpfs" => path::mount(&tgt, crate::fs::lookup_abs("/dev")?)?,
        _ => {
            klog!("fs", "mount {} on {} type {}: unsupported", src, tgt, ty);
            return Err(ENODEV);
        }
    }
    Ok(0)
}

pub fn umount(target: usize) -> SysResult {
    let tgt = read_path(target)?;
    path::umount(&tgt)?;
    Ok(0)
}

/// Read the whole content of a path (used by exec and the kernel).
pub fn read_file(p: &str) -> Result<Vec<u8>> {
    let inode = crate::fs::lookup_abs(p)?;
    vfs::read_all(&inode)
}

pub fn write_bytes_to_fd(fd: i32, data: &[u8]) -> Result<usize> {
    let f = cur_file(fd)?;
    f.write(data)
}

pub fn copy_user_path(path: usize) -> Result<String> {
    read_path(path)
}

pub fn _unused() {
    let _ = copy_from_user;
}
