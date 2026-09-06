//! The process (thread group) structure and the global process table.

use crate::dev::tty::Tty;
use crate::fs::file::FdTable;
use crate::fs::vfs::Inode;
use crate::mm::addrspace::AddressSpace;
use crate::sched::task::Task;
use crate::sched::wait::WaitQueue;
use crate::sync::SpinLock;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicU8, Ordering};

pub const RLIMIT_CPU: usize = 0;
pub const RLIMIT_FSIZE: usize = 1;
pub const RLIMIT_DATA: usize = 2;
pub const RLIMIT_STACK: usize = 3;
pub const RLIMIT_CORE: usize = 4;
pub const RLIMIT_RSS: usize = 5;
pub const RLIMIT_NPROC: usize = 6;
pub const RLIMIT_NOFILE: usize = 7;
pub const RLIMIT_MEMLOCK: usize = 8;
pub const RLIMIT_AS: usize = 9;
pub const RLIM_INFINITY: u64 = u64::MAX;
pub const RLIMIT_COUNT: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProcState {
    Running = 0,
    Stopped = 1,
    Zombie = 2,
}

pub struct FsInfo {
    pub cwd: Arc<dyn Inode>,
    pub cwd_path: String,
    pub root: Arc<dyn Inode>,
}

#[derive(Clone, Copy)]
pub struct SigAction {
    pub handler: u64,
    pub flags: u64,
    pub restorer: u64,
    pub mask: u64,
}

impl SigAction {
    pub const DEFAULT: SigAction = SigAction { handler: 0, flags: 0, restorer: 0, mask: 0 };
}

pub struct SigHandlers {
    pub actions: SpinLock<[SigAction; 65]>,
}

