//! Signal generation and delivery (Linux x86_64 signal frames).

use super::process::{ProcState, Process, SigAction};
use crate::arch::x86_64::interrupts::TrapFrame;
use crate::mm::errno::*;
use crate::mm::user::{copy_from_user, copy_to_user, read_user};
use crate::sched::task::Task;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;

pub const SIGHUP: i32 = 1;
pub const SIGINT: i32 = 2;
pub const SIGQUIT: i32 = 3;
pub const SIGILL: i32 = 4;
pub const SIGTRAP: i32 = 5;
pub const SIGABRT: i32 = 6;
pub const SIGBUS: i32 = 7;
pub const SIGFPE: i32 = 8;
pub const SIGKILL: i32 = 9;
pub const SIGUSR1: i32 = 10;
pub const SIGSEGV: i32 = 11;
pub const SIGUSR2: i32 = 12;
pub const SIGPIPE: i32 = 13;
pub const SIGALRM: i32 = 14;
pub const SIGTERM: i32 = 15;
pub const SIGCHLD: i32 = 17;
pub const SIGCONT: i32 = 18;
pub const SIGSTOP: i32 = 19;
pub const SIGTSTP: i32 = 20;
pub const SIGTTIN: i32 = 21;
pub const SIGTTOU: i32 = 22;
pub const SIGURG: i32 = 23;
pub const SIGWINCH: i32 = 28;
pub const SIGSYS: i32 = 31;

pub const SA_NOCLDSTOP: u64 = 1;
pub const SA_NOCLDWAIT: u64 = 2;
pub const SA_SIGINFO: u64 = 4;
pub const SA_ONSTACK: u64 = 0x0800_0000;
pub const SA_RESTART: u64 = 0x1000_0000;
pub const SA_NODEFER: u64 = 0x4000_0000;
pub const SA_RESETHAND: u64 = 0x8000_0000;
pub const SA_RESTORER: u64 = 0x0400_0000;

pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;

const KILL_MASK: u64 = (1 << (SIGKILL - 1)) | (1 << (SIGSTOP - 1));

#[derive(Clone, Copy, PartialEq, Eq)]
enum Default {
    Terminate,
    Core,
    Ignore,
    Stop,
    Continue,
}

fn default_action(signo: i32) -> Default {
    match signo {
        SIGCHLD | SIGURG | SIGWINCH => Default::Ignore,
        SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => Default::Stop,
        SIGCONT => Default::Continue,
        SIGQUIT | SIGILL | SIGTRAP | SIGABRT | SIGBUS | SIGFPE | SIGSEGV | SIGSYS | 24 | 25 => Default::Core,
        _ => Default::Terminate,
    }
}

pub fn sig_bit(signo: i32) -> u64 {
    1u64 << (signo - 1)
}

/// Queue `signo` for the process, choosing a thread that does not block it.
pub fn send(p: &Arc<Process>, signo: i32) {
    if signo <= 0 || signo > 64 || p.pid == 0 {
        return;
    }
    if p.is_zombie() {
        return;
    }
    let action = p.sighand().get(signo);
    // signals ignored by default or explicitly are dropped early (except SIGCONT bookkeeping)
    if signo == SIGCONT {
        // wake a stopped process
        if p.state() == ProcState::Stopped {
            p.set_state(ProcState::Running);
            p.continued_reported.store(false, Ordering::Relaxed);
            p.stop_wq.wake_all();
            if let Some(parent) = p.parent() {
                parent.child_wq.wake_all();
            }
        }
        // discard pending stop signals
        let stopmask = sig_bit(SIGSTOP) | sig_bit(SIGTSTP) | sig_bit(SIGTTIN) | sig_bit(SIGTTOU);
        p.shared_pending.fetch_and(!stopmask, Ordering::Relaxed);
    }
    if matches!(signo, SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU) {
        p.shared_pending.fetch_and(!sig_bit(SIGCONT), Ordering::Relaxed);
    }
    if action.handler == SIG_IGN && signo != SIGKILL && signo != SIGSTOP {
        return;
    }
    if action.handler == SIG_DFL && default_action(signo) == Default::Ignore {
        return;
    }
    p.shared_pending.fetch_or(sig_bit(signo), Ordering::Relaxed);
    // wake an appropriate thread
    let threads = p.threads.lock().clone();
    let mut target = None;
    for t in &threads {
        if t.signal_mask() & sig_bit(signo) == 0 {
            target = Some(t.clone());
            break;
        }
    }
    let target = target.or_else(|| threads.first().cloned());
    if let Some(t) = target {
        t.interrupt_if_sleeping();
        if signo == SIGKILL {
            for t in &threads {
                t.interrupt_if_sleeping();
            }
        }
    }
}

