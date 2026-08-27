//! Poison-tolerant mutex locking.
//!
//! # Why this exists
//!
//! `Mutex::lock()` returns `Err` forever once a thread panics while holding the
//! guard. `.lock().unwrap()` therefore turns a single, local panic into a
//! permanent one: every subsequent lock of that mutex panics too. In this app
//! the mutexes guard recorder handles, mute state, model-loading flags and
//! download progress — all touched from audio callbacks and worker threads. One
//! panic in a callback would take the whole capture path down for the rest of
//! the process lifetime, and the user would see an app that has stopped
//! recording with no way back short of a restart.
//!
//! Recovering with [`PoisonError::into_inner`] keeps the app running with the
//! state as the panicking thread left it. That is the right trade here because
//! the guarded values are small and independently meaningful — an
//! `Option<Recorder>`, a `bool`, an enum, a progress counter — not multi-field
//! invariants that a half-finished update would corrupt.
//!
//! # This is the house style, not a new opinion
//!
//! The newer modules already do exactly this, each with its own private helper:
//! `managers::meeting::lock`, `audio_toolkit::audio::meeting_recorder::lock_recover`,
//! and inline `unwrap_or_else(|e| e.into_inner())` in `managers::transcription`
//! and `managers::meeting_transcription`. This module is the single canonical
//! version those converge on, so the pattern stops being re-invented per file.
//!
//! # Where `unwrap` is still correct
//!
//! Tests. A poisoned mutex in a test means a thread under test panicked, and
//! that should fail the test loudly rather than be papered over. Test modules
//! keep `.lock().unwrap()`.

use std::sync::{Mutex, MutexGuard};

/// Locking that survives a poisoned mutex.
pub trait MutexExt<T: ?Sized> {
    /// Lock, recovering the guard if the mutex was poisoned by a panicking
    /// thread.
    ///
    /// Prefer this over `.lock().unwrap()` everywhere outside tests.
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> MutexExt<T> for Mutex<T> {
    #[inline]
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::MutexExt;
    use std::sync::{Arc, Mutex};

    #[test]
    fn lock_recover_returns_the_guard_on_a_healthy_mutex() {
        let m = Mutex::new(7);
        assert_eq!(*m.lock_recover(), 7);
    }

    #[test]
    fn lock_recover_survives_a_poisoned_mutex_and_sees_the_last_write() {
        let m = Arc::new(Mutex::new(0));

        let poisoner = {
            let m = Arc::clone(&m);
            std::thread::spawn(move || {
                let mut guard = m.lock().expect("first lock cannot be poisoned");
                *guard = 42;
                panic!("poison the mutex while holding the guard");
            })
        };
        assert!(
            poisoner.join().is_err(),
            "the helper thread must have panicked"
        );

        // The contrast that motivates this module: the std path is now
        // permanently broken...
        assert!(m.lock().is_err(), "the mutex must be poisoned");
        // ...while the recovering path keeps working and observes the write the
        // panicking thread had already completed.
        assert_eq!(*m.lock_recover(), 42);

        // Still usable afterwards, not a one-shot recovery.
        *m.lock_recover() = 43;
        assert_eq!(*m.lock_recover(), 43);
    }
}
