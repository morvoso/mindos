use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

static PANICKING: AtomicBool = AtomicBool::new(false);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { core::arch::asm!("cli") };
    if PANICKING.swap(true, Ordering::SeqCst) {
        // Nested panic: just stop.
        loop {
            unsafe { core::arch::asm!("hlt") };
        }
    }
    crate::console::print_unlocked(format_args!("\n\n*** KERNEL PANIC ***\n"));
    if let Some(loc) = info.location() {
        crate::console::print_unlocked(format_args!("at {}:{}:{}\n", loc.file(), loc.line(), loc.column()));
    }
    crate::console::print_unlocked(format_args!("{}\n", info.message()));
    crate::sched::panic_dump_current();
    crate::console::print_unlocked(format_args!("system halted.\n"));
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}
