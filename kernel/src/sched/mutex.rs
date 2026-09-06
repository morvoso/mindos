//! Sleeping mutex for data that may be held across blocking operations.

use super::wait::WaitQueue;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub struct Mutex<T: ?Sized> {
    locked: AtomicBool,
    wq: WaitQueue,
    data: UnsafeCell<T>,
}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}

pub struct MutexGuard<'a, T: ?Sized> {
    m: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    pub const fn new(t: T) -> Self {
        Mutex { locked: AtomicBool::new(false), wq: WaitQueue::new(), data: UnsafeCell::new(t) }
    }
}

impl<T: ?Sized> Mutex<T> {
    pub fn lock(&self) -> MutexGuard<'_, T> {
        loop {
            if self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                return MutexGuard { m: self };
            }
            if super::try_current().is_none() {
                core::hint::spin_loop();
                continue;
            }
            self.wq.wait_until_uninterruptible(|| !self.locked.load(Ordering::Relaxed));
        }
    }
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        if self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            Some(MutexGuard { m: self })
        } else {
            None
        }
    }
    pub unsafe fn get_unchecked(&self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.m.data.get() }
    }
}
impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.m.data.get() }
    }
}
impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.m.locked.store(false, Ordering::Release);
        self.m.wq.wake_one();
    }
}
