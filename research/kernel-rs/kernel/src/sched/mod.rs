//! Scheduler: tasks, run queue, context switching, wait queues, sleeping.
//!
//! The kernel is non-preemptible: a task switches only when it blocks or on
//! return to user mode when the timer has marked `need_resched`.

pub mod mutex;
pub mod task;
pub mod timer;
pub mod wait;

use crate::arch::x86_64::interrupts::TrapFrame;
use crate::arch::x86_64::{cpu, msr, percpu};
use crate::sync::{irq_restore, irq_save, SpinLock};
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use task::{Task, TaskState};

pub static TICKS: AtomicU64 = AtomicU64::new(0);
pub const DEFAULT_SLICE: u32 = 5; // ticks (20 ms at 250 Hz)

struct RunQueue {
    queue: VecDeque<Arc<Task>>,
}

static RUNQ: SpinLock<RunQueue> = SpinLock::new(RunQueue { queue: VecDeque::new() });
/// Task whose reference must be dropped after the next context switch completes
/// (it may be a zombie whose stack we were running on).
static PREV_TO_DROP: SpinLock<Option<Arc<Task>>> = SpinLock::new(None);

extern "C" {
    fn context_switch(prev_rsp: *mut u64, next_rsp: u64);
    fn task_trampoline_entry();
}

core::arch::global_asm!(
    r#"
.section .text
.global context_switch
context_switch:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    mov [rdi], rsp
    mov rsp, rsi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret

.global task_trampoline_entry
task_trampoline_entry:
    call task_trampoline
    ud2
"#
);

/// First code run by every new task, on its own stack.
#[no_mangle]
extern "C" fn task_trampoline() -> ! {
    finish_switch();
    crate::arch::x86_64::enable_interrupts();
    let t = current();
    let entry = t.take_entry();
    match entry {
        Some((f, arg)) => {
            f(arg);
            exit_current_task(0);
        }
        None => {
            // user task created by clone/fork: return through its trap frame
            t.on_first_user_entry();
            let frame = t.user_frame_ptr();
            drop(t);
            unsafe { crate::arch::x86_64::interrupts::enter_user_frame(frame) }
        }
    }
}

/// Drop the previous task reference handed over by `schedule` (runs on the new stack).
pub fn finish_switch() {
    let prev = PREV_TO_DROP.lock().take();
    drop(prev);
}

pub fn init() {
    // The boot context becomes the idle task of the BSP.
    let idle = Task::new_idle(0);
    let pc = percpu::this();
    pc.set_kernel_stack(idle.kstack_top as u64);
    unsafe {
        *pc.idle.get() = Some(idle.clone());
    }
    pc.set_current(Some(idle));
    timer::init();
}

pub fn current() -> Arc<Task> {
    percpu::this().current_task().expect("no current task")
}
pub fn try_current() -> Option<Arc<Task>> {
    if !percpu::this().online.load(Ordering::Relaxed) {
        return None;
    }
    percpu::this().current_task()
}
/// Borrow the current task without touching the refcount.
pub fn current_ref() -> &'static Arc<Task> {
    percpu::this().current_ref().expect("no current task")
}
pub fn current_proc() -> Arc<crate::proc::process::Process> {
    current_ref().proc.clone()
}

/// Make a task runnable (idempotent).
pub fn wake(t: &Arc<Task>) {
    let mut rq = RUNQ.lock();
    let st = t.state();
    if st == TaskState::Zombie {
        return;
    }
    t.set_state(TaskState::Runnable);
    if !t.on_rq.swap(true, Ordering::AcqRel) && !t.is_current() {
        rq.queue.push_back(t.clone());
    } else if !t.is_current() && !rq.queue.iter().any(|x| Arc::ptr_eq(x, t)) {
        // on_rq was set but the task is not queued (was current when marked): enqueue
        rq.queue.push_back(t.clone());
    }
}

pub fn enqueue(t: Arc<Task>) {
    let mut rq = RUNQ.lock();
    t.set_state(TaskState::Runnable);
    t.on_rq.store(true, Ordering::Release);
    rq.queue.push_back(t);
}

fn pick_next() -> Option<Arc<Task>> {
    let mut rq = RUNQ.lock();
    while let Some(t) = rq.queue.pop_front() {
        t.on_rq.store(false, Ordering::Release);
        if t.state() == TaskState::Runnable {
            return Some(t);
        }
    }
    None
}

