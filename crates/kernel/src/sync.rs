//! Locks for data that interrupt handlers can reach too.
//!
//! An interrupt handler that takes a lock its own processor already holds
//! would spin forever, so an [`IrqMutex`] holds off this processor's
//! interrupts for as long as it's held. Other processors' interrupts carry
//! on, and their handlers just wait their turn.

use core::{
    hint,
    sync::atomic::{AtomicBool, Ordering},
};

use lock_api::{GuardNoSend, RawMutex};

use crate::arch;

/// A spin lock that disables this processor's interrupts while it's held,
/// and restores them once it's released.
///
/// Guards restore the interrupt state from when they locked, so they must
/// be released in the reverse order they were taken, as scoped guards are.
pub type IrqMutex<T> = lock_api::Mutex<RawIrqMutex, T>;

/// The lock behind [`IrqMutex`].
pub struct RawIrqMutex {
    locked: AtomicBool,
    /// Whether the holder's interrupts were enabled when it locked. Only
    /// the holder touches it.
    enable_on_unlock: AtomicBool,
}

impl RawIrqMutex {
    fn acquire(&self) -> bool {
        self.locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }
}

// SAFETY: `locked` is only set by a successful compare-exchange, so one
// holder at a time has the data, and its Acquire and the Release in
// `unlock` order the holders' accesses.
unsafe impl RawMutex for RawIrqMutex {
    // How a lock starts out: each use is a fresh copy, never shared.
    #[allow(clippy::declare_interior_mutable_const)]
    const INIT: Self = RawIrqMutex {
        locked: AtomicBool::new(false),
        enable_on_unlock: AtomicBool::new(false),
    };

    // A guard holds off interrupts on the processor that took it, so it has
    // to be released there.
    type GuardMarker = GuardNoSend;

    fn lock(&self) {
        let enabled = arch::interrupts_enabled();
        arch::disable_interrupts();
        while !self.acquire() {
            hint::spin_loop();
        }
        self.enable_on_unlock.store(enabled, Ordering::Relaxed);
    }

    fn try_lock(&self) -> bool {
        let enabled = arch::interrupts_enabled();
        arch::disable_interrupts();
        let locked = self.acquire();
        if locked {
            self.enable_on_unlock.store(enabled, Ordering::Relaxed);
        } else if enabled {
            arch::enable_interrupts();
        }
        locked
    }

    unsafe fn unlock(&self) {
        let enable = self.enable_on_unlock.load(Ordering::Relaxed);
        self.locked.store(false, Ordering::Release);
        if enable {
            arch::enable_interrupts();
        }
    }

    fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}
