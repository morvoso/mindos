//! Task (thread) structure.

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::interrupts::TrapFrame;
use crate::mm::{self, KSTACK_PAGES, PAGE_SIZE};
use crate::proc::process::Process;
use crate::sync::SpinLock;
use alloc::string::String;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicU8, Ordering};

pub type Tid = u32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum TaskState {
    Runnable = 0,
    Running = 1,
    Blocked = 2,
    Zombie = 3,
}

#[repr(C)]
pub struct Context {
    pub rsp: u64,
}

pub struct SignalState {
    pub pending: u64,
    pub mask: u64,
    pub altstack_sp: u64,
    pub altstack_size: u64,
    pub altstack_flags: u32,
}

static NEXT_TID: AtomicU32 = AtomicU32::new(1);

pub struct Task {
    pub tid: Tid,
    pub proc: Arc<Process>,
    pub kstack_top: usize,
    ctx: UnsafeCell<Context>,
    state: AtomicU8,
    pub on_rq: AtomicBool,
    fpu: *mut u8,
    fpu_layout: core::alloc::Layout,
    pub fs_base: AtomicU64,
    pub clear_child_tid: AtomicU64,
    pub set_child_tid: AtomicU64,
    pub sig: SpinLock<SignalState>,
    /// Set when the task is blocked in an interruptible wait.
    pub interruptible: AtomicBool,
    pub slice: AtomicU32,
    pub exit_code: AtomicI32,
    pub cpu_time_ns: AtomicU64,
    last_switch_in: AtomicU64,
    idle: bool,
    entry: SpinLock<Option<(fn(usize), usize)>>,
    pub name: SpinLock<String>,
    pub wake_deadline: AtomicU64,
    pub timed_out: AtomicBool,
    /// Mask to restore after a handler runs (sigsuspend); u64::MAX = none.
    pub saved_mask: AtomicU64,
    /// Number of the syscall being executed (for restarts).
    pub syscall_nr: AtomicU64,
    pub fault_addr: AtomicU64,
    pub fault_code: AtomicI32,
}
unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl Task {
    fn alloc_fpu() -> (*mut u8, core::alloc::Layout) {
        let size = cpu::features().xsave_size as usize;
        let layout = core::alloc::Layout::from_size_align(size.max(576), 64).unwrap();
        let p = unsafe { alloc::alloc::alloc(layout) };
        assert!(!p.is_null(), "out of memory for FPU state");
        cpu::init_fpu_area(unsafe { core::slice::from_raw_parts_mut(p, size) });
        (p, layout)
    }

    fn base(proc: Arc<Process>, kstack_top: usize, idle: bool, name: &str) -> Task {
        let (fpu, fpu_layout) = Self::alloc_fpu();
        Task {
            tid: NEXT_TID.fetch_add(1, Ordering::Relaxed),
            proc,
            kstack_top,
            ctx: UnsafeCell::new(Context { rsp: 0 }),
            state: AtomicU8::new(TaskState::Runnable as u8),
            on_rq: AtomicBool::new(false),
            fpu,
            fpu_layout,
            fs_base: AtomicU64::new(0),
            clear_child_tid: AtomicU64::new(0),
            set_child_tid: AtomicU64::new(0),
            sig: SpinLock::new(SignalState { pending: 0, mask: 0, altstack_sp: 0, altstack_size: 0, altstack_flags: 2 /* SS_DISABLE */ }),
            interruptible: AtomicBool::new(false),
            slice: AtomicU32::new(super::DEFAULT_SLICE),
            exit_code: AtomicI32::new(0),
            cpu_time_ns: AtomicU64::new(0),
            last_switch_in: AtomicU64::new(0),
            idle,
            entry: SpinLock::new(None),
            name: SpinLock::new(String::from(name)),
            wake_deadline: AtomicU64::new(0),
            timed_out: AtomicBool::new(false),
            saved_mask: AtomicU64::new(u64::MAX),
            syscall_nr: AtomicU64::new(0),
            fault_addr: AtomicU64::new(0),
            fault_code: AtomicI32::new(0),
        }
    }

    /// Wrap the boot context as the idle task (uses the bootloader stack).
    pub fn new_idle(cpu: u32) -> Arc<Task> {
        let rsp: u64;
        unsafe { core::arch::asm!("mov {}, rsp", out(reg) rsp) };
        // stack top: round up to the 256 KiB region Limine gave us
        let top = ((rsp as usize) + 0x40000) & !0xFFF;
        let mut t = Task::base(Process::kernel_process(), top, true, "idle");
        t.set_state(TaskState::Running);
        *t.name.lock() = alloc::format!("idle/{}", cpu);
        Arc::new(t)
    }

    /// A kernel thread running `f(arg)`.
    pub fn new_kernel(name: &str, f: fn(usize), arg: usize) -> Arc<Task> {
        Self::new_kernel_in(Process::kernel_process(), name, f, arg)
    }

    /// A kernel-mode thread owned by `proc` (used to bootstrap user processes).
    pub fn new_kernel_in(proc: Arc<Process>, name: &str, f: fn(usize), arg: usize) -> Arc<Task> {
        let top = mm::alloc_kernel_stack(KSTACK_PAGES);
        let t = Task::base(proc, top, false, name);
        *t.entry.lock() = Some((f, arg));
        t.init_stack();
        Arc::new(t)
    }

    /// A user thread for `proc` whose initial register state is `frame`.
    pub fn new_user(proc: Arc<Process>, frame: &TrapFrame, name: &str) -> Arc<Task> {
        let top = mm::alloc_kernel_stack(KSTACK_PAGES);
        let t = Task::base(proc, top, false, name);
        unsafe {
            core::ptr::write(t.user_frame_ptr() as *mut TrapFrame, *frame);
        }
        t.init_stack();
        Arc::new(t)
    }

    /// Prepare the initial kernel stack so the first switch lands in the trampoline.
    fn init_stack(&self) {
        let frame = self.user_frame_ptr() as usize;
        // below the trap frame: 6 callee-saved registers + return address
        let sp = frame - 7 * 8;
        unsafe {
            let p = sp as *mut u64;
            for i in 0..6 {
                p.add(i).write(0);
            }
            p.add(6).write(super::task_trampoline_entry as usize as u64);
            (*self.ctx.get()).rsp = sp as u64;
        }
    }

    #[inline]
    pub fn user_frame_ptr(&self) -> *mut TrapFrame {
        (self.kstack_top - core::mem::size_of::<TrapFrame>()) as *mut TrapFrame
    }
    /// The saved user register state (valid while the task is in the kernel).
    pub fn user_frame(&self) -> &mut TrapFrame {
        unsafe { &mut *self.user_frame_ptr() }
    }

    pub fn ctx_ptr(&self) -> *mut Context {
        self.ctx.get()
    }
    pub fn fpu_area(&self) -> *mut u8 {
        self.fpu
    }
    pub fn copy_fpu_from(&self, other: &Task) {
        let n = cpu::features().xsave_size as usize;
        unsafe { core::ptr::copy_nonoverlapping(other.fpu, self.fpu, n) };
    }

    pub fn state(&self) -> TaskState {
        match self.state.load(Ordering::Acquire) {
            0 => TaskState::Runnable,
            1 => TaskState::Running,
            2 => TaskState::Blocked,
            _ => TaskState::Zombie,
        }
    }
    pub fn set_state(&self, s: TaskState) {
        self.state.store(s as u8, Ordering::Release);
    }
    pub fn is_idle(&self) -> bool {
        self.idle
    }
    pub fn is_current(&self) -> bool {
        match crate::arch::x86_64::percpu::this().current_ref() {
            Some(c) => core::ptr::eq(&**c, self),
            None => false,
        }
    }
    pub fn take_entry(&self) -> Option<(fn(usize), usize)> {
        self.entry.lock().take()
    }
    pub fn is_kernel_thread(&self) -> bool {
        self.proc.pid == 0
    }

    pub fn account_switch_in(&self, now: u64) {
        self.last_switch_in.store(now, Ordering::Relaxed);
    }
    pub fn account_switch_out(&self, now: u64) {
        let start = self.last_switch_in.load(Ordering::Relaxed);
        if start != 0 && now > start {
            self.cpu_time_ns.fetch_add(now - start, Ordering::Relaxed);
        }
    }

    /// Hook run on the child's first entry to user mode after clone.
    pub fn on_first_user_entry(&self) {
        let addr = self.set_child_tid.swap(0, Ordering::Relaxed);
        if addr != 0 {
            let _ = crate::mm::user::write_user::<u32>(addr as usize, self.tid);
        }
    }

    // ---- signals -----------------------------------------------------------

    pub fn pending_unblocked(&self) -> u64 {
        let s = self.sig.lock();
        (s.pending | self.proc.shared_pending()) & !s.mask
    }
    pub fn has_pending_signal(&self) -> bool {
        self.pending_unblocked() != 0
    }
    pub fn signal_mask(&self) -> u64 {
        self.sig.lock().mask
    }
    /// Queue a signal on this thread and interrupt its sleep if needed.
    pub fn post_signal(self: &Arc<Self>, signo: i32) {
        {
            let mut s = self.sig.lock();
            s.pending |= 1u64 << (signo - 1);
        }
        self.interrupt_if_sleeping();
    }
    pub fn interrupt_if_sleeping(self: &Arc<Self>) {
        if self.state() == TaskState::Blocked && self.interruptible.load(Ordering::Acquire) {
            super::wake(self);
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        unsafe { alloc::alloc::dealloc(self.fpu, self.fpu_layout) };
        if !self.idle {
            mm::free_kernel_stack(self.kstack_top, KSTACK_PAGES);
        }
        let _ = PAGE_SIZE;
    }
}
