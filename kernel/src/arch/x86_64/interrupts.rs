//! Interrupt and syscall entry/exit, the trap frame, exception handling
//! and the IRQ handler registry.

use super::cpu;
use super::lapic;
use super::percpu;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Register state saved on every entry into the kernel. Layout must match the
/// assembly below (pushed in reverse order after the CPU-provided iret frame).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl TrapFrame {
    pub fn from_user(&self) -> bool {
        self.cs & 3 == 3
    }
    pub fn is_syscall(&self) -> bool {
        self.vector == u64::MAX
    }
    /// Frame for a brand new user thread.
    pub fn new_user(rip: u64, rsp: u64) -> Self {
        TrapFrame {
            rip,
            rsp,
            cs: super::gdt::USER_CS as u64,
            ss: super::gdt::USER_DS as u64,
            rflags: 0x202,
            ..Default::default()
        }
    }
}

core::arch::global_asm!(include_str!(concat!(env!("OUT_DIR"), "/isr.S")));

core::arch::global_asm!(
    r#"
.section .text
.global isr_common
isr_common:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    test qword ptr [rsp + 144], 3
    jz 1f
    swapgs
1:
    cld
    mov rdi, rsp
    call interrupt_dispatch
.global trap_return
trap_return:
    test qword ptr [rsp + 144], 3
    jz 2f
    mov rdi, rsp
    call return_to_user_hook
    swapgs
2:
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    add rsp, 16
    iretq

.global syscall_entry
syscall_entry:
    swapgs
    mov qword ptr gs:[8], rsp
    mov rsp, qword ptr gs:[0]
    push 0x1b
    push qword ptr gs:[8]
    push r11
    push 0x23
    push rcx
    push 0
    push -1
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    cld
    mov rdi, rsp
    call syscall_dispatch
    mov rdi, rsp
    call return_to_user_hook
    mov rax, [rsp + 136]
    shr rax, 47
    jnz 3f
    or qword ptr [rsp + 152], 0x200
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    add rsp, 16
    mov rcx, [rsp]
    mov r11, [rsp + 16]
    mov rsp, [rsp + 24]
    swapgs
    sysretq
3:
    swapgs
    jmp 2b

/* Jump into user mode with a prepared trap frame (new task / exec). */
.global enter_user_frame
enter_user_frame:
    mov rsp, rdi
    jmp trap_return
"#
);

extern "C" {
    pub fn syscall_entry();
    pub fn enter_user_frame(frame: *const TrapFrame) -> !;
}

pub fn init_syscall_msrs() {
    use super::msr;
    msr::write(msr::IA32_EFER, msr::read(msr::IA32_EFER) | 1); // SCE
    // STAR: [47:32] kernel CS/SS base, [63:48] user base (sysret: CS = base+16, SS = base+8)
    msr::write(msr::IA32_STAR, ((0x10u64) << 48) | ((super::gdt::KERNEL_CS as u64) << 32));
    msr::write(msr::IA32_LSTAR, syscall_entry as *const () as usize as u64);
    // mask IF, TF, DF, AC, NT, RF on entry
    msr::write(msr::IA32_FMASK, 0x200 | 0x100 | 0x400 | 0x4_0000 | 0x4000 | 0x1_0000);
}

pub type IrqHandler = fn(&mut TrapFrame);

static IRQ_HANDLERS: [AtomicUsize; 256] = [const { AtomicUsize::new(0) }; 256];
static SPURIOUS_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn register_handler(vector: u8, handler: IrqHandler) {
    IRQ_HANDLERS[vector as usize].store(handler as usize, Ordering::Release);
}

const EXCEPTION_NAMES: [&str; 32] = [
    "#DE divide error", "#DB debug", "NMI", "#BP breakpoint", "#OF overflow", "#BR bound range", "#UD invalid opcode",
    "#NM device not available", "#DF double fault", "coprocessor segment overrun", "#TS invalid TSS", "#NP segment not present",
    "#SS stack fault", "#GP general protection", "#PF page fault", "reserved", "#MF x87 FP", "#AC alignment check",
    "#MC machine check", "#XM SIMD FP", "#VE virtualization", "#CP control protection", "reserved", "reserved", "reserved",
    "reserved", "reserved", "reserved", "#HV hypervisor injection", "#VC VMM communication", "#SX security", "reserved",
];

#[no_mangle]
extern "C" fn interrupt_dispatch(frame: &mut TrapFrame) {
    let pc = percpu::this();
    let vec = frame.vector as usize;
    if vec < 32 {
        handle_exception(frame);
        return;
    }
    pc.irq_depth.fetch_add(1, Ordering::Relaxed);
    if vec == super::VEC_SPURIOUS as usize {
        SPURIOUS_COUNT.fetch_add(1, Ordering::Relaxed);
    } else {
        let h = IRQ_HANDLERS[vec].load(Ordering::Acquire);
        if h != 0 {
            let f: IrqHandler = unsafe { core::mem::transmute(h) };
            f(frame);
        } else {
            klog!("irq", "unexpected interrupt vector {:#x}", vec);
        }
        lapic::eoi();
    }
    pc.irq_depth.fetch_sub(1, Ordering::Relaxed);
}

