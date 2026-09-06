//! Networking: unix domain sockets now; IP later.

pub mod unix;

use crate::mm::errno::*;

pub fn syscall(nr: u64, a: [u64; 6]) -> Result<u64> {
    unix::syscall(nr, a)
}
