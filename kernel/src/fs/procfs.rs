//! /proc: a synthetic view of processes and kernel state.

use super::vfs::{self, DirEntry, Inode, Kind, Stat};
use crate::mm::errno::*;
use crate::proc::process::{self, Process};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Root,
    SelfLink,
    ThreadSelfLink,
    RootFile(&'static str),
    Sys,
    SysKernel,
    SysFile(&'static str),
    Pid(u32),
    PidFile(u32, &'static str),
    PidLink(u32, &'static str),
    PidFd(u32),
    PidFdLink(u32, i32),
    PidTask(u32),
}

const ROOT_FILES: &[&str] = &["cpuinfo", "meminfo", "uptime", "version", "mounts", "stat", "loadavg", "filesystems", "cmdline", "devices", "kmsg"];
const SYS_KERNEL_FILES: &[&str] = &["hostname", "osrelease", "ostype", "version", "pid_max", "random"];
const PID_FILES: &[&str] = &["stat", "status", "cmdline", "comm", "maps", "environ", "statm", "mounts", "limits", "oom_score_adj", "stack"];
const PID_LINKS: &[&str] = &["exe", "cwd", "root"];

pub struct ProcInode {
    node: Node,
    fs_id: u64,
}

fn fs_id() -> u64 {
    static ID: crate::sync::Once<u64> = crate::sync::Once::new();
    *ID.call_once(vfs::next_fs_id)
}

fn mk(node: Node) -> Arc<dyn Inode> {
    Arc::new(ProcInode { node, fs_id: fs_id() })
}

pub fn new_fs() -> Arc<dyn Inode> {
    mk(Node::Root)
}

fn ino_of(n: &Node) -> u64 {
    match n {
        Node::Root => 1,
        Node::SelfLink => 2,
        Node::ThreadSelfLink => 3,
        Node::Sys => 4,
        Node::SysKernel => 5,
        Node::RootFile(f) => 0x100 + ROOT_FILES.iter().position(|x| x == f).unwrap_or(0) as u64,
        Node::SysFile(f) => 0x200 + SYS_KERNEL_FILES.iter().position(|x| x == f).unwrap_or(0) as u64,
        Node::Pid(p) => (*p as u64) << 16,
        Node::PidFile(p, f) => ((*p as u64) << 16) | 0x100 | PID_FILES.iter().position(|x| x == f).unwrap_or(0) as u64,
        Node::PidLink(p, f) => ((*p as u64) << 16) | 0x200 | PID_LINKS.iter().position(|x| x == f).unwrap_or(0) as u64,
        Node::PidFd(p) => ((*p as u64) << 16) | 0x300,
        Node::PidFdLink(p, fd) => ((*p as u64) << 16) | 0x1000 | (*fd as u64 & 0xfff),
        Node::PidTask(p) => ((*p as u64) << 16) | 0x400,
    }
}

fn current_pid() -> u32 {
    crate::sched::try_current().map(|t| t.proc.pid).unwrap_or(0)
}

fn proc_of(pid: u32) -> Result<Arc<Process>> {
    process::lookup(pid).ok_or(ENOENT)
}

fn state_char(p: &Process) -> char {
    match p.state() {
        process::ProcState::Zombie => 'Z',
        process::ProcState::Stopped => 'T',
        _ => {
            let th = p.threads.lock();
            if th.iter().any(|t| t.state() == crate::sched::task::TaskState::Running || t.state() == crate::sched::task::TaskState::Runnable) {
                'R'
            } else {
                'S'
            }
        }
    }
}

fn gen_content(node: &Node) -> Result<Vec<u8>> {
    let s: String = match node {
        Node::RootFile("cpuinfo") => {
            let f = crate::arch::x86_64::cpu::features();
            let mut s = String::new();
            let n = crate::arch::x86_64::percpu::count().max(1);
            for i in 0..n {
                s.push_str(&format!(
                    "processor\t: {}\nvendor_id\t: {}\ncpu family\t: {}\nmodel\t\t: {}\nmodel name\t: {}\ncpu MHz\t\t: {}\nflags\t\t: fpu tsc msr pae cx8 apic sep pge cmov pat clflush mmx fxsr sse sse2 ht syscall nx lm sse3 ssse3 sse4_1 sse4_2 popcnt {}{}{}{}\n\n",
                    i,
                    f.vendor(),
                    f.family,
                    f.model,
                    f.brand(),
                    crate::arch::x86_64::tsc::frequency() / 1_000_000,
                    if f.avx { "avx " } else { "" },
                    if f.avx2 { "avx2 " } else { "" },
                    if f.avx512f { "avx512f " } else { "" },
                    if f.fma { "fma f16c " } else { "" }
                ));
            }
            s
        }
        Node::RootFile("meminfo") => {
            let (total, free) = crate::mm::pmm::stats();
            let (heap, _) = crate::mm::heap::stats();
            format!(
                "MemTotal:       {:8} kB\nMemFree:        {:8} kB\nMemAvailable:   {:8} kB\nBuffers:               0 kB\nCached:                0 kB\nSwapTotal:             0 kB\nSwapFree:              0 kB\nShmem:                 0 kB\nSlab:           {:8} kB\n",
                total * 4,
                free * 4,
                free * 4,
                heap / 1024
            )
        }
        Node::RootFile("uptime") => {
            let up = crate::arch::x86_64::tsc::uptime_ns();
            format!("{}.{:02} {}.{:02}\n", up / 1_000_000_000, (up / 10_000_000) % 100, up / 1_000_000_000, (up / 10_000_000) % 100)
        }
        Node::RootFile("version") => format!("MindOS version {} (mind@mindos) (rustc) #1 {}\n", crate::VERSION, "2026"),
        Node::RootFile("mounts") | Node::PidFile(_, "mounts") => {
            let mut s = String::new();
            for (path, fstype) in super::path::mounts() {
                let dev = match fstype {
                    "initrd" => "initrd",
                    "devtmpfs" => "devtmpfs",
                    "proc" => "proc",
                    "ext2" => "/dev/vda2",
                    _ => "tmpfs",
                };
                s.push_str(&format!("{} {} {} rw 0 0\n", dev, path, fstype));
            }
            s
        }
        Node::RootFile("stat") => {
            let ticks = crate::sched::TICKS.load(Ordering::Relaxed);
            format!("cpu  {} 0 0 {} 0 0 0 0 0 0\nintr 0\nctxt 0\nbtime {}\nprocesses {}\nprocs_running 1\nprocs_blocked 0\n", ticks / 10, ticks, crate::dev::rtc::wall_time_ns() / 1_000_000_000 - crate::arch::x86_64::tsc::uptime_ns() / 1_000_000_000, process::count())
        }
        Node::RootFile("loadavg") => format!("0.00 0.00 0.00 {}/{} {}\n", crate::sched::runnable_count() + 1, process::count(), process::count() + 1),
        Node::RootFile("filesystems") => String::from("nodev\tproc\nnodev\ttmpfs\nnodev\tdevtmpfs\n\text2\n"),
        Node::RootFile("cmdline") => format!("{}\n", crate::boot::limine::info().cmdline),
        Node::RootFile("devices") => String::from("Character devices:\n  1 mem\n  4 tty\n  5 /dev/tty\n  5 /dev/console\n  5 /dev/ptmx\n 10 misc\n 13 input\n 29 fb\n136 pts\n\nBlock devices:\n254 virtblk\n"),
        Node::RootFile("kmsg") => {
            let mut buf = alloc::vec![0u8; 128 * 1024];
            let (n, _) = crate::console::CONSOLE.lock().log.read_from(0, &mut buf);
            buf.truncate(n);
            return Ok(buf);
        }
        Node::SysFile("hostname") => format!("{}\n", crate::syscall::misc::hostname()),
        Node::SysFile("osrelease") => format!("{}\n", crate::syscall::misc::RELEASE),
        Node::SysFile("ostype") => String::from("Linux\n"),
        Node::SysFile("version") => String::from("#1 SMP MindOS\n"),
        Node::SysFile("pid_max") => String::from("4000000\n"),
        Node::SysFile("random") => String::new(),
        Node::PidFile(pid, "stat") => {
            let p = proc_of(*pid)?;
            let cpu: u64 = p.threads.lock().iter().map(|t| t.cpu_time_ns.load(Ordering::Relaxed)).sum();
            let ticks = cpu / 10_000_000;
            let rss = p.aspace().map(|a| a.rss()).unwrap_or(0);
            let vsize = p.aspace().map(|a| a.vsize()).unwrap_or(0);
            let tty_nr = p.tty.lock().as_ref().map(|t| if t.is_pty_slave { (136 << 8) | t.index } else { (4 << 8) | 1 }).unwrap_or(0);
            format!(
                "{} ({}) {} {} {} {} {} 0 0 0 0 0 {} 0 0 0 20 0 {} 0 {} {} {} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
                p.pid,
                p.comm(),
                state_char(&p),
                p.ppid(),
                p.pgid(),
                p.sid(),
                tty_nr,
                ticks,
                p.thread_count(),
                p.start_ns / 10_000_000,
                vsize,
                rss
            )
        }
        Node::PidFile(pid, "status") => {
            let p = proc_of(*pid)?;
            let rss = p.aspace().map(|a| a.rss()).unwrap_or(0);
            let vsize = p.aspace().map(|a| a.vsize()).unwrap_or(0);
            let st = match state_char(&p) {
                'R' => "R (running)",
                'Z' => "Z (zombie)",
                'T' => "T (stopped)",
                _ => "S (sleeping)",
            };
            let pending = p.shared_pending();
            let blocked = p.main_thread().map(|t| t.signal_mask()).unwrap_or(0);
            format!(
                "Name:\t{}\nUmask:\t{:04o}\nState:\t{}\nTgid:\t{}\nNgid:\t0\nPid:\t{}\nPPid:\t{}\nTracerPid:\t0\nUid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\nFDSize:\t64\nGroups:\t0\nVmPeak:\t{:8} kB\nVmSize:\t{:8} kB\nVmRSS:\t{:8} kB\nThreads:\t{}\nSigQ:\t0/4096\nSigPnd:\t{:016x}\nShdPnd:\t{:016x}\nSigBlk:\t{:016x}\nSigIgn:\t0000000000000000\nSigCgt:\t0000000000000000\nCpus_allowed:\tf\nCpus_allowed_list:\t0-3\n",
                p.comm(),
                p.umask.load(Ordering::Relaxed),
                st,
                p.pid,
                p.pid,
                p.ppid(),
                vsize / 1024,
                vsize / 1024,
                rss * 4,
                p.thread_count(),
                0,
                pending,
                blocked
            )
        }
        Node::PidFile(pid, "cmdline") => {
            let p = proc_of(*pid)?;
            let mut v = Vec::new();
            for a in p.cmdline.lock().iter() {
                v.extend_from_slice(a.as_bytes());
                v.push(0);
            }
            return Ok(v);
        }
        Node::PidFile(pid, "comm") => format!("{}\n", proc_of(*pid)?.comm()),
        Node::PidFile(pid, "maps") => {
            let p = proc_of(*pid)?;
            let mut s = String::new();
            if let Some(a) = p.aspace() {
                let inner = a.inner.lock();
                for v in inner.vmas.values() {
                    let perms = format!(
                        "{}{}{}{}",
                        if v.prot & 1 != 0 { 'r' } else { '-' },
                        if v.prot & 2 != 0 { 'w' } else { '-' },
                        if v.prot & 4 != 0 { 'x' } else { '-' },
                        if v.shared { 's' } else { 'p' }
                    );
                    let off = match &v.backing {
                        crate::mm::addrspace::Backing::File { offset, .. } => *offset,
                        _ => 0,
                    };
                    s.push_str(&format!("{:012x}-{:012x} {} {:08x} 00:00 0 {}\n", v.start, v.end, perms, off, v.name));
                }
            }
            s
        }
        Node::PidFile(_, "environ") => String::new(),
        Node::PidFile(pid, "statm") => {
            let p = proc_of(*pid)?;
            let rss = p.aspace().map(|a| a.rss()).unwrap_or(0);
            let vsize = p.aspace().map(|a| a.vsize()).unwrap_or(0) / 4096;
            format!("{} {} 0 0 0 {} 0\n", vsize, rss, rss)
        }
        Node::PidFile(pid, "limits") => {
            let p = proc_of(*pid)?;
            let (soft, hard) = p.rlimit(process::RLIMIT_NOFILE);
            format!("Limit                     Soft Limit           Hard Limit           Units     \nMax open files            {:<20} {:<20} files     \n", soft, hard)
        }
        Node::PidFile(_, "oom_score_adj") => String::from("0\n"),
        Node::PidFile(_, "stack") => String::new(),
        _ => return Err(EINVAL),
    };
    Ok(s.into_bytes())
}

impl ProcInode {
    fn kind_of(&self) -> Kind {
        match &self.node {
            Node::Root | Node::Sys | Node::SysKernel | Node::Pid(_) | Node::PidFd(_) | Node::PidTask(_) => Kind::Dir,
            Node::SelfLink | Node::ThreadSelfLink | Node::PidLink(..) | Node::PidFdLink(..) => Kind::Symlink,
            _ => Kind::File,
        }
    }

    fn entries(&self) -> Vec<DirEntry> {
        let mut v = Vec::new();
        let dir = |n: &str, node: Node| DirEntry { name: String::from(n), ino: ino_of(&node), kind: Kind::Dir };
        let file = |n: &str, node: Node| DirEntry { name: String::from(n), ino: ino_of(&node), kind: Kind::File };
        let link = |n: &str, node: Node| DirEntry { name: String::from(n), ino: ino_of(&node), kind: Kind::Symlink };
        match &self.node {
            Node::Root => {
                v.push(link("self", Node::SelfLink));
                v.push(link("thread-self", Node::ThreadSelfLink));
                v.push(dir("sys", Node::Sys));
                for f in ROOT_FILES {
                    v.push(file(f, Node::RootFile(f)));
                }
                let mut pids: Vec<u32> = process::all().iter().map(|p| p.pid).collect();
                pids.sort();
                for pid in pids {
                    v.push(dir(&format!("{}", pid), Node::Pid(pid)));
                }
            }
            Node::Sys => v.push(dir("kernel", Node::SysKernel)),
            Node::SysKernel => {
                for f in SYS_KERNEL_FILES {
                    v.push(file(f, Node::SysFile(f)));
                }
            }
            Node::Pid(pid) => {
                for f in PID_FILES {
                    v.push(file(f, Node::PidFile(*pid, f)));
                }
                for f in PID_LINKS {
                    v.push(link(f, Node::PidLink(*pid, f)));
                }
                v.push(dir("fd", Node::PidFd(*pid)));
                v.push(dir("task", Node::PidTask(*pid)));
            }
            Node::PidFd(pid) => {
                if let Some(p) = process::lookup(*pid) {
                    for (fd, _) in p.files().iter() {
                        v.push(link(&format!("{}", fd), Node::PidFdLink(*pid, fd)));
                    }
                }
            }
            Node::PidTask(pid) => {
                if let Some(p) = process::lookup(*pid) {
                    for t in p.threads.lock().iter() {
                        v.push(dir(&format!("{}", t.tid), Node::Pid(*pid)));
                    }
                }
            }
            _ => {}
        }
        v
    }
}

impl Inode for ProcInode {
    fn kind(&self) -> Kind {
        self.kind_of()
    }
    fn ino(&self) -> u64 {
        ino_of(&self.node)
    }
    fn fs_id(&self) -> u64 {
        self.fs_id
    }
    fn fs_name(&self) -> &'static str {
        "proc"
    }
    fn stat(&self) -> Stat {
        let kind = self.kind_of();
        let (mode, size) = match kind {
            Kind::Dir => (0o555, 0),
            Kind::Symlink => (0o777, 0),
            _ => (0o444, 0),
        };
        let mut s = Stat::simple(self.fs_id, self.ino(), kind, mode, size, if kind == Kind::Dir { 2 } else { 1 });
        s.st_blksize = 1024;
        s
    }
    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if self.kind_of() != Kind::Dir {
            return Err(ENOTDIR);
        }
        if name == "." {
            return Ok(mk(self.node.clone()));
        }
        if name == ".." {
            let parent = match &self.node {
                Node::Root => Node::Root,
                Node::Sys | Node::Pid(_) => Node::Root,
                Node::SysKernel => Node::Sys,
                Node::PidFd(p) | Node::PidTask(p) => Node::Pid(*p),
                _ => Node::Root,
            };
            return Ok(mk(parent));
        }
        match &self.node {
            Node::Root => {
                if name == "self" {
                    return Ok(mk(Node::SelfLink));
                }
                if name == "thread-self" {
                    return Ok(mk(Node::ThreadSelfLink));
                }
                if name == "sys" {
                    return Ok(mk(Node::Sys));
                }
                if let Some(f) = ROOT_FILES.iter().find(|f| **f == name) {
                    return Ok(mk(Node::RootFile(f)));
                }
                if let Ok(pid) = name.parse::<u32>() {
                    if process::lookup(pid).is_some() {
                        return Ok(mk(Node::Pid(pid)));
                    }
                }
                Err(ENOENT)
            }
            Node::Sys => {
                if name == "kernel" {
                    Ok(mk(Node::SysKernel))
                } else {
                    Err(ENOENT)
                }
            }
            Node::SysKernel => SYS_KERNEL_FILES.iter().find(|f| **f == name).map(|f| mk(Node::SysFile(f))).ok_or(ENOENT),
            Node::Pid(pid) => {
                if let Some(f) = PID_FILES.iter().find(|f| **f == name) {
                    return Ok(mk(Node::PidFile(*pid, f)));
                }
                if let Some(f) = PID_LINKS.iter().find(|f| **f == name) {
                    return Ok(mk(Node::PidLink(*pid, f)));
                }
                if name == "fd" {
                    return Ok(mk(Node::PidFd(*pid)));
                }
                if name == "task" {
                    return Ok(mk(Node::PidTask(*pid)));
                }
                Err(ENOENT)
            }
            Node::PidFd(pid) => {
                let fd: i32 = name.parse().map_err(|_| ENOENT)?;
                let p = proc_of(*pid)?;
                p.files().get(fd).map_err(|_| ENOENT)?;
                Ok(mk(Node::PidFdLink(*pid, fd)))
            }
            Node::PidTask(pid) => {
                let tid: u32 = name.parse().map_err(|_| ENOENT)?;
                let p = proc_of(*pid)?;
                if p.threads.lock().iter().any(|t| t.tid == tid) {
                    Ok(mk(Node::Pid(*pid)))
                } else {
                    Err(ENOENT)
                }
            }
            _ => Err(ENOTDIR),
        }
    }
    fn readdir(&self, pos: usize) -> Result<Option<DirEntry>> {
        if self.kind_of() != Kind::Dir {
            return Err(ENOTDIR);
        }
        if pos == 0 {
            return Ok(Some(DirEntry { name: String::from("."), ino: self.ino(), kind: Kind::Dir }));
        }
        if pos == 1 {
            return Ok(Some(DirEntry { name: String::from(".."), ino: 1, kind: Kind::Dir }));
        }
        Ok(self.entries().into_iter().nth(pos - 2))
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize> {
        if self.kind_of() == Kind::Dir {
            return Err(EISDIR);
        }
        let content = gen_content(&self.node)?;
        if off as usize >= content.len() {
            return Ok(0);
        }
        let n = (content.len() - off as usize).min(buf.len());
        buf[..n].copy_from_slice(&content[off as usize..off as usize + n]);
        Ok(n)
    }
    fn write_at(&self, _off: u64, buf: &[u8]) -> Result<usize> {
        match &self.node {
            Node::SysFile("hostname") => {
                let s = core::str::from_utf8(buf).map_err(|_| EINVAL)?.trim();
                crate::syscall::misc::set_hostname(s);
                Ok(buf.len())
            }
            Node::PidFile(_, "oom_score_adj") | Node::PidFile(_, "comm") => Ok(buf.len()),
            _ => Err(EACCES),
        }
    }
    fn readlink(&self) -> Result<String> {
        match &self.node {
            Node::SelfLink => Ok(format!("{}", current_pid())),
            Node::ThreadSelfLink => Ok(format!("{}/task/{}", current_pid(), crate::sched::try_current().map(|t| t.tid).unwrap_or(0))),
            Node::PidLink(pid, "exe") => Ok(proc_of(*pid)?.exe.lock().clone()),
            Node::PidLink(pid, "cwd") => Ok(proc_of(*pid)?.cwd().1),
            Node::PidLink(_, "root") => Ok(String::from("/")),
            Node::PidFdLink(pid, fd) => {
                let p = proc_of(*pid)?;
                let f = p.files().get(*fd).map_err(|_| ENOENT)?;
                Ok(f.path())
            }
            _ => Err(EINVAL),
        }
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
