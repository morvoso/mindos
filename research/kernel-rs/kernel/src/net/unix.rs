//! AF_UNIX sockets (placeholder until the socket layer lands).

use crate::mm::errno::*;

pub fn syscall(_nr: u64, _a: [u64; 6]) -> Result<u64> {
    Err(EAFNOSUPPORT)
}
