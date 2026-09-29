//! Timer ticks as an async stream.
//!
//! The timer interrupt handler only records that a tick happened and wakes
//! the task waiting for it. Everything else happens in async code consuming
//! [`Ticks`].

use core::{
    marker::PhantomData,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll},
};

use futures_util::{Stream, task::AtomicWaker};

use crate::arch::Clock;

/// Timer ticks since the timer was started.
static TICKS: AtomicU64 = AtomicU64::new(0);
/// The task waiting on the [`Ticks`] stream, if any.
static WAKER: AtomicWaker = AtomicWaker::new();

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

/// The first tick count by which at least `periods` full timer periods will
/// have passed since the count was `now`. The period under way at `now` has
/// partly gone already, so it doesn't count.
pub const fn after(now: u64, periods: u64) -> u64 {
    now + periods + 1
}

/// The stream of timer ticks.
///
/// Taking the clock proves that ticks will come. Borrowing it mutably makes
/// this the only stream while it lives, which it has to be: there is a
/// single waker slot, so only one task can wait on ticks at a time. Fanning
/// ticks out to many waiters (sleep futures, for example) belongs in a task
/// that owns this stream.
pub fn ticks(_: &mut Clock) -> Ticks<'_> {
    Ticks {
        seen: now(),
        clock: PhantomData,
    }
}

/// A stream of the tick count, yielding each time it has advanced.
///
/// Ticks that arrive while the consumer is busy are coalesced: each item is
/// the latest count, so the consumer can tell how many it missed.
pub struct Ticks<'clock> {
    seen: u64,
    clock: PhantomData<&'clock mut Clock>,
}

impl Ticks<'_> {
    fn advance(&mut self) -> Option<u64> {
        let now = now();
        (now != self.seen).then(|| {
            self.seen = now;
            now
        })
    }
}

impl Stream for Ticks<'_> {
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

impl Drop for Ticks<'_> {
    fn drop(&mut self) {
        WAKER.take();
    }
}
