//! Timed tests of one test binary run one at a time. Cargo runs a binary's
//! tests on parallel threads, and a measurement that shares the CPU with
//! another one is slower by an amount that depends on both.

use std::sync::{Mutex, MutexGuard, PoisonError};

static TIMING: Mutex<()> = Mutex::new(());

/// Held for a whole timed test: no other timed test of the binary runs
/// meanwhile. A failed timed test does not block the next ones.
pub fn exclusive() -> MutexGuard<'static, ()> {
    TIMING.lock().unwrap_or_else(PoisonError::into_inner)
}
