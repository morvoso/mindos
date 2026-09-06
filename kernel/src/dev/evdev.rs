//! /dev/input/event0: evdev-compatible access to keyboard and mouse events.

use super::input::{InputEvent, EV_KEY, EV_REL, EV_SYN};
use crate::fs::vfs::{File, FileOps, POLLIN};
use crate::mm::errno::*;
use crate::mm::user::copy_to_user;
use crate::sched::wait::WaitQueue;
use crate::sync::SpinLock;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

pub struct EvdevReader {
    queue: SpinLock<VecDeque<InputEvent>>,
    wq: WaitQueue,
    grab: AtomicBool,
}

static READERS: SpinLock<Vec<Weak<EvdevReader>>> = SpinLock::new(Vec::new());
static GRABBED: AtomicBool = AtomicBool::new(false);
static WQ: WaitQueue = WaitQueue::new();

/// True while a client holds EVIOCGRAB (the console tty then ignores keys).
pub fn grabbed() -> bool {
    GRABBED.load(Ordering::Relaxed)
}

/// Move events from the global queue to every open reader (called after IRQs queue events).
pub fn distribute() {
    let mut evs: Vec<InputEvent> = Vec::new();
    {
        let mut q = super::input::EVENTS.lock();
        while let Some(e) = q.pop() {
            evs.push(e);
        }
    }
    if evs.is_empty() {
        return;
    }
    let readers = READERS.lock();
    for r in readers.iter() {
        if let Some(r) = r.upgrade() {
            let mut q = r.queue.lock();
            for e in &evs {
                if q.len() >= 1024 {
                    q.pop_front();
                }
                q.push_back(*e);
            }
            r.wq.wake_all();
        }
    }
    WQ.wake_all();
}

fn waker() {
    distribute();
}

pub fn init() {
    super::input::set_waker(waker);
}

pub fn open(_flags: u32) -> Result<Arc<dyn FileOps>> {
    let r = Arc::new(EvdevReader { queue: SpinLock::new(VecDeque::new()), wq: WaitQueue::new(), grab: AtomicBool::new(false) });
    let mut readers = READERS.lock();
    readers.retain(|w| w.strong_count() > 0);
    readers.push(Arc::downgrade(&r));
    Ok(r)
}

impl FileOps for EvdevReader {
    fn read(&self, f: &File, b: &mut [u8]) -> Result<usize> {
        const SZ: usize = core::mem::size_of::<InputEvent>();
        if b.len() < SZ {
            return Err(EINVAL);
        }
        loop {
            {
                let mut q = self.queue.lock();
                if !q.is_empty() {
                    let mut n = 0;
                    while n + SZ <= b.len() {
                        match q.pop_front() {
                            Some(e) => {
                                let bytes = unsafe { core::slice::from_raw_parts(&e as *const _ as *const u8, SZ) };
                                b[n..n + SZ].copy_from_slice(bytes);
                                n += SZ;
                            }
                            None => break,
                        }
                    }
                    return Ok(n);
                }
            }
            if f.nonblock() {
                return Err(EAGAIN);
            }
            self.wq.wait_until(|| !self.queue.lock().is_empty()).map_err(|_| EINTR)?;
        }
    }
    fn write(&self, _f: &File, b: &[u8]) -> Result<usize> {
        Ok(b.len())
    }
    fn poll(&self, _f: &File) -> u32 {
        if self.queue.lock().is_empty() {
            0
        } else {
            POLLIN
        }
    }
    fn poll_wait(&self) -> Option<&WaitQueue> {
        Some(&self.wq)
    }
    fn ioctl(&self, _f: &File, cmd: u32, arg: usize) -> Result<usize> {
        let nr = cmd & 0xff;
        let size = ((cmd >> 16) & 0x3fff) as usize;
        let typ = (cmd >> 8) & 0xff;
        if typ != b'E' as u32 {
            return Err(ENOTTY);
        }
        match nr {
            0x01 => {
                copy_to_user(arg, &0x010001u32.to_le_bytes())?; // EVIOCGVERSION
                Ok(0)
            }
            0x02 => {
                // EVIOCGID: bustype, vendor, product, version
                let id: [u16; 4] = [0x11, 0x0001, 0x0001, 0x0100];
                copy_to_user(arg, unsafe { core::slice::from_raw_parts(id.as_ptr() as *const u8, 8) })?;
                Ok(0)
            }
            0x06 | 0x07 | 0x08 => {
                // EVIOCGNAME / EVIOCGPHYS / EVIOCGUNIQ
                let s: &[u8] = match nr {
                    0x06 => b"MindOS PS/2 Keyboard and Mouse\0",
                    0x07 => b"isa0060/serio0\0",
                    _ => b"\0",
                };
                let n = s.len().min(size);
                copy_to_user(arg, &s[..n])?;
                Ok(n)
            }
            0x18 => {
                // EVIOCGKEY: key state bitmap (report none pressed)
                let z = alloc::vec![0u8; size];
                copy_to_user(arg, &z)?;
                Ok(size)
            }
            0x20..=0x3f => {
                // EVIOCGBIT(ev, len)
                let ev = nr - 0x20;
                let mut bits = alloc::vec![0u8; size];
                let set = |bits: &mut Vec<u8>, i: usize| {
                    if i / 8 < bits.len() {
                        bits[i / 8] |= 1 << (i % 8);
                    }
                };
                match ev as u16 {
                    0 => {
                        set(&mut bits, EV_SYN as usize);
                        set(&mut bits, EV_KEY as usize);
                        set(&mut bits, EV_REL as usize);
                    }
                    EV_KEY => {
                        for k in 1..=127 {
                            set(&mut bits, k);
                        }
                        for k in 0x110..=0x114 {
                            set(&mut bits, k);
                        }
                    }
                    EV_REL => {
                        set(&mut bits, 0);
                        set(&mut bits, 1);
                        set(&mut bits, 8);
                    }
                    _ => {}
                }
                copy_to_user(arg, &bits)?;
                Ok(size)
            }
            0x90 => {
                // EVIOCGRAB
                let on = arg != 0;
                self.grab.store(on, Ordering::Relaxed);
                GRABBED.store(on, Ordering::Relaxed);
                Ok(0)
            }
            0xa0 => Ok(0), // EVIOCSCLOCKID
            _ => Err(ENOTTY),
        }
    }
    fn release(&self, _f: &File) {
        if self.grab.load(Ordering::Relaxed) {
            GRABBED.store(false, Ordering::Relaxed);
        }
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