/// Give up the CPU. The current task must already be in the desired state
/// (Runnable to be re-queued, Blocked to sleep, Zombie to die).
pub fn schedule() {
    let flags = irq_save();
    let pc = percpu::this();
    let cur = pc.current_task().expect("schedule without current");
    if cur.state() == TaskState::Running {
        cur.set_state(TaskState::Runnable);
    }
    if cur.state() == TaskState::Runnable && !cur.is_idle() {
        let mut rq = RUNQ.lock();
        if !cur.on_rq.swap(true, Ordering::AcqRel) {
            rq.queue.push_back(cur.clone());
        }
    }
    let next = match pick_next() {
        Some(t) => t,
        None => {
            if cur.state() == TaskState::Runnable {
                // nothing else to run: keep going
                cur.on_rq.store(false, Ordering::Release);
                {
                    let mut rq = RUNQ.lock();
                    rq.queue.retain(|t| !Arc::ptr_eq(t, &cur));
                }
                cur.set_state(TaskState::Running);
                pc.need_resched.store(false, Ordering::Relaxed);
                irq_restore(flags);
                return;
            }
            unsafe { (*pc.idle.get()).clone().expect("no idle task") }
        }
    };
    if Arc::ptr_eq(&next, &cur) {
        cur.set_state(TaskState::Running);
        pc.need_resched.store(false, Ordering::Relaxed);
        irq_restore(flags);
        return;
    }
    next.set_state(TaskState::Running);
    next.slice.store(DEFAULT_SLICE, Ordering::Relaxed);
    pc.need_resched.store(false, Ordering::Relaxed);
    unsafe { switch_to(pc, &cur, next) };
    // back on `cur`'s stack
    finish_switch();
    irq_restore(flags);
}

unsafe fn switch_to(pc: &percpu::PerCpu, prev: &Arc<Task>, next: Arc<Task>) {
    let now = crate::arch::x86_64::tsc::uptime_ns();
    prev.account_switch_out(now);
    next.account_switch_in(now);
    // extended (FPU/SIMD) state belongs to user tasks only, but saving is harmless
    unsafe { cpu::xsave(prev.fpu_area()) };
    pc.set_kernel_stack(next.kstack_top as u64);
    msr::write(msr::IA32_FS_BASE, next.fs_base.load(Ordering::Relaxed));
    // address space
    let next_pml4 = next.proc.pml4();
    if cpu::read_cr3() & crate::mm::vmm::ADDR_MASK != next_pml4 {
        cpu::write_cr3(next_pml4);
    }
    unsafe { cpu::xrstor(next.fpu_area()) };
    let prev_ctx = prev.ctx_ptr();
    let next_rsp = unsafe { (*next.ctx_ptr()).rsp };
    let old = pc.set_current(Some(next));
    *PREV_TO_DROP.lock() = old;
    unsafe { context_switch(prev_ctx as *mut u64, next_rsp) };
}

/// Voluntarily let other runnable tasks go first.
pub fn yield_now() {
    let cur = current();
    cur.set_state(TaskState::Runnable);
    schedule();
}

/// Timer interrupt: accounting, time slices and sleep timeouts.
pub fn timer_tick(_frame: &mut TrapFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    let pc = percpu::this();
    pc.ticks.fetch_add(1, Ordering::Relaxed);
    if let Some(cur) = pc.current_ref() {
        if !cur.is_idle() {
            let s = cur.slice.load(Ordering::Relaxed);
            if s <= 1 {
                pc.need_resched.store(true, Ordering::Relaxed);
            } else {
                cur.slice.store(s - 1, Ordering::Relaxed);
            }
        }
    }
    timer::tick();
}

pub fn need_resched() -> bool {
    percpu::this().need_resched.load(Ordering::Relaxed)
}
pub fn set_need_resched() {
    percpu::this().need_resched.store(true, Ordering::Relaxed);
}

/// Called on every return to user mode: reschedule if needed, then deliver signals.
pub fn return_to_user(frame: &mut TrapFrame) {
    loop {
        if need_resched() {
            yield_now();
        }
        if crate::proc::signal::deliver_pending(frame) {
            continue; // delivering may have changed things; re-check
        }
        if !need_resched() {
            break;
        }
    }
}

/// Terminate the current task. Never returns.
pub fn exit_current_task(code: i32) -> ! {
    let cur = current();
    crate::proc::exit::task_exit(&cur, code);
    drop(cur);
    crate::arch::x86_64::disable_interrupts();
    current_ref().set_state(TaskState::Zombie);
    schedule();
    unreachable!("zombie task resumed");
}

/// Spawn a kernel thread.
pub fn spawn_kernel(name: &str, f: fn(usize), arg: usize) -> Arc<Task> {
    let t = Task::new_kernel(name, f, arg);
    enqueue(t.clone());
    t
}

/// The idle loop for the boot CPU (runs as the idle task).
pub fn idle_loop() -> ! {
    loop {
        if need_resched() || !RUNQ.lock().queue.is_empty() {
            let cur = current();
            cur.set_state(TaskState::Runnable);
            schedule();
        }
        crate::arch::x86_64::idle_wait();
    }
}

pub fn panic_dump_current() {
    if let Some(t) = try_current() {
        crate::console::print_unlocked(format_args!("current task: tid {} pid {} '{}'\n", t.tid, t.proc.pid, t.proc.comm()));
    }
}

pub fn runnable_count() -> usize {
    RUNQ.lock().queue.len()
}
