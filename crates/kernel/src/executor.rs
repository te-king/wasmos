//! Runs the kernel's root future, halting the processor while it waits.
//!
//! There is one root future per [`block_on`] call. Concurrency comes from
//! composing futures (`join`, `select`, `FuturesUnordered`) rather than from
//! spawning tasks, so there is no task queue: waking only sets a flag, which
//! makes wakers safe to use from interrupt handlers.

use core::{
    future::Future,
    pin::pin,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use x86_64::instructions::interrupts;

/// Set when the root future has been woken since it was last polled.
static WOKEN: AtomicBool = AtomicBool::new(false);
/// Whether [`block_on`] is running. Nested calls would share [`WOKEN`].
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Polls `future` to completion, halting the processor whenever it is
/// pending and nothing has woken it.
///
/// # Panics
/// If called while another `block_on` is already running.
pub fn block_on<F: Future>(future: F) -> F::Output {
    assert!(
        !RUNNING.swap(true, Ordering::Acquire),
        "block_on is not reentrant"
    );
    let mut future = pin!(future);
    let waker = waker();
    let mut cx = Context::from_waker(&waker);

    let output = loop {
        WOKEN.store(false, Ordering::Release);
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            break output;
        }
        // With interrupts off, a wake-up from an interrupt handler can't land
        // between checking the flag and halting. `enable_and_hlt` re-enables
        // them atomically with the halt, so the next interrupt resumes us.
        interrupts::disable();
        if WOKEN.load(Ordering::Acquire) {
            interrupts::enable();
        } else {
            interrupts::enable_and_hlt();
        }
    };

    RUNNING.store(false, Ordering::Release);
    output
}

/// A waker that sets [`WOKEN`]. It carries no data, so cloning and dropping
/// it never allocate or free, even from an interrupt handler.
fn waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake, noop);

    fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(ptr::null(), &VTABLE)
    }
    fn wake(_: *const ()) {
        WOKEN.store(true, Ordering::Release);
    }
    fn noop(_: *const ()) {}

    // SAFETY: The vtable functions ignore the data pointer, and all of them
    // are safe to call from any context, any number of times.
    unsafe { Waker::from_raw(RawWaker::new(ptr::null(), &VTABLE)) }
}
