//! Per-CPU data, reachable through the GS segment base in kernel mode.
//!
//! Layout is `#[repr(C)]` because the syscall entry stub reads the first two
//! fields by fixed offset (0: kernel_rsp, 8: user_rsp scratch, 16: self).

use super::gdt::{Gdt, Tss};
use super::msr;
use crate::sched::task::Task;
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub const MAX_CPUS: usize = 64;

#[repr(C)]
pub struct PerCpu {
    pub kernel_rsp: UnsafeCell<u64>,
    pub user_rsp: UnsafeCell<u64>,
    pub self_ptr: u64,
    pub cpu_id: u32,
    pub lapic_id: u32,
    pub need_resched: AtomicBool,
    pub irq_depth: AtomicU32,
    pub ticks: AtomicU64,
    pub current: UnsafeCell<Option<Arc<Task>>>,
    pub idle: UnsafeCell<Option<Arc<Task>>>,
    pub tss: UnsafeCell<Tss>,
    pub gdt: UnsafeCell<Gdt>,
    pub online: AtomicBool,
}
unsafe impl Sync for PerCpu {}

static CPUS: crate::sync::SpinLock<[Option<&'static PerCpu>; MAX_CPUS]> = crate::sync::SpinLock::new([None; MAX_CPUS]);
static CPU_COUNT: AtomicU32 = AtomicU32::new(0);

impl PerCpu {
    fn new(cpu_id: u32, lapic_id: u32) -> &'static mut PerCpu {
        let pc = Box::leak(Box::new(PerCpu {
            kernel_rsp: UnsafeCell::new(0),
            user_rsp: UnsafeCell::new(0),
            self_ptr: 0,
            cpu_id,
            lapic_id,
            need_resched: AtomicBool::new(false),
            irq_depth: AtomicU32::new(0),
            ticks: AtomicU64::new(0),
            current: UnsafeCell::new(None),
            idle: UnsafeCell::new(None),
            tss: UnsafeCell::new(Tss::new()),
            gdt: UnsafeCell::new(Gdt::new()),
            online: AtomicBool::new(false),
        }));
        pc.self_ptr = pc as *const PerCpu as u64;
        pc
    }

    /// Set up descriptor tables and GS base on the calling CPU.
    unsafe fn install(&'static self) {
        unsafe {
            let tss = &mut *self.tss.get();
            // interrupt stacks for #DF and NMI
            for i in 0..2 {
                let stack = crate::mm::alloc_kernel_stack(4);
                tss.ist[i] = stack as u64;
            }
            let gdt = &mut *self.gdt.get();
            gdt.set_tss(tss);
            (&*self.gdt.get()).load();
            msr::write(msr::IA32_GS_BASE, self.self_ptr);
            msr::write(msr::IA32_KERNEL_GS_BASE, 0);
        }
        self.online.store(true, Ordering::Release);
    }

    pub fn set_kernel_stack(&self, top: u64) {
        unsafe {
            *self.kernel_rsp.get() = top;
            (*self.tss.get()).rsp[0] = top;
        }
    }

    pub fn current_task(&self) -> Option<Arc<Task>> {
        unsafe { (*self.current.get()).clone() }
    }
    pub fn current_ref(&self) -> Option<&Arc<Task>> {
        unsafe { (*self.current.get()).as_ref() }
    }
    pub fn set_current(&self, t: Option<Arc<Task>>) -> Option<Arc<Task>> {
        unsafe { core::mem::replace(&mut *self.current.get(), t) }
    }
    pub fn in_irq(&self) -> bool {
        self.irq_depth.load(Ordering::Relaxed) > 0
    }
}

pub fn init_bsp() {
    let lapic_id = (super::cpu::cpuid(1, 0).ebx >> 24) as u32;
    let pc = PerCpu::new(0, lapic_id);
    let pc: &'static PerCpu = pc;
    CPUS.lock()[0] = Some(pc);
    CPU_COUNT.store(1, Ordering::Release);
    unsafe { pc.install() };
}

/// Register and install a per-CPU area for an application processor.
pub fn init_ap(cpu_id: u32, lapic_id: u32) -> &'static PerCpu {
    let pc = PerCpu::new(cpu_id, lapic_id);
    let pc: &'static PerCpu = pc;
    CPUS.lock()[cpu_id as usize] = Some(pc);
    CPU_COUNT.fetch_add(1, Ordering::AcqRel);
    unsafe { pc.install() };
    pc
}

#[inline(always)]
pub fn this() -> &'static PerCpu {
    let p: u64;
    unsafe { core::arch::asm!("mov {}, qword ptr gs:[16]", out(reg) p, options(nostack, preserves_flags, readonly)) };
    unsafe { &*(p as *const PerCpu) }
}

pub fn get(cpu: usize) -> Option<&'static PerCpu> {
    CPUS.lock()[cpu]
}

pub fn count() -> usize {
    CPU_COUNT.load(Ordering::Acquire) as usize
}

pub fn cpu_id() -> u32 {
    this().cpu_id
}
