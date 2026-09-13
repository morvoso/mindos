//! A mutex that panics, instead of waiting forever, when the thread holding
//! it locks it again. A MindOS change: see `MINDOS-PATCHES.md`.

use std::{
    fmt,
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        LockResult, Mutex, MutexGuard, PoisonError,
    },
};

/// A [`Mutex`] that knows which thread holds it.
///
/// Smithay calls back into the compositor while holding some of its locks. A
/// surface-tree walk holds the lock of each surface it visits while the
/// closures run, and the seat holds the pointer's or keyboard's lock while a
/// grab or a focus callback runs. A callback that takes the same lock again,
/// such as `with_states` on the surface being walked or `current_location()`
/// inside a pointer grab, blocks on itself. The event loop stops with it, and
/// every display freezes. Taking a lock this thread already holds panics
/// here instead, and a compositor that catches panics loses one callback.
///
/// Poisoning is ignored for the same reason: after a panic has been caught,
/// the locks it unwound through still have to open.
///
/// `lock` returns a [`LockResult`] so that call sites written for
/// [`Mutex`] need no change, but the result is never an error.
pub struct CheckedMutex<T> {
    /// The token of the thread holding the lock, or 0.
    holder: AtomicUsize,
    inner: Mutex<T>,
}

/// The guard of a [`CheckedMutex`]; the lock is released when it drops.
pub struct CheckedMutexGuard<'a, T> {
    holder: &'a AtomicUsize,
    guard: MutexGuard<'a, T>,
}

/// A number that differs between any two live threads: the address of a
/// thread-local.
fn this_thread() -> usize {
    thread_local!(static TOKEN: u8 = const { 0 });
    TOKEN.with(|token| token as *const u8 as usize)
}

impl<T> CheckedMutex<T> {
    /// A new unlocked mutex holding `value`.
    pub fn new(value: T) -> Self {
        CheckedMutex {
            holder: AtomicUsize::new(0),
            inner: Mutex::new(value),
        }
    }

    /// Whether the calling thread holds this lock right now.
    ///
    /// Only the holder writes its own token, and it clears the token before
    /// it unlocks, so the answer cannot be stale for the calling thread.
    pub fn is_held_by_this_thread(&self) -> bool {
        self.holder.load(Ordering::Relaxed) == this_thread()
    }

    /// Lock, waiting for another thread to let go if one holds it.
    ///
    /// # Panics
    ///
    /// If the calling thread already holds the lock, which would otherwise
    /// wait forever.
    #[track_caller]
    pub fn lock(&self) -> LockResult<CheckedMutexGuard<'_, T>> {
        if self.is_held_by_this_thread() {
            panic!(
                "{} is already locked by this thread: a callback that runs while it is held \
                 tried to take it again, which would wait forever",
                std::any::type_name::<T>()
            );
        }
        Ok(self.lock_unchecked())
    }

    /// Lock without the check: for callers that have done it themselves,
    /// with a better message.
    pub(crate) fn lock_unchecked(&self) -> CheckedMutexGuard<'_, T> {
        let guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        self.holder.store(this_thread(), Ordering::Relaxed);
        CheckedMutexGuard {
            holder: &self.holder,
            guard,
        }
    }
}

impl<T: Default> Default for CheckedMutex<T> {
    fn default() -> Self {
        CheckedMutex::new(T::default())
    }
}

impl<T: fmt::Debug> fmt::Debug for CheckedMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `Mutex`'s own `Debug` only tries the lock, so it cannot block here.
        self.inner.fmt(f)
    }
}

impl<T> Drop for CheckedMutexGuard<'_, T> {
    fn drop(&mut self) {
        // Runs before `guard` unlocks.
        self.holder.store(0, Ordering::Relaxed);
    }
}

impl<T> Deref for CheckedMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for CheckedMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T: fmt::Debug> fmt::Debug for CheckedMutexGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.guard.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        panic::{catch_unwind, AssertUnwindSafe},
        sync::Arc,
        time::Duration,
    };

    #[test]
    fn locking_twice_on_one_thread_panics_instead_of_hanging() {
        let lock = CheckedMutex::new(1);
        let held = lock.lock().unwrap();
        assert!(lock.is_held_by_this_thread());
        let again = catch_unwind(AssertUnwindSafe(|| {
            let _second = lock.lock();
        }));
        let message = again.expect_err("the second lock must panic");
        let message = message.downcast_ref::<String>().unwrap();
        assert!(message.contains("already locked by this thread"), "{message}");
        assert!(message.contains("i32"), "{message}");
        drop(held);
        assert!(!lock.is_held_by_this_thread());
        assert_eq!(*lock.lock().unwrap(), 1, "the lock opens again after the guard drops");
    }

    #[test]
    fn another_thread_still_waits_its_turn() {
        let lock = Arc::new(CheckedMutex::new(0));
        let mut held = lock.lock().unwrap();
        let other = {
            let lock = Arc::clone(&lock);
            std::thread::spawn(move || {
                assert!(!lock.is_held_by_this_thread());
                *lock.lock().unwrap() += 1;
            })
        };
        std::thread::sleep(Duration::from_millis(20));
        *held += 10;
        drop(held);
        other.join().unwrap();
        assert_eq!(*lock.lock().unwrap(), 11);
    }

    #[test]
    fn a_panic_under_the_lock_does_not_poison_it() {
        let lock = CheckedMutex::new(vec![1]);
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut held = lock.lock().unwrap();
            held.push(2);
            panic!("a callback went wrong with the lock held");
        }));
        assert!(!lock.is_held_by_this_thread());
        assert_eq!(*lock.lock().unwrap(), vec![1, 2]);
    }
}
