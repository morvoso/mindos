//! Linux x86_64 system call dispatch.

pub mod fs;
pub mod futex;
pub mod misc;
pub mod mm;
pub mod poll;
pub mod proc;
pub mod signal;
pub mod time;

use crate::arch::x86_64::interrupts::TrapFrame;
use crate::mm::errno::*;
use core::sync::atomic::{AtomicBool, Ordering};

pub static TRACE: AtomicBool = AtomicBool::new(false);

pub type SysResult = Result<u64>;

macro_rules! nr {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u64 = $v;)* };
}
nr! {
    SYS_READ = 0, SYS_WRITE = 1, SYS_OPEN = 2, SYS_CLOSE = 3, SYS_STAT = 4, SYS_FSTAT = 5, SYS_LSTAT = 6,
    SYS_POLL = 7, SYS_LSEEK = 8, SYS_MMAP = 9, SYS_MPROTECT = 10, SYS_MUNMAP = 11, SYS_BRK = 12,
    SYS_RT_SIGACTION = 13, SYS_RT_SIGPROCMASK = 14, SYS_RT_SIGRETURN = 15, SYS_IOCTL = 16,
    SYS_PREAD64 = 17, SYS_PWRITE64 = 18, SYS_READV = 19, SYS_WRITEV = 20, SYS_ACCESS = 21, SYS_PIPE = 22,
    SYS_SELECT = 23, SYS_SCHED_YIELD = 24, SYS_MREMAP = 25, SYS_MSYNC = 26, SYS_MINCORE = 27,
    SYS_MADVISE = 28, SYS_DUP = 32, SYS_DUP2 = 33, SYS_PAUSE = 34, SYS_NANOSLEEP = 35, SYS_GETITIMER = 36,
    SYS_ALARM = 37, SYS_SETITIMER = 38, SYS_GETPID = 39, SYS_SENDFILE = 40, SYS_SOCKET = 41, SYS_CONNECT = 42,
    SYS_ACCEPT = 43, SYS_SENDTO = 44, SYS_RECVFROM = 45, SYS_SENDMSG = 46, SYS_RECVMSG = 47, SYS_SHUTDOWN = 48,
    SYS_BIND = 49, SYS_LISTEN = 50, SYS_GETSOCKNAME = 51, SYS_GETPEERNAME = 52, SYS_SOCKETPAIR = 53,
    SYS_SETSOCKOPT = 54, SYS_GETSOCKOPT = 55, SYS_CLONE = 56, SYS_FORK = 57, SYS_VFORK = 58, SYS_EXECVE = 59,
    SYS_EXIT = 60, SYS_WAIT4 = 61, SYS_KILL = 62, SYS_UNAME = 63, SYS_FCNTL = 72, SYS_FLOCK = 73, SYS_FSYNC = 74,
    SYS_FDATASYNC = 75, SYS_TRUNCATE = 76, SYS_FTRUNCATE = 77, SYS_GETDENTS = 78, SYS_GETCWD = 79, SYS_CHDIR = 80,
    SYS_FCHDIR = 81, SYS_RENAME = 82, SYS_MKDIR = 83, SYS_RMDIR = 84, SYS_CREAT = 85, SYS_LINK = 86, SYS_UNLINK = 87,
    SYS_SYMLINK = 88, SYS_READLINK = 89, SYS_CHMOD = 90, SYS_FCHMOD = 91, SYS_CHOWN = 92, SYS_FCHOWN = 93,
    SYS_LCHOWN = 94, SYS_UMASK = 95, SYS_GETTIMEOFDAY = 96, SYS_GETRLIMIT = 97, SYS_GETRUSAGE = 98, SYS_SYSINFO = 99,
    SYS_TIMES = 100, SYS_GETUID = 102, SYS_SYSLOG = 103, SYS_GETGID = 104, SYS_SETUID = 105, SYS_SETGID = 106,
    SYS_GETEUID = 107, SYS_GETEGID = 108, SYS_SETPGID = 109, SYS_GETPPID = 110, SYS_GETPGRP = 111, SYS_SETSID = 112,
    SYS_SETREUID = 113, SYS_SETREGID = 114, SYS_GETGROUPS = 115, SYS_SETGROUPS = 116, SYS_SETRESUID = 117,
    SYS_GETRESUID = 118, SYS_SETRESGID = 119, SYS_GETRESGID = 120, SYS_GETPGID = 121, SYS_SETFSUID = 122,
    SYS_SETFSGID = 123, SYS_GETSID = 124, SYS_CAPGET = 125, SYS_CAPSET = 126, SYS_RT_SIGPENDING = 127,
    SYS_RT_SIGTIMEDWAIT = 128, SYS_RT_SIGQUEUEINFO = 129, SYS_RT_SIGSUSPEND = 130, SYS_SIGALTSTACK = 131,
    SYS_UTIME = 132, SYS_MKNOD = 133, SYS_PERSONALITY = 135, SYS_STATFS = 137, SYS_FSTATFS = 138,
    SYS_GETPRIORITY = 140, SYS_SETPRIORITY = 141, SYS_SCHED_SETPARAM = 142, SYS_SCHED_GETPARAM = 143,
    SYS_SCHED_SETSCHEDULER = 144, SYS_SCHED_GETSCHEDULER = 145, SYS_SCHED_GET_PRIORITY_MAX = 146,
    SYS_SCHED_GET_PRIORITY_MIN = 147, SYS_SCHED_RR_GET_INTERVAL = 148, SYS_MLOCK = 149, SYS_MUNLOCK = 150,
    SYS_MLOCKALL = 151, SYS_MUNLOCKALL = 152, SYS_PRCTL = 157, SYS_ARCH_PRCTL = 158, SYS_SETRLIMIT = 160,
    SYS_CHROOT = 161, SYS_SYNC = 162, SYS_MOUNT = 165, SYS_UMOUNT2 = 166, SYS_REBOOT = 169, SYS_SETHOSTNAME = 170,
    SYS_SETDOMAINNAME = 171, SYS_GETTID = 186, SYS_TKILL = 200, SYS_TIME = 201, SYS_FUTEX = 202,
    SYS_SCHED_SETAFFINITY = 203, SYS_SCHED_GETAFFINITY = 204, SYS_EPOLL_CREATE = 213, SYS_GETDENTS64 = 217,
    SYS_SET_TID_ADDRESS = 218, SYS_FADVISE64 = 221, SYS_TIMER_CREATE = 222, SYS_CLOCK_SETTIME = 227,
    SYS_CLOCK_GETTIME = 228, SYS_CLOCK_GETRES = 229, SYS_CLOCK_NANOSLEEP = 230, SYS_EXIT_GROUP = 231,
    SYS_EPOLL_WAIT = 232, SYS_EPOLL_CTL = 233, SYS_TGKILL = 234, SYS_UTIMES = 235, SYS_WAITID = 247,
    SYS_IOPRIO_SET = 251, SYS_IOPRIO_GET = 252, SYS_INOTIFY_INIT = 253, SYS_INOTIFY_ADD_WATCH = 254,
    SYS_OPENAT = 257, SYS_MKDIRAT = 258, SYS_MKNODAT = 259, SYS_FCHOWNAT = 260, SYS_FUTIMESAT = 261,
    SYS_NEWFSTATAT = 262, SYS_UNLINKAT = 263, SYS_RENAMEAT = 264, SYS_LINKAT = 265, SYS_SYMLINKAT = 266,
    SYS_READLINKAT = 267, SYS_FCHMODAT = 268, SYS_FACCESSAT = 269, SYS_PSELECT6 = 270, SYS_PPOLL = 271,
    SYS_UNSHARE = 272, SYS_SET_ROBUST_LIST = 273, SYS_GET_ROBUST_LIST = 274, SYS_SPLICE = 275, SYS_TEE = 276,
    SYS_SYNC_FILE_RANGE = 277, SYS_UTIMENSAT = 280, SYS_EPOLL_PWAIT = 281, SYS_SIGNALFD = 282,
    SYS_TIMERFD_CREATE = 283, SYS_EVENTFD = 284, SYS_FALLOCATE = 285, SYS_TIMERFD_SETTIME = 286,
    SYS_TIMERFD_GETTIME = 287, SYS_ACCEPT4 = 288, SYS_SIGNALFD4 = 289, SYS_EVENTFD2 = 290, SYS_EPOLL_CREATE1 = 291,
    SYS_DUP3 = 292, SYS_PIPE2 = 293, SYS_INOTIFY_INIT1 = 294, SYS_PREADV = 295, SYS_PWRITEV = 296,
    SYS_RT_TGSIGQUEUEINFO = 297, SYS_RECVMMSG = 299, SYS_PRLIMIT64 = 302, SYS_SENDMMSG = 307, SYS_GETCPU = 309,
    SYS_SCHED_SETATTR = 314, SYS_SCHED_GETATTR = 315, SYS_RENAMEAT2 = 316, SYS_GETRANDOM = 318,
    SYS_MEMFD_CREATE = 319, SYS_EXECVEAT = 322, SYS_MEMBARRIER = 324, SYS_MLOCK2 = 325, SYS_COPY_FILE_RANGE = 326,
    SYS_PREADV2 = 327, SYS_PWRITEV2 = 328, SYS_STATX = 332, SYS_RSEQ = 334, SYS_PIDFD_SEND_SIGNAL = 424,
    SYS_PIDFD_OPEN = 434, SYS_CLONE3 = 435, SYS_CLOSE_RANGE = 436, SYS_OPENAT2 = 437, SYS_PIDFD_GETFD = 438,
    SYS_FACCESSAT2 = 439, SYS_EPOLL_PWAIT2 = 441, SYS_FUTEX_WAITV = 449,
}

