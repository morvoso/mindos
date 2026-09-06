//! Kernel synchronisation primitives.
//!
//! `SpinLock` disables interrupts on the local CPU while held, so it is safe to
//! share data between task context and interrupt handlers. It must never be
//! held across anything that can sleep. Sleeping mutexes live in `sched`.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[inline(always)]
pub fn irq_save() -> u64 {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq; pop {}; cli", out(reg) flags, options(nomem, preserves_flags));
    }
    flags
}

#[inline(always)]
pub fn irq_restore(flags: u64) {
    if flags & 0x200 != 0 {
        unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
    }
}

#[inline(always)]
pub fn irqs_enabled() -> bool {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags));
    }
    flags & 0x200 != 0
}

/// RAII guard that keeps interrupts disabled on the local CPU.
pub struct IrqGuard(u64);
impl IrqGuard {
    pub fn new() -> Self {
        IrqGuard(irq_save())
    }
}
impl Drop for IrqGuard {
    fn drop(&mut self) {
        irq_restore(self.0);
    }
}

pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}
unsafe impl<T: ?Sized + Send> Sync for SpinLock<T> {}
unsafe impl<T: ?Sized + Send> Send for SpinLock<T> {}

pub struct SpinLockGuard<'a, T: ?Sized> {
    lock: &'a SpinLock<T>,
    flags: u64,
}

impl<T> SpinLock<T> {
    pub const fn new(data: T) -> Self {
        SpinLock { locked: AtomicBool::new(false), data: UnsafeCell::new(data) }
    }
    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: ?Sized> SpinLock<T> {
    #[inline]
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let flags = irq_save();
        loop {
            if self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                break;
            }
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        SpinLockGuard { lock: self, flags }
    }
    #[inline]
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let flags = irq_save();
        if self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            Some(SpinLockGuard { lock: self, flags })
        } else {
            irq_restore(flags);
            None
        }
    }
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
    /// Break a lock during panic handling. Unsound if another holder exists.
    pub unsafe fn force_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
    /// Access the data without locking (for single-threaded init or panic paths).
    pub unsafe fn get_unchecked(&self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }
}

impl<T: ?Sized> Deref for SpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}
impl<T: ?Sized> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}
impl<T: ?Sized> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        irq_restore(self.flags);
    }
}

/// Write-once cell for global state initialised during boot.
pub struct Once<T> {
    state: AtomicU8, // 0 = empty, 1 = initialising, 2 = ready
    data: UnsafeCell<MaybeUninit<T>>,
}
unsafe impl<T: Send + Sync> Sync for Once<T> {}
unsafe impl<T: Send> Send for Once<T> {}

impl<T> Once<T> {
    pub const fn new() -> Self {
        Once { state: AtomicU8::new(0), data: UnsafeCell::new(MaybeUninit::uninit()) }
    }
    pub fn call_once<F: FnOnce() -> T>(&self, f: F) -> &T {
        if self.state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            unsafe { (*self.data.get()).write(f()) };
            self.state.store(2, Ordering::Release);
        } else {
            while self.state.load(Ordering::Acquire) != 2 {
                core::hint::spin_loop();
            }
        }
        unsafe { (*self.data.get()).assume_init_ref() }
    }
    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == 2 {
            Some(unsafe { (*self.data.get()).assume_init_ref() })
        } else {
            None
        }
    }
    pub fn is_ready(&self) -> bool {
        self.state.load(Ordering::Acquire) == 2
    }
}
impl<T> Deref for Once<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get().expect("Once accessed before initialisation")
    }
}

/// Lazily initialised global, evaluated on first dereference.
pub struct Lazy<T, F = fn() -> T> {
    once: Once<T>,
    init: UnsafeCell<Option<F>>,
}
unsafe impl<T: Send + Sync, F: Send> Sync for Lazy<T, F> {}
impl<T, F: FnOnce() -> T> Lazy<T, F> {
    pub const fn new(f: F) -> Self {
        Lazy { once: Once::new(), init: UnsafeCell::new(Some(f)) }
    }
    pub fn force(&self) -> &T {
        self.once.call_once(|| {
            let f = unsafe { (*self.init.get()).take() }.expect("Lazy init taken twice");
            f()
        })
    }
}
impl<T, F: FnOnce() -> T> Deref for Lazy<T, F> {
    type Target = T;
    fn deref(&self) -> &T {
        self.force()
    }
}
