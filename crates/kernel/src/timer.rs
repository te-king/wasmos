//! Timer ticks as an async stream.
//!
//! The timer interrupt handler only records that a tick happened and wakes
//! the task waiting for it. Everything else happens in async code consuming
//! [`Ticks`].

use core::{
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    task::{Context, Poll},
};

use futures_util::{Stream, task::AtomicWaker};

/// Timer ticks since the timer was started.
static TICKS: AtomicU64 = AtomicU64::new(0);
/// The task waiting on the [`Ticks`] stream, if any.
static WAKER: AtomicWaker = AtomicWaker::new();
/// Whether a [`Ticks`] stream currently exists.
static TAKEN: AtomicBool = AtomicBool::new(false);

/// Records a timer tick. Called from the timer interrupt handler, so the
/// waker it wakes must be safe to call from interrupt context.
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Release);
    WAKER.wake();
}

/// Timer ticks since the timer was started.
pub fn now() -> u64 {
    TICKS.load(Ordering::Acquire)
}

/// Returns the stream of timer ticks, or `None` while another one exists.
///
/// There is a single waker slot, so only one task can wait on ticks at a
/// time. Fanning ticks out to many waiters (sleep futures, for example)
/// belongs in a task that owns this stream.
pub fn ticks() -> Option<Ticks> {
    let taken = TAKEN.swap(true, Ordering::Acquire);
    (!taken).then(|| Ticks { seen: now() })
}

/// A stream of the tick count, yielding each time it has advanced.
///
/// Ticks that arrive while the consumer is busy are coalesced: each item is
/// the latest count, so the consumer can tell how many it missed.
pub struct Ticks {
    seen: u64,
}

impl Ticks {
    fn advance(&mut self) -> Option<u64> {
        let now = now();
        (now != self.seen).then(|| {
            self.seen = now;
            now
        })
    }
}

impl Stream for Ticks {
    type Item = u64;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<u64>> {
        if let Some(now) = self.advance() {
            return Poll::Ready(Some(now));
        }
        WAKER.register(cx.waker());
        // A tick may have landed between the first check and registering.
        self.advance()
            .map_or(Poll::Pending, |now| Poll::Ready(Some(now)))
    }
}

impl Drop for Ticks {
    fn drop(&mut self) {
        WAKER.take();
        TAKEN.store(false, Ordering::Release);
    }
}