pub fn name(nr: u64) -> &'static str {
    match nr {
        0 => "read", 1 => "write", 2 => "open", 3 => "close", 4 => "stat", 5 => "fstat", 6 => "lstat", 7 => "poll",
        8 => "lseek", 9 => "mmap", 10 => "mprotect", 11 => "munmap", 12 => "brk", 13 => "rt_sigaction",
        14 => "rt_sigprocmask", 15 => "rt_sigreturn", 16 => "ioctl", 17 => "pread64", 18 => "pwrite64", 19 => "readv",
        20 => "writev", 21 => "access", 22 => "pipe", 23 => "select", 24 => "sched_yield", 25 => "mremap",
        28 => "madvise", 32 => "dup", 33 => "dup2", 34 => "pause", 35 => "nanosleep", 39 => "getpid", 41 => "socket",
        42 => "connect", 43 => "accept", 44 => "sendto", 45 => "recvfrom", 46 => "sendmsg", 47 => "recvmsg",
        49 => "bind", 50 => "listen", 53 => "socketpair", 56 => "clone", 57 => "fork", 58 => "vfork", 59 => "execve",
        60 => "exit", 61 => "wait4", 62 => "kill", 63 => "uname", 72 => "fcntl", 78 => "getdents", 79 => "getcwd",
        80 => "chdir", 83 => "mkdir", 87 => "unlink", 89 => "readlink", 96 => "gettimeofday", 97 => "getrlimit",
        102 => "getuid", 104 => "getgid", 107 => "geteuid", 108 => "getegid", 109 => "setpgid", 110 => "getppid",
        111 => "getpgrp", 112 => "setsid", 131 => "sigaltstack", 157 => "prctl", 158 => "arch_prctl", 186 => "gettid",
        202 => "futex", 217 => "getdents64", 218 => "set_tid_address", 228 => "clock_gettime", 230 => "clock_nanosleep",
        231 => "exit_group", 232 => "epoll_wait", 233 => "epoll_ctl", 234 => "tgkill", 257 => "openat",
        262 => "newfstatat", 263 => "unlinkat", 270 => "pselect6", 271 => "ppoll", 273 => "set_robust_list",
        290 => "eventfd2", 291 => "epoll_create1", 293 => "pipe2", 302 => "prlimit64", 318 => "getrandom",
        319 => "memfd_create", 332 => "statx", 334 => "rseq", 435 => "clone3", 436 => "close_range",
        _ => "?",
    }
}

