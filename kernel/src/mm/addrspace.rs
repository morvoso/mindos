//! User address spaces (filled in with the process layer).

use crate::arch::x86_64::interrupts::TrapFrame;

/// Try to resolve a page fault in user memory. Returns true if handled.
pub fn handle_page_fault(_addr: usize, _error: u64, _frame: &mut TrapFrame) -> bool {
    false
}
