use crate::arch::x86_64::interrupts::TrapFrame;

pub fn fault_signal(frame: &mut TrapFrame, signo: i32, addr: usize, vec: usize) {
    crate::arch::x86_64::interrupts::dump_frame(frame);
    panic!("user fault: signal {} at {:#x} (vector {})", signo, addr, vec);
}

pub fn kernel_fault_on_user_addr(frame: &mut TrapFrame, addr: usize) {
    crate::arch::x86_64::interrupts::dump_frame(frame);
    panic!("kernel fault on user address {:#x}", addr);
}
