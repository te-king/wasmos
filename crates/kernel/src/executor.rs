//! Runs the kernel's root futures, halting the processor while they wait.
//!
//! An [`Executor`] runs one root future per [`Executor::block_on`] call.
//! Concurrency comes from composing futures (`join`, `select`,
//! `FuturesUnordered`) rather than from spawning tasks, so there is no task
//! queue: waking only sets the executor's flag, which makes wakers safe to
//! use from interrupt handlers.

use alloc::boxed::Box;
use core::{
    future::{self, Future},
    pin::pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use crate::arch::{self, WakeTarget};

/// Runs root futures on the current processor.
pub struct Executor {
    /// Never freed, since a waker can outlive any one `block_on`: the timer
    /// keeps the last one it was given.
    wake: &'static Wake,
}

/// What a waker needs to wake an executor.
struct Wake {
    /// Set when the root future has been woken since it was last polled.
    woken: AtomicBool,
    /// The processor the executor runs on, which a wake from another one
    /// has to interrupt.
    home: WakeTarget,
}

impl Executor {
    /// An executor for the processor `home`, which is the one it must run
    /// on.
    pub fn new(home: WakeTarget) -> Self {
        Executor {
            wake: Box::leak(Box::new(Wake {
                woken: AtomicBool::new(false),
                home,
            })),
        }
    }

    /// Polls `future` to completion, halting the processor whenever it is
    /// pending and nothing has woken it.
    ///
    /// It borrows the executor mutably, so `future` can't call it again on
    /// the same executor: the inner call would share the flag and could
    /// swallow a wake-up meant for the outer one.
    pub fn block_on<F: Future>(&mut self, future: F) -> F::Output {
        let mut future = pin!(future);
        let waker = waker(self.wake);
        let mut cx = Context::from_waker(&waker);
        loop {
            self.wake.woken.store(false, Ordering::Release);
            if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
            // The flag is checked with interrupts off, so a wake-up from an
            // interrupt handler can't be missed between the check and the
            // halt.
            arch::wait_for_interrupt(|| self.wake.woken.load(Ordering::Acquire));
        }
    }
}

/// Lets the rest of the root future run before carrying on: pending once,
/// having woken itself, so the executor polls again straight away.
pub async fn yield_now() {
    let mut yielded = false;
    future::poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}

/// A waker that sets the executor's flag, then makes sure its processor
/// notices. It only carries a pointer to [`Wake`], so cloning and dropping
/// it never allocate or free, even from an interrupt handler.
fn waker(state: &'static Wake) -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake, noop);

    fn clone(wake: *const ()) -> RawWaker {
        RawWaker::new(wake, &VTABLE)
    }
    fn wake(wake: *const ()) {
        // SAFETY: Every waker's data is a `&'static Wake` (see below).
        let wake = unsafe { &*wake.cast::<Wake>() };
        // Set before the processor is interrupted, so it sees the flag.
        wake.woken.store(true, Ordering::Release);
        arch::wake(wake.home);
    }
    fn noop(_: *const ()) {}

    // SAFETY: The data is a `&'static Wake`, valid for as long as any clone
    // could use it, and the vtable functions are safe to call from any
    // context, any number of times.
    unsafe { Waker::from_raw(RawWaker::new((&raw const *state).cast(), &VTABLE)) }
}
