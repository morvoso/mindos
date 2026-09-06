//! /dev/mind: the channel between user space and the kernel's AI-facing services.
//!
//! For now it exposes the kernel log and accepts control messages; the LLM
//! runtime itself lives in user space (mindd) and talks over unix sockets.

use crate::fs::vfs::{File, FileOps, POLLIN, POLLOUT};
use crate::mm::errno::*;
use alloc::sync::Arc;

pub struct MindDev;

pub fn open(_flags: u32) -> Result<Arc<dyn FileOps>> {
    if let Some(t) = crate::sched::try_current() {
        t.proc.mind_client.store(true, core::sync::atomic::Ordering::Relaxed);
    }
    Ok(Arc::new(MindDev))
}

impl FileOps for MindDev {
    fn read(&self, f: &File, b: &mut [u8]) -> Result<usize> {
        let pos = f.pos() as usize;
        let (n, newpos) = crate::console::CONSOLE.lock().log.read_from(pos, b);
        f.set_pos(newpos as u64);
        Ok(n)
    }
    fn write(&self, _f: &File, b: &[u8]) -> Result<usize> {
        let s = core::str::from_utf8(b).unwrap_or("").trim_end();
        match s {
            "reboot" => crate::arch::x86_64::reboot(),
            "poweroff" => crate::arch::x86_64::power_off(),
            _ => klog!("mind", "{}", s),
        }
        Ok(b.len())
    }
    fn poll(&self, _f: &File) -> u32 {
        POLLIN | POLLOUT
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
