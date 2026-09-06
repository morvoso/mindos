//! Linux-compatible system call layer.

use crate::arch::x86_64::interrupts::TrapFrame;

#[no_mangle]
extern "C" fn syscall_dispatch(frame: &mut TrapFrame) {
    frame.rax = (-38i64) as u64; // ENOSYS
}