pub fn send_to_pgrp(pgid: u32, signo: i32) -> usize {
    let mut n = 0;
    for p in super::process::all() {
        if p.pgid() == pgid && !p.is_zombie() {
            send(&p, signo);
            n += 1;
        }
    }
    n
}

pub fn send_to_current(signo: i32) {
    if let Some(t) = crate::sched::try_current() {
        t.post_signal(signo);
    }
}

/// Signal a specific thread (tgkill).
pub fn send_to_task(t: &Arc<Task>, signo: i32) {
    if signo <= 0 || signo > 64 {
        return;
    }
    let action = t.proc.sighand().get(signo);
    if action.handler == SIG_IGN && signo != SIGKILL && signo != SIGSTOP {
        return;
    }
    if action.handler == SIG_DFL && default_action(signo) == Default::Ignore {
        return;
    }
    t.post_signal(signo);
}

// ---- delivery ----------------------------------------------------------------

/// Linux `struct sigcontext` / `mcontext` layout.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SigContext {
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rbx: u64,
    rdx: u64,
    rax: u64,
    rcx: u64,
    rsp: u64,
    rip: u64,
    eflags: u64,
    cs: u16,
    gs: u16,
    fs: u16,
    ss: u16,
    err: u64,
    trapno: u64,
    oldmask: u64,
    cr2: u64,
    fpstate: u64,
    reserved: [u64; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct StackT {
    ss_sp: u64,
    ss_flags: i32,
    _pad: i32,
    ss_size: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct UContext {
    uc_flags: u64,
    uc_link: u64,
    uc_stack: StackT,
    uc_mcontext: SigContext,
    uc_sigmask: u64,
    _pad: [u64; 15],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SigInfo {
    si_signo: i32,
    si_errno: i32,
    si_code: i32,
    _pad0: i32,
    fields: [u64; 14],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RtSigFrame {
    pretcode: u64,
    uc: UContext,
    info: SigInfo,
}

/// Pick the next deliverable signal for the current task.
fn dequeue(t: &Arc<Task>) -> Option<i32> {
    let mut s = t.sig.lock();
    let mask = s.mask;
    let mut pending = s.pending & !mask;
    let mut shared = false;
    if pending == 0 {
        pending = t.proc.shared_pending() & !mask;
        shared = true;
    }
    if pending == 0 {
        return None;
    }
    // SIGKILL first, then synchronous faults, then lowest-numbered
    let signo = if pending & sig_bit(SIGKILL) != 0 {
        SIGKILL
    } else {
        let sync_mask = sig_bit(SIGSEGV) | sig_bit(SIGBUS) | sig_bit(SIGILL) | sig_bit(SIGFPE) | sig_bit(SIGTRAP);
        let cand = if pending & sync_mask != 0 { pending & sync_mask } else { pending };
        cand.trailing_zeros() as i32 + 1
    };
    if shared {
        t.proc.shared_pending.fetch_and(!sig_bit(signo), Ordering::Relaxed);
    } else {
        s.pending &= !sig_bit(signo);
    }
    Some(signo)
}

/// Deliver pending signals to the current task. Returns true if anything was done.
pub fn deliver_pending(frame: &mut TrapFrame) -> bool {
    let t = crate::sched::current();
    if t.is_kernel_thread() {
        return false;
    }
    let mut did = false;
    // process stopped by job control?
    if t.proc.state() == ProcState::Stopped {
        t.proc.stop_wq.wait_until_uninterruptible(|| t.proc.state() != ProcState::Stopped || t.proc.group_exit.load(Ordering::Relaxed));
        did = true;
    }
    loop {
        let signo = match dequeue(&t) {
            Some(s) => s,
            None => break,
        };
        did = true;
        let action = t.proc.sighand().get(signo);
        if action.handler == SIG_IGN && signo != SIGKILL && signo != SIGSTOP {
            continue;
        }
        if action.handler == SIG_DFL || signo == SIGKILL || signo == SIGSTOP {
            match default_action(signo) {
                Default::Ignore => continue,
                Default::Continue => continue,
                Default::Stop => {
                    do_stop(&t, signo);
                    continue;
                }
                Default::Terminate | Default::Core => {
                    restart_syscall_check(&t, frame, None);
                    super::exit::exit_group_signal(signo);
                }
            }
        }
        // user handler
        restart_syscall_check(&t, frame, Some(&action));
        if setup_frame(&t, frame, signo, &action).is_err() {
            // cannot build the frame: kill with SIGSEGV
            super::exit::exit_group_signal(SIGSEGV);
        }
        if action.flags & SA_RESETHAND != 0 {
            t.proc.sighand().set(signo, SigAction::DEFAULT);
        }
        // only one handler per return to user space
        break;
    }
    // restore the mask saved by sigsuspend once a handler ran
    if did {
        let saved = t.saved_mask.swap(u64::MAX, Ordering::Relaxed);
        if saved != u64::MAX {
            t.sig.lock().mask = saved & !KILL_MASK;
        }
    } else {
        // a syscall interrupted with no signal actually delivered: restart it
        restart_syscall_check(&t, frame, None);
    }
    did
}

/// Handle -ERESTARTSYS in the syscall return value.
fn restart_syscall_check(t: &Arc<Task>, frame: &mut TrapFrame, action: Option<&SigAction>) {
    if !frame.is_syscall() {
        return;
    }
    if frame.rax != ERESTARTSYS.as_ret() {
        return;
    }
    let restart = match action {
        None => true,
        Some(a) => a.flags & SA_RESTART != 0,
    };
    if restart {
        frame.rax = t.syscall_nr.load(Ordering::Relaxed);
        frame.rip -= 2; // re-execute the `syscall` instruction
    } else {
        frame.rax = EINTR.as_ret();
    }
}

fn do_stop(t: &Arc<Task>, signo: i32) {
    let p = &t.proc;
    p.set_state(ProcState::Stopped);
    p.stop_signal.store(signo, Ordering::Relaxed);
    p.stopped_reported.store(false, Ordering::Relaxed);
    if let Some(parent) = p.parent() {
        let pa = parent.sighand().get(SIGCHLD);
        if pa.flags & SA_NOCLDSTOP == 0 {
            send(&parent, SIGCHLD);
        }
        parent.child_wq.wake_all();
    }
    p.stop_wq.wait_until_uninterruptible(|| p.state() != ProcState::Stopped || p.group_exit.load(Ordering::Relaxed));
}

fn setup_frame(t: &Arc<Task>, frame: &mut TrapFrame, signo: i32, action: &SigAction) -> Result<()> {
    let xsave_size = crate::arch::x86_64::cpu::features().xsave_size as usize;
    let (altstack_sp, altstack_size, altstack_flags, oldmask) = {
        let s = t.sig.lock();
        (s.altstack_sp, s.altstack_size, s.altstack_flags, s.mask)
    };
    let on_altstack = altstack_flags & 2 == 0 && altstack_size != 0 && !(frame.rsp > altstack_sp && frame.rsp <= altstack_sp + altstack_size);
    let mut sp = if action.flags & SA_ONSTACK != 0 && on_altstack { altstack_sp + altstack_size } else { frame.rsp - 128 };
    // fpstate area (64-byte aligned)
    sp = (sp - xsave_size as u64) & !63;
    let fpstate = sp;
    // the frame itself: 16-byte aligned minus 8
    let frame_size = core::mem::size_of::<RtSigFrame>() as u64;
    sp = ((sp - frame_size) & !15) - 8;
    let frame_addr = sp;

    // save the live FPU state (the kernel does not touch it)
    crate::mm::user::access_ok(fpstate as usize, xsave_size, true)?;
    unsafe { crate::arch::x86_64::cpu::xsave(fpstate as *mut u8) };

    let mut sf = RtSigFrame {
        pretcode: if action.flags & SA_RESTORER != 0 { action.restorer } else { 0 },
        uc: UContext::default(),
        info: SigInfo { si_signo: signo, si_errno: 0, si_code: 0, _pad0: 0, fields: [0; 14] },
    };
    sf.uc.uc_flags = 0;
    sf.uc.uc_stack = StackT { ss_sp: altstack_sp, ss_flags: altstack_flags as i32, _pad: 0, ss_size: altstack_size };
    let mc = &mut sf.uc.uc_mcontext;
    mc.r8 = frame.r8;
    mc.r9 = frame.r9;
    mc.r10 = frame.r10;
    mc.r11 = frame.r11;
    mc.r12 = frame.r12;
    mc.r13 = frame.r13;
    mc.r14 = frame.r14;
    mc.r15 = frame.r15;
    mc.rdi = frame.rdi;
    mc.rsi = frame.rsi;
    mc.rbp = frame.rbp;
    mc.rbx = frame.rbx;
    mc.rdx = frame.rdx;
    mc.rax = frame.rax;
    mc.rcx = frame.rcx;
    mc.rsp = frame.rsp;
    mc.rip = frame.rip;
    mc.eflags = frame.rflags;
    mc.cs = frame.cs as u16;
    mc.ss = frame.ss as u16;
    mc.err = frame.error;
    mc.trapno = if frame.vector == u64::MAX { 0 } else { frame.vector };
    mc.oldmask = oldmask;
    mc.cr2 = t.fault_addr.load(Ordering::Relaxed);
    mc.fpstate = fpstate;
    sf.uc.uc_sigmask = oldmask;
    // siginfo details for faults
    if matches!(signo, SIGSEGV | SIGBUS | SIGILL | SIGFPE | SIGTRAP) {
        sf.info.si_code = t.fault_code.load(Ordering::Relaxed);
        sf.info.fields[0] = t.fault_addr.load(Ordering::Relaxed);
    } else if signo == SIGCHLD {
        sf.info.si_code = 1; // CLD_EXITED
    } else {
        sf.info.si_code = 0; // SI_USER
        sf.info.fields[0] = (t.proc.pid as u64) | ((t.proc.uid.load(Ordering::Relaxed) as u64) << 32);
    }
    let bytes = unsafe { core::slice::from_raw_parts(&sf as *const _ as *const u8, frame_size as usize) };
    copy_to_user(frame_addr as usize, bytes)?;

    // new signal mask while the handler runs
    {
        let mut s = t.sig.lock();
        let mut m = s.mask | action.mask;
        if action.flags & SA_NODEFER == 0 {
            m |= sig_bit(signo);
        }
        s.mask = m & !KILL_MASK;
    }

    frame.rsp = frame_addr;
    frame.rip = action.handler;
    frame.rdi = signo as u64;
    frame.rsi = frame_addr + 8 + core::mem::size_of::<UContext>() as u64; // &info
    frame.rdx = frame_addr + 8; // &uc
    frame.rax = 0;
    frame.rflags &= !(1 << 10); // clear DF
    frame.cs = crate::arch::x86_64::gdt::USER_CS as u64;
    frame.ss = crate::arch::x86_64::gdt::USER_DS as u64;
    Ok(())
}

/// rt_sigreturn: restore the context saved by `setup_frame`.
pub fn sigreturn(frame: &mut TrapFrame) -> Result<u64> {
    let t = crate::sched::current();
    let uc_addr = frame.rsp as usize; // pretcode was popped by `ret`
    let mut uc = UContext::default();
    let bytes = unsafe { core::slice::from_raw_parts_mut(&mut uc as *mut _ as *mut u8, core::mem::size_of::<UContext>()) };
    copy_from_user(bytes, uc_addr)?;
    let mc = &uc.uc_mcontext;
    frame.r8 = mc.r8;
    frame.r9 = mc.r9;
    frame.r10 = mc.r10;
    frame.r11 = mc.r11;
    frame.r12 = mc.r12;
    frame.r13 = mc.r13;
    frame.r14 = mc.r14;
    frame.r15 = mc.r15;
    frame.rdi = mc.rdi;
    frame.rsi = mc.rsi;
    frame.rbp = mc.rbp;
    frame.rbx = mc.rbx;
    frame.rdx = mc.rdx;
    frame.rax = mc.rax;
    frame.rcx = mc.rcx;
    frame.rsp = mc.rsp;
    frame.rip = mc.rip;
    // only allow user-modifiable flags
    const FLAG_MASK: u64 = 0xcd5 | (1 << 10);
    frame.rflags = (mc.eflags & FLAG_MASK) | 0x202;
    frame.cs = crate::arch::x86_64::gdt::USER_CS as u64;
    frame.ss = crate::arch::x86_64::gdt::USER_DS as u64;
    {
        let mut s = t.sig.lock();
        s.mask = uc.uc_sigmask & !KILL_MASK;
    }
    if mc.fpstate != 0 {
        let size = crate::arch::x86_64::cpu::features().xsave_size as usize;
        if crate::mm::user::access_ok(mc.fpstate as usize, size, false).is_ok() && mc.fpstate & 63 == 0 {
            // validate the xsave header to avoid #GP on xrstor
            let xstate_bv: u64 = read_user(mc.fpstate as usize + 512)?;
            let xcomp_bv: u64 = read_user(mc.fpstate as usize + 520)?;
            let xcr0 = crate::arch::x86_64::cpu::features().xcr0;
            if xstate_bv & !xcr0 == 0 && xcomp_bv == 0 {
                unsafe { crate::arch::x86_64::cpu::xrstor(mc.fpstate as *const u8) };
            }
        }
    }
    // the value in rax is the interrupted context's; return it unchanged
    Ok(frame.rax)
}

// ---- faults -------------------------------------------------------------------

/// A user-mode exception: queue the matching signal (forced if blocked/ignored).
pub fn fault_signal(frame: &mut TrapFrame, signo: i32, addr: usize, vector: u64) {
    let t = crate::sched::current();
    t.fault_addr.store(addr as u64, Ordering::Relaxed);
    let code = match vector {
        14 => {
            if frame.error & 1 != 0 {
                2
            } else {
                1
            }
        } // SEGV_ACCERR / SEGV_MAPERR
        _ => 0x80,
    };
    t.fault_code.store(code, Ordering::Relaxed);
    let action = t.proc.sighand().get(signo);
    let blocked = t.signal_mask() & sig_bit(signo) != 0;
    if action.handler == SIG_IGN || blocked {
        // force default: terminate
        t.proc.sighand().set(signo, SigAction::DEFAULT);
        let mut s = t.sig.lock();
        s.mask &= !sig_bit(signo);
    }
    if crate::proc::process::lookup(t.proc.pid).is_some() {
        if signo == SIGSEGV || signo == SIGBUS || signo == SIGILL || signo == SIGFPE {
            klog!("proc", "pid {} '{}' fault: sig {} at {:#x} rip={:#x} err={:#x}", t.proc.pid, t.proc.comm(), signo, addr, frame.rip, frame.error);
        }
    }
    t.post_signal(signo);
}

/// The kernel touched an unmapped user address while executing on behalf of a task.
pub fn kernel_fault_on_user_addr(frame: &mut TrapFrame, addr: usize) -> ! {
    crate::console::print_unlocked(format_args!("kernel access to unmapped user address {:#x} at rip={:#x}\n", addr, frame.rip));
    crate::arch::x86_64::interrupts::dump_frame(frame);
    panic!("unhandled kernel-mode fault on user memory");
}

/// Should a process with these pending signals stop blocking?
pub fn is_fatal_pending(t: &Task) -> bool {
    let p = t.pending_unblocked();
    p & sig_bit(SIGKILL) != 0
}

pub fn ignored_or_default_ignore(p: &Process, signo: i32) -> bool {
    let a = p.sighand().get(signo);
    a.handler == SIG_IGN || (a.handler == SIG_DFL && default_action(signo) == Default::Ignore)
}