impl SigHandlers {
    pub fn new() -> Arc<SigHandlers> {
        Arc::new(SigHandlers { actions: SpinLock::new([SigAction::DEFAULT; 65]) })
    }
    pub fn duplicate(&self) -> Arc<SigHandlers> {
        Arc::new(SigHandlers { actions: SpinLock::new(*self.actions.lock()) })
    }
    pub fn get(&self, signo: i32) -> SigAction {
        self.actions.lock()[signo as usize]
    }
    pub fn set(&self, signo: i32, a: SigAction) {
        self.actions.lock()[signo as usize] = a;
    }
    /// On exec: handled signals revert to default, ignored ones stay ignored.
    pub fn reset_for_exec(&self) {
        let mut a = self.actions.lock();
        for s in a.iter_mut() {
            if s.handler != 1 {
                *s = SigAction::DEFAULT;
            } else {
                s.flags = 0;
                s.mask = 0;
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct ITimer {
    pub next_ns: u64,
    pub interval_ns: u64,
}

pub struct Process {
    pub pid: u32,
    pub parent: SpinLock<Weak<Process>>,
    pub children: SpinLock<Vec<Arc<Process>>>,
    pub threads: SpinLock<Vec<Arc<Task>>>,
    aspace: SpinLock<Option<Arc<AddressSpace>>>,
    pub files: SpinLock<Arc<FdTable>>,
    pub fs: SpinLock<Option<FsInfo>>,
    pub sighand: SpinLock<Arc<SigHandlers>>,
    pub shared_pending: AtomicU64,
    pub state: AtomicU8,
    pub exit_code: AtomicI32,
    pub pgid: AtomicU32,
    pub sid: AtomicU32,
    pub child_wq: WaitQueue,
    pub vfork_done: AtomicBool,
    pub vfork_wq: WaitQueue,
    pub comm: SpinLock<String>,
    pub exe: SpinLock<String>,
    pub cmdline: SpinLock<Vec<String>>,
    pub tty: SpinLock<Option<Arc<Tty>>>,
    pub itimer_real: SpinLock<ITimer>,
    pub rlimits: SpinLock<[(u64, u64); RLIMIT_COUNT]>,
    pub start_ns: u64,
    pub umask: AtomicU32,
    pub uid: AtomicU32,
    pub gid: AtomicU32,
    pub group_exit: AtomicBool,
    pub stop_signal: AtomicI32,
    pub stopped_reported: AtomicBool,
    pub continued_reported: AtomicBool,
    pub stop_wq: WaitQueue,
    pub exit_signal: AtomicI32,
    pub mind_client: AtomicBool,
}

static PROCS: SpinLock<BTreeMap<u32, Arc<Process>>> = SpinLock::new(BTreeMap::new());
static NEXT_PID: AtomicU32 = AtomicU32::new(1);
static KERNEL_PROC: crate::sync::Once<Arc<Process>> = crate::sync::Once::new();

fn default_rlimits() -> [(u64, u64); RLIMIT_COUNT] {
    let mut r = [(RLIM_INFINITY, RLIM_INFINITY); RLIMIT_COUNT];
    r[RLIMIT_STACK] = (8 * 1024 * 1024, RLIM_INFINITY);
    r[RLIMIT_NOFILE] = (1024, 4096);
    r[RLIMIT_CORE] = (0, RLIM_INFINITY);
    r[RLIMIT_NPROC] = (4096, 4096);
    r
}

impl Process {
    fn raw(pid: u32) -> Process {
        Process {
            pid,
            parent: SpinLock::new(Weak::new()),
            children: SpinLock::new(Vec::new()),
            threads: SpinLock::new(Vec::new()),
            aspace: SpinLock::new(None),
            files: SpinLock::new(FdTable::new()),
            fs: SpinLock::new(None),
            sighand: SpinLock::new(SigHandlers::new()),
            shared_pending: AtomicU64::new(0),
            state: AtomicU8::new(ProcState::Running as u8),
            exit_code: AtomicI32::new(0),
            pgid: AtomicU32::new(pid),
            sid: AtomicU32::new(pid),
            child_wq: WaitQueue::new(),
            vfork_done: AtomicBool::new(false),
            vfork_wq: WaitQueue::new(),
            comm: SpinLock::new(String::new()),
            exe: SpinLock::new(String::new()),
            cmdline: SpinLock::new(Vec::new()),
            tty: SpinLock::new(None),
            itimer_real: SpinLock::new(ITimer::default()),
            rlimits: SpinLock::new(default_rlimits()),
            start_ns: crate::arch::x86_64::tsc::uptime_ns(),
            umask: AtomicU32::new(0o022),
            uid: AtomicU32::new(0),
            gid: AtomicU32::new(0),
            group_exit: AtomicBool::new(false),
            stop_signal: AtomicI32::new(0),
            stopped_reported: AtomicBool::new(false),
            continued_reported: AtomicBool::new(false),
            stop_wq: WaitQueue::new(),
            exit_signal: AtomicI32::new(17),
            mind_client: AtomicBool::new(false),
        }
    }

    /// The kernel's own process (pid 0), owner of kernel threads.
    pub fn kernel_process() -> Arc<Process> {
        KERNEL_PROC
            .call_once(|| {
                let p = Process::raw(0);
                *p.comm.lock() = String::from("kernel");
                Arc::new(p)
            })
            .clone()
    }

    /// Allocate a new process with a fresh pid (registered in the table).
    pub fn new() -> Arc<Process> {
        let pid = alloc_pid();
        let p = Arc::new(Process::raw(pid));
        PROCS.lock().insert(pid, p.clone());
        p
    }

    pub fn aspace(&self) -> Option<Arc<AddressSpace>> {
        self.aspace.lock().clone()
    }
    pub fn set_aspace(&self, a: Option<Arc<AddressSpace>>) -> Option<Arc<AddressSpace>> {
        core::mem::replace(&mut *self.aspace.lock(), a)
    }
    pub fn pml4(&self) -> u64 {
        match self.aspace.lock().as_ref() {
            Some(a) => a.pml4(),
            None => crate::mm::vmm::kernel_pml4(),
        }
    }
    pub fn files(&self) -> Arc<FdTable> {
        self.files.lock().clone()
    }
    pub fn sighand(&self) -> Arc<SigHandlers> {
        self.sighand.lock().clone()
    }
    pub fn comm(&self) -> String {
        self.comm.lock().clone()
    }
    pub fn state(&self) -> ProcState {
        match self.state.load(Ordering::Acquire) {
            0 => ProcState::Running,
            1 => ProcState::Stopped,
            _ => ProcState::Zombie,
        }
    }
    pub fn set_state(&self, s: ProcState) {
        self.state.store(s as u8, Ordering::Release);
    }
    pub fn is_zombie(&self) -> bool {
        self.state() == ProcState::Zombie
    }
    pub fn parent(&self) -> Option<Arc<Process>> {
        self.parent.lock().upgrade()
    }
    pub fn ppid(&self) -> u32 {
        self.parent().map(|p| p.pid).unwrap_or(0)
    }
    pub fn pgid(&self) -> u32 {
        self.pgid.load(Ordering::Relaxed)
    }
    pub fn sid(&self) -> u32 {
        self.sid.load(Ordering::Relaxed)
    }
    pub fn shared_pending(&self) -> u64 {
        self.shared_pending.load(Ordering::Relaxed)
    }
    pub fn cwd(&self) -> (Arc<dyn Inode>, String) {
        let fs = self.fs.lock();
        match fs.as_ref() {
            Some(f) => (f.cwd.clone(), f.cwd_path.clone()),
            None => (crate::fs::path::root(), String::from("/")),
        }
    }
    pub fn set_cwd(&self, inode: Arc<dyn Inode>, path: String) {
        let mut fs = self.fs.lock();
        match fs.as_mut() {
            Some(f) => {
                f.cwd = inode;
                f.cwd_path = path;
            }
            None => {
                *fs = Some(FsInfo { cwd: inode, cwd_path: path, root: crate::fs::path::root() });
            }
        }
    }
    pub fn main_thread(&self) -> Option<Arc<Task>> {
        self.threads.lock().first().cloned()
    }
    pub fn thread_count(&self) -> usize {
        self.threads.lock().len()
    }
    pub fn add_thread(&self, t: Arc<Task>) {
        self.threads.lock().push(t);
    }
    pub fn remove_thread(&self, t: &Arc<Task>) -> usize {
        let mut th = self.threads.lock();
        th.retain(|x| !Arc::ptr_eq(x, t));
        th.len()
    }
    pub fn rlimit(&self, which: usize) -> (u64, u64) {
        self.rlimits.lock()[which]
    }
    pub fn add_child(self: &Arc<Self>, child: &Arc<Process>) {
        *child.parent.lock() = Arc::downgrade(self);
        self.children.lock().push(child.clone());
    }
}

fn alloc_pid() -> u32 {
    loop {
        let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
        if pid >= 4_000_000 {
            NEXT_PID.store(2, Ordering::Relaxed);
            continue;
        }
        if !PROCS.lock().contains_key(&pid) {
            return pid;
        }
    }
}

pub fn lookup(pid: u32) -> Option<Arc<Process>> {
    PROCS.lock().get(&pid).cloned()
}

pub fn all() -> Vec<Arc<Process>> {
    PROCS.lock().values().cloned().collect()
}

pub fn count() -> usize {
    PROCS.lock().len()
}

/// Remove a reaped zombie from the table.
pub fn unregister(pid: u32) {
    PROCS.lock().remove(&pid);
}

/// Find a task by tid across all processes.
pub fn find_task(tid: u32) -> Option<Arc<Task>> {
    for p in PROCS.lock().values() {
        if let Some(t) = p.threads.lock().iter().find(|t| t.tid == tid) {
            return Some(t.clone());
        }
    }
    None
}

pub fn init_process() -> Option<Arc<Process>> {
    lookup(1)
}

// ---- controlling terminal --------------------------------------------------

pub fn controlling_tty() -> Option<Arc<Tty>> {
    crate::sched::try_current().and_then(|t| t.proc.tty.lock().clone())
}

/// Make `tty` the controlling terminal of the current process if it is a
/// session leader without one (or when forced by TIOCSCTTY).
pub fn set_controlling_tty(tty: Arc<Tty>, force: bool) {
    let t = match crate::sched::try_current() {
        Some(t) => t,
        None => return,
    };
    let p = &t.proc;
    if p.pid == 0 {
        return;
    }
    let is_leader = p.sid() == p.pid;
    if !force && !is_leader {
        return;
    }
    let mut cur = p.tty.lock();
    if cur.is_some() && !force {
        return;
    }
    if tty.session.load(Ordering::Relaxed) == 0 || force {
        tty.session.store(p.sid(), Ordering::Relaxed);
        tty.fg_pgrp.store(p.pgid(), Ordering::Relaxed);
    }
    *cur = Some(tty);
}

pub fn drop_controlling_tty() {
    if let Some(t) = crate::sched::try_current() {
        *t.proc.tty.lock() = None;
    }
}

/// Deliver expired real-time interval timers (called from the timer tick).
pub fn tick_itimers(now: u64) {
    let procs: Vec<Arc<Process>> = {
        let table = PROCS.lock();
        table.values().filter(|p| p.itimer_real.lock().next_ns != 0).cloned().collect()
    };
    for p in procs {
        let fire = {
            let mut it = p.itimer_real.lock();
            if it.next_ns != 0 && now >= it.next_ns {
                if it.interval_ns != 0 {
                    it.next_ns = now + it.interval_ns;
                } else {
                    it.next_ns = 0;
                }
                true
            } else {
                false
            }
        };
        if fire {
            super::signal::send(&p, 14); // SIGALRM
        }
    }
}