fn handle_exception(frame: &mut TrapFrame) {
    let vec = frame.vector as usize;
    if vec == 14 {
        let addr = cpu::read_cr2();
        // User-space faults (and kernel faults on user addresses during syscalls)
        // are handled by the memory manager; only unrecoverable ones reach here.
        if addr < crate::mm::USER_TOP as u64 || frame.from_user() {
            if frame.from_user() {
                super::enable_interrupts();
            }
            if crate::mm::addrspace::handle_page_fault(addr as usize, frame.error, frame) {
                return;
            }
            if frame.from_user() {
                crate::proc::signal::fault_signal(frame, 11 /* SIGSEGV */, addr as usize, vec);
                return;
            }
            crate::proc::signal::kernel_fault_on_user_addr(frame, addr as usize);
            // not reached if the process was killed
        }
        dump_and_panic(frame, Some(addr));
    }
    if vec == 2 {
        klog!("nmi", "non-maskable interrupt received");
        return;
    }
    if frame.from_user() {
        let signo = match vec {
            0 | 16 | 19 => 8,      // SIGFPE
            1 | 3 => 5,            // SIGTRAP
            6 => 4,                // SIGILL
            13 | 11 | 12 | 10 => 11, // SIGSEGV (GP and segment faults)
            17 => 7,               // SIGBUS
            _ => 11,
        };
        if vec == 13 || vec == 6 {
            klog!("trap", "{} in user task at rip={:#x} err={:#x}", EXCEPTION_NAMES[vec], frame.rip, frame.error);
        }
        super::enable_interrupts();
        crate::proc::signal::fault_signal(frame, signo, frame.rip as usize, vec);
        return;
    }
    dump_and_panic(frame, None);
}

pub fn dump_frame(frame: &TrapFrame) {
    let p = |a: core::fmt::Arguments| crate::console::print_unlocked(a);
    p(format_args!("rip={:#018x} cs={:#x} rflags={:#x} rsp={:#018x} ss={:#x}\n", frame.rip, frame.cs, frame.rflags, frame.rsp, frame.ss));
    p(format_args!("rax={:#018x} rbx={:#018x} rcx={:#018x} rdx={:#018x}\n", frame.rax, frame.rbx, frame.rcx, frame.rdx));
    p(format_args!("rsi={:#018x} rdi={:#018x} rbp={:#018x} r8 ={:#018x}\n", frame.rsi, frame.rdi, frame.rbp, frame.r8));
    p(format_args!("r9 ={:#018x} r10={:#018x} r11={:#018x} r12={:#018x}\n", frame.r9, frame.r10, frame.r11, frame.r12));
    p(format_args!("r13={:#018x} r14={:#018x} r15={:#018x} cr3={:#x}\n", frame.r13, frame.r14, frame.r15, cpu::read_cr3()));
}

fn dump_and_panic(frame: &TrapFrame, fault_addr: Option<u64>) -> ! {
    super::disable_interrupts();
    let vec = frame.vector as usize;
    crate::console::print_unlocked(format_args!("\n*** CPU EXCEPTION {} ({}) error={:#x} ***\n", vec, EXCEPTION_NAMES.get(vec).unwrap_or(&"?"), frame.error));
    if let Some(a) = fault_addr {
        let e = frame.error;
        crate::console::print_unlocked(format_args!(
            "page fault at {:#x}: {} {} {}{}{}\n",
            a,
            if e & 4 != 0 { "user" } else { "kernel" },
            if e & 2 != 0 { "write" } else { "read" },
            if e & 1 != 0 { "protection-violation" } else { "not-present" },
            if e & 16 != 0 { " instruction-fetch" } else { "" },
            if e & 8 != 0 { " reserved-bit" } else { "" }
        ));
    }
    dump_frame(frame);
    backtrace(frame.rbp);
    panic!("unhandled CPU exception {}", vec);
}

/// Walk frame pointers (the kernel is built with frame pointers enabled).
pub fn backtrace(mut rbp: u64) {
    crate::console::print_unlocked(format_args!("backtrace:\n"));
    for _ in 0..24 {
        if rbp < crate::mm::KERNEL_BASE as u64 || rbp & 7 != 0 {
            break;
        }
        let ret = unsafe { *((rbp + 8) as *const u64) };
        if ret == 0 {
            break;
        }
        crate::console::print_unlocked(format_args!("  {:#018x}\n", ret));
        rbp = unsafe { *(rbp as *const u64) };
    }
}

/// Called on every return to user mode (after syscalls, interrupts, faults).
#[no_mangle]
extern "C" fn return_to_user_hook(frame: &mut TrapFrame) {
    crate::sched::return_to_user(frame);
}