#[no_mangle]
pub extern "C" fn syscall_dispatch(frame: &mut TrapFrame) {
    let nr = frame.rax;
    let (a1, a2, a3, a4, a5, a6) = (frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8, frame.r9);
    let t = crate::sched::current_ref();
    t.syscall_nr.store(nr, Ordering::Relaxed);
    crate::arch::x86_64::enable_interrupts();
    let r = dispatch(frame, nr, [a1, a2, a3, a4, a5, a6]);
    let ret = match r {
        Ok(v) => v,
        Err(e) => e.as_ret(),
    };
    if TRACE.load(Ordering::Relaxed) {
        let t = crate::sched::current_ref();
        klog!("sys", "[{}] {}({:#x}, {:#x}, {:#x}) = {}", t.proc.pid, name(nr), a1, a2, a3, ret as i64);
    }
    frame.rax = ret;
}

fn dispatch(frame: &mut TrapFrame, nr: u64, a: [u64; 6]) -> SysResult {
    let u = |v: u64| v as usize;
    let i = |v: u64| v as i32;
    match nr {
        SYS_READ => fs::read(i(a[0]), u(a[1]), u(a[2])),
        SYS_WRITE => fs::write(i(a[0]), u(a[1]), u(a[2])),
        SYS_OPEN => fs::openat(crate::fs::vfs::AT_FDCWD, u(a[0]), a[1] as u32, a[2] as u32),
        SYS_CLOSE => fs::close(i(a[0])),
        SYS_STAT => fs::stat_path(crate::fs::vfs::AT_FDCWD, u(a[0]), u(a[1]), 0),
        SYS_FSTAT => fs::fstat(i(a[0]), u(a[1])),
        SYS_LSTAT => fs::stat_path(crate::fs::vfs::AT_FDCWD, u(a[0]), u(a[1]), crate::fs::vfs::AT_SYMLINK_NOFOLLOW),
        SYS_POLL => poll::poll(u(a[0]), u(a[1]), a[2] as i32 as i64),
        SYS_LSEEK => fs::lseek(i(a[0]), a[1] as i64, a[2] as u32),
        SYS_MMAP => mm::mmap(u(a[0]), u(a[1]), a[2] as u32, a[3] as u32, i(a[4]), a[5]),
        SYS_MPROTECT => mm::mprotect(u(a[0]), u(a[1]), a[2] as u32),
        SYS_MUNMAP => mm::munmap(u(a[0]), u(a[1])),
        SYS_BRK => mm::brk(u(a[0])),
        SYS_RT_SIGACTION => signal::rt_sigaction(i(a[0]), u(a[1]), u(a[2]), u(a[3])),
        SYS_RT_SIGPROCMASK => signal::rt_sigprocmask(i(a[0]), u(a[1]), u(a[2]), u(a[3])),
        SYS_RT_SIGRETURN => crate::proc::signal::sigreturn(frame),
        SYS_IOCTL => fs::ioctl(i(a[0]), a[1] as u32, u(a[2])),
        SYS_PREAD64 => fs::pread(i(a[0]), u(a[1]), u(a[2]), a[3]),
        SYS_PWRITE64 => fs::pwrite(i(a[0]), u(a[1]), u(a[2]), a[3]),
        SYS_READV => fs::readv(i(a[0]), u(a[1]), u(a[2])),
        SYS_WRITEV => fs::writev(i(a[0]), u(a[1]), u(a[2])),
        SYS_ACCESS => fs::faccessat(crate::fs::vfs::AT_FDCWD, u(a[0]), a[1] as u32, 0),
        SYS_PIPE => fs::pipe2(u(a[0]), 0),
        SYS_SELECT => poll::select(i(a[0]), u(a[1]), u(a[2]), u(a[3]), u(a[4]), false),
        SYS_SCHED_YIELD => {
            crate::sched::yield_now();
            Ok(0)
        }
        SYS_MREMAP => mm::mremap(u(a[0]), u(a[1]), u(a[2]), a[3] as u32, u(a[4])),
        SYS_MSYNC => Ok(0),
        SYS_MINCORE => Err(ENOSYS),
        SYS_MADVISE => Ok(0),
        SYS_DUP => fs::dup(i(a[0])),
        SYS_DUP2 => fs::dup3(i(a[0]), i(a[1]), 0, true),
        SYS_PAUSE => signal::pause(),
        SYS_NANOSLEEP => time::nanosleep(u(a[0]), u(a[1])),
        SYS_GETITIMER => time::getitimer(i(a[0]), u(a[1])),
        SYS_ALARM => time::alarm(a[0] as u32),
        SYS_SETITIMER => time::setitimer(i(a[0]), u(a[1]), u(a[2])),
        SYS_GETPID => Ok(crate::sched::current_ref().proc.pid as u64),
        SYS_SENDFILE => fs::sendfile(i(a[0]), i(a[1]), u(a[2]), u(a[3])),
        SYS_SOCKET | SYS_CONNECT | SYS_ACCEPT | SYS_SENDTO | SYS_RECVFROM | SYS_SENDMSG | SYS_RECVMSG | SYS_SHUTDOWN | SYS_BIND | SYS_LISTEN | SYS_GETSOCKNAME | SYS_GETPEERNAME | SYS_SOCKETPAIR | SYS_SETSOCKOPT | SYS_GETSOCKOPT | SYS_ACCEPT4 | SYS_RECVMMSG | SYS_SENDMMSG => {
            crate::net::syscall(nr, a)
        }
        SYS_CLONE => proc::clone(frame, a[0], a[1], u(a[2]), u(a[3]), a[4]),
        SYS_FORK => proc::fork(frame, false),
        SYS_VFORK => proc::fork(frame, true),
        SYS_EXECVE => proc::execve(frame, u(a[0]), u(a[1]), u(a[2])),
        SYS_EXIT => proc::exit(i(a[0])),
        SYS_WAIT4 => proc::wait4(a[0] as i64, u(a[1]), a[2] as u32, u(a[3])),
        SYS_KILL => signal::kill(a[0] as i64, i(a[1])),
        SYS_UNAME => misc::uname(u(a[0])),
        SYS_FCNTL => fs::fcntl(i(a[0]), a[1] as u32, a[2]),
        SYS_FLOCK => Ok(0),
        SYS_FSYNC | SYS_FDATASYNC => fs::fsync(i(a[0])),
        SYS_TRUNCATE => fs::truncate(u(a[0]), a[1]),
        SYS_FTRUNCATE => fs::ftruncate(i(a[0]), a[1]),
        SYS_GETDENTS => Err(ENOSYS),
        SYS_GETCWD => fs::getcwd(u(a[0]), u(a[1])),
        SYS_CHDIR => fs::chdir(u(a[0])),
        SYS_FCHDIR => fs::fchdir(i(a[0])),
        SYS_RENAME => fs::renameat(crate::fs::vfs::AT_FDCWD, u(a[0]), crate::fs::vfs::AT_FDCWD, u(a[1])),
        SYS_MKDIR => fs::mkdirat(crate::fs::vfs::AT_FDCWD, u(a[0]), a[1] as u32),
        SYS_RMDIR => fs::unlinkat(crate::fs::vfs::AT_FDCWD, u(a[0]), crate::fs::vfs::AT_REMOVEDIR),
        SYS_CREAT => fs::openat(crate::fs::vfs::AT_FDCWD, u(a[0]), crate::fs::vfs::O_CREAT | crate::fs::vfs::O_WRONLY | crate::fs::vfs::O_TRUNC, a[1] as u32),
        SYS_LINK => fs::linkat(crate::fs::vfs::AT_FDCWD, u(a[0]), crate::fs::vfs::AT_FDCWD, u(a[1]), 0),
        SYS_UNLINK => fs::unlinkat(crate::fs::vfs::AT_FDCWD, u(a[0]), 0),
        SYS_SYMLINK => fs::symlinkat(u(a[0]), crate::fs::vfs::AT_FDCWD, u(a[1])),
        SYS_READLINK => fs::readlinkat(crate::fs::vfs::AT_FDCWD, u(a[0]), u(a[1]), u(a[2])),
        SYS_CHMOD => fs::fchmodat(crate::fs::vfs::AT_FDCWD, u(a[0]), a[1] as u32),
        SYS_FCHMOD => fs::fchmod(i(a[0]), a[1] as u32),
        SYS_CHOWN | SYS_LCHOWN | SYS_FCHOWN | SYS_FCHOWNAT => Ok(0),
        SYS_UMASK => {
            let p = &crate::sched::current_ref().proc;
            let old = p.umask.swap(a[0] as u32 & 0o777, Ordering::Relaxed);
            Ok(old as u64)
        }
        SYS_GETTIMEOFDAY => time::gettimeofday(u(a[0]), u(a[1])),
        SYS_GETRLIMIT => misc::getrlimit(u(a[0]), u(a[1])),
        SYS_GETRUSAGE => misc::getrusage(i(a[0]), u(a[1])),
        SYS_SYSINFO => misc::sysinfo(u(a[0])),
        SYS_TIMES => time::times(u(a[0])),
        SYS_GETUID | SYS_GETEUID => Ok(crate::sched::current_ref().proc.uid.load(Ordering::Relaxed) as u64),
        SYS_GETGID | SYS_GETEGID => Ok(crate::sched::current_ref().proc.gid.load(Ordering::Relaxed) as u64),
        SYS_SYSLOG => misc::syslog(i(a[0]), u(a[1]), u(a[2])),
        SYS_SETUID | SYS_SETGID | SYS_SETREUID | SYS_SETREGID | SYS_SETRESUID | SYS_SETRESGID | SYS_SETFSUID | SYS_SETFSGID | SYS_SETGROUPS => Ok(0),
        SYS_GETGROUPS => Ok(0),
        SYS_GETRESUID | SYS_GETRESGID => {
            for p in [a[0], a[1], a[2]] {
                crate::mm::user::write_user::<u32>(u(p), 0)?;
            }
            Ok(0)
        }
        SYS_SETPGID => proc::setpgid(a[0] as u32, a[1] as u32),
        SYS_GETPPID => Ok(crate::sched::current_ref().proc.ppid() as u64),
        SYS_GETPGRP => Ok(crate::sched::current_ref().proc.pgid() as u64),
        SYS_SETSID => proc::setsid(),
        SYS_GETPGID => proc::getpgid(a[0] as u32),
        SYS_GETSID => proc::getsid(a[0] as u32),
        SYS_CAPGET | SYS_CAPSET => Ok(0),
        SYS_RT_SIGPENDING => signal::rt_sigpending(u(a[0]), u(a[1])),
        SYS_RT_SIGTIMEDWAIT => signal::rt_sigtimedwait(u(a[0]), u(a[1]), u(a[2]), u(a[3])),
        SYS_RT_SIGQUEUEINFO => signal::kill(a[0] as i64, i(a[1])),
        SYS_RT_SIGSUSPEND => signal::rt_sigsuspend(u(a[0]), u(a[1])),
        SYS_SIGALTSTACK => signal::sigaltstack(u(a[0]), u(a[1])),
        SYS_UTIME | SYS_UTIMES | SYS_FUTIMESAT | SYS_UTIMENSAT => Ok(0),
        SYS_MKNOD => fs::mknodat(crate::fs::vfs::AT_FDCWD, u(a[0]), a[1] as u32, a[2]),
        SYS_PERSONALITY => Ok(0),
        SYS_STATFS => fs::statfs(u(a[0]), u(a[1])),
        SYS_FSTATFS => fs::fstatfs(i(a[0]), u(a[1])),
        SYS_GETPRIORITY => Ok(20),
        SYS_SETPRIORITY => Ok(0),
        SYS_SCHED_SETPARAM | SYS_SCHED_SETSCHEDULER | SYS_SCHED_SETATTR => Ok(0),
        SYS_SCHED_GETPARAM => {
            crate::mm::user::write_user::<i32>(u(a[1]), 0)?;
            Ok(0)
        }
        SYS_SCHED_GETSCHEDULER => Ok(0),
        SYS_SCHED_GET_PRIORITY_MAX | SYS_SCHED_GET_PRIORITY_MIN => Ok(0),
        SYS_SCHED_RR_GET_INTERVAL => {
            time::write_timespec(u(a[1]), 20_000_000)?;
            Ok(0)
        }
        SYS_MLOCK | SYS_MUNLOCK | SYS_MLOCKALL | SYS_MUNLOCKALL | SYS_MLOCK2 => Ok(0),
        SYS_PRCTL => misc::prctl(i(a[0]), a[1], a[2], a[3], a[4]),
        SYS_ARCH_PRCTL => misc::arch_prctl(i(a[0]), a[1]),
        SYS_SETRLIMIT => misc::setrlimit(u(a[0]), u(a[1])),
        SYS_CHROOT => Ok(0),
        SYS_SYNC => Ok(0),
        SYS_MOUNT => fs::mount(u(a[0]), u(a[1]), u(a[2]), a[3], u(a[4])),
        SYS_UMOUNT2 => fs::umount(u(a[0])),
        SYS_REBOOT => misc::reboot(a[2] as u32),
        SYS_SETHOSTNAME => misc::sethostname(u(a[0]), u(a[1])),
        SYS_SETDOMAINNAME => Ok(0),
        SYS_GETTID => Ok(crate::sched::current_ref().tid as u64),
        SYS_TKILL => signal::tgkill(0, a[0] as u32, i(a[1])),
        SYS_TIME => time::time(u(a[0])),
        SYS_FUTEX => futex::futex(u(a[0]), a[1] as u32, a[2] as u32, u(a[3]), u(a[4]), a[5] as u32),
        SYS_SCHED_SETAFFINITY => Ok(0),
        SYS_SCHED_GETAFFINITY => misc::sched_getaffinity(u(a[1]), u(a[2])),
        SYS_EPOLL_CREATE | SYS_EPOLL_CREATE1 => poll::epoll_create(a[0] as u32),
        SYS_GETDENTS64 => fs::getdents64(i(a[0]), u(a[1]), u(a[2])),
        SYS_SET_TID_ADDRESS => {
            let t = crate::sched::current_ref();
            t.clear_child_tid.store(a[0], Ordering::Relaxed);
            Ok(t.tid as u64)
        }
        SYS_FADVISE64 => Ok(0),
        SYS_TIMER_CREATE => Err(ENOSYS),
        SYS_CLOCK_SETTIME => time::clock_settime(i(a[0]), u(a[1])),
        SYS_CLOCK_GETTIME => time::clock_gettime(i(a[0]), u(a[1])),
        SYS_CLOCK_GETRES => time::clock_getres(i(a[0]), u(a[1])),
        SYS_CLOCK_NANOSLEEP => time::clock_nanosleep(i(a[0]), i(a[1]), u(a[2]), u(a[3])),
        SYS_EXIT_GROUP => proc::exit_group(i(a[0])),
        SYS_EPOLL_WAIT => poll::epoll_wait(i(a[0]), u(a[1]), i(a[2]), a[3] as i32 as i64),
        SYS_EPOLL_CTL => poll::epoll_ctl(i(a[0]), i(a[1]), i(a[2]), u(a[3])),
        SYS_TGKILL => signal::tgkill(a[0] as u32, a[1] as u32, i(a[2])),
        SYS_WAITID => proc::waitid(i(a[0]), a[1] as i64, u(a[2]), a[3] as u32, u(a[4])),
        SYS_IOPRIO_SET | SYS_IOPRIO_GET => Ok(0),
        SYS_INOTIFY_INIT | SYS_INOTIFY_INIT1 | SYS_INOTIFY_ADD_WATCH => Err(ENOSYS),
        SYS_OPENAT => fs::openat(i(a[0]), u(a[1]), a[2] as u32, a[3] as u32),
        SYS_MKDIRAT => fs::mkdirat(i(a[0]), u(a[1]), a[2] as u32),
        SYS_MKNODAT => fs::mknodat(i(a[0]), u(a[1]), a[2] as u32, a[3]),
        SYS_NEWFSTATAT => fs::stat_path(i(a[0]), u(a[1]), u(a[2]), a[3] as u32),
        SYS_UNLINKAT => fs::unlinkat(i(a[0]), u(a[1]), a[2] as u32),
        SYS_RENAMEAT | SYS_RENAMEAT2 => fs::renameat(i(a[0]), u(a[1]), i(a[2]), u(a[3])),
        SYS_LINKAT => fs::linkat(i(a[0]), u(a[1]), i(a[2]), u(a[3]), a[4] as u32),
        SYS_SYMLINKAT => fs::symlinkat(u(a[0]), i(a[1]), u(a[2])),
        SYS_READLINKAT => fs::readlinkat(i(a[0]), u(a[1]), u(a[2]), u(a[3])),
        SYS_FCHMODAT => fs::fchmodat(i(a[0]), u(a[1]), a[2] as u32),
        SYS_FACCESSAT | SYS_FACCESSAT2 => fs::faccessat(i(a[0]), u(a[1]), a[2] as u32, a[3] as u32),
        SYS_PSELECT6 => poll::pselect6(i(a[0]), u(a[1]), u(a[2]), u(a[3]), u(a[4]), u(a[5])),
        SYS_PPOLL => poll::ppoll(u(a[0]), u(a[1]), u(a[2]), u(a[3]), u(a[4])),
        SYS_UNSHARE => Ok(0),
        SYS_SET_ROBUST_LIST | SYS_GET_ROBUST_LIST => Ok(0),
        SYS_SPLICE | SYS_TEE | SYS_COPY_FILE_RANGE => Err(ENOSYS),
        SYS_SYNC_FILE_RANGE => Ok(0),
        SYS_EPOLL_PWAIT => poll::epoll_wait(i(a[0]), u(a[1]), i(a[2]), a[3] as i32 as i64),
        SYS_EPOLL_PWAIT2 => poll::epoll_pwait2(i(a[0]), u(a[1]), i(a[2]), u(a[3])),
        SYS_SIGNALFD | SYS_SIGNALFD4 => Err(ENOSYS),
        SYS_TIMERFD_CREATE => poll::timerfd_create(i(a[0]), a[1] as u32),
        SYS_EVENTFD => poll::eventfd(a[0] as u32, 0),
        SYS_EVENTFD2 => poll::eventfd(a[0] as u32, a[1] as u32),
        SYS_FALLOCATE => fs::fallocate(i(a[0]), a[1] as u32, a[2], a[3]),
        SYS_TIMERFD_SETTIME => poll::timerfd_settime(i(a[0]), a[1] as u32, u(a[2]), u(a[3])),
        SYS_TIMERFD_GETTIME => poll::timerfd_gettime(i(a[0]), u(a[1])),
        SYS_DUP3 => fs::dup3(i(a[0]), i(a[1]), a[2] as u32, false),
        SYS_PIPE2 => fs::pipe2(u(a[0]), a[1] as u32),
        SYS_PREADV | SYS_PREADV2 => fs::preadv(i(a[0]), u(a[1]), u(a[2]), a[3]),
        SYS_PWRITEV | SYS_PWRITEV2 => fs::pwritev(i(a[0]), u(a[1]), u(a[2]), a[3]),
        SYS_RT_TGSIGQUEUEINFO => signal::tgkill(a[0] as u32, a[1] as u32, i(a[2])),
        SYS_PRLIMIT64 => misc::prlimit64(a[0] as u32, u(a[1]), u(a[2]), u(a[3])),
        SYS_GETCPU => {
            if a[0] != 0 {
                crate::mm::user::write_user::<u32>(u(a[0]), crate::arch::x86_64::percpu::cpu_id())?;
            }
            if a[1] != 0 {
                crate::mm::user::write_user::<u32>(u(a[1]), 0)?;
            }
            Ok(0)
        }
        SYS_SCHED_GETATTR => Err(ENOSYS),
        SYS_GETRANDOM => misc::getrandom(u(a[0]), u(a[1]), a[2] as u32),
        SYS_MEMFD_CREATE => fs::memfd_create(u(a[0]), a[1] as u32),
        SYS_EXECVEAT => proc::execveat(frame, i(a[0]), u(a[1]), u(a[2]), u(a[3]), a[4] as u32),
        SYS_MEMBARRIER => Ok(0),
        SYS_STATX => fs::statx(i(a[0]), u(a[1]), a[2] as u32, a[3] as u32, u(a[4])),
        SYS_RSEQ => Err(ENOSYS),
        SYS_PIDFD_SEND_SIGNAL | SYS_PIDFD_OPEN | SYS_PIDFD_GETFD => Err(ENOSYS),
        SYS_CLONE3 => proc::clone3(frame, u(a[0]), u(a[1])),
        SYS_CLOSE_RANGE => fs::close_range(a[0] as u32, a[1] as u32, a[2] as u32),
        SYS_OPENAT2 => Err(ENOSYS),
        SYS_FUTEX_WAITV => Err(ENOSYS),
        _ => {
            klog!("sys", "unimplemented syscall {} ({}) from pid {}", nr, name(nr), crate::sched::current_ref().proc.pid);
            Err(ENOSYS)
        }
    }
}
