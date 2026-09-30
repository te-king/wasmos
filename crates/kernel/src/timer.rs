//! Timer ticks, and sleeping for them.
//!
//! The timer interrupt handler only counts the tick and wakes the task that
//! serves the [`Timer`]. That task, which [`serve`] runs alongside the rest
//! of the kernel, owns the ticks and wakes each [`Sleep`] once its deadline
//! has passed. Everything else happens in async code.

use alloc::collections::BTreeMap;
use core::{
    convert::Infallible,
    future::{self, Future},
    pin::{Pin, pin},
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

use futures_util::{
    future::{Either, select},
    task::AtomicWaker,
};

use crate::{arch::Clock, sync::IrqMutex};

/// Timer ticks since the timer was started.
static TICKS: AtomicU64 = AtomicU64::new(0);
/// The task serving the timer, waiting for the next tick.
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

/// Runs `work` with a [`Timer`] to sleep on, serving the timer's sleepers
/// from the clock's ticks meanwhile, and returns what `work` returns.
///
/// The timer only exists while it's served, so a sleep on it always ends.
/// The clock, which proves ticks will come, stays borrowed mutably while
/// this runs, so it's the only consumer of ticks, which it has to be: there
/// is a single waker slot for them.
pub async fn serve<T>(_: &mut Clock, work: impl AsyncFnOnce(&Timer) -> T) -> T {
    let timer = Timer {
        sleepers: IrqMutex::new(Sleepers::default()),
    };
    let ticks = Ticks { seen: now() };
    let work = pin!(work(&timer));
    let service = pin!(timer.wake_sleepers(ticks));
    match select(work, service).await {
        Either::Left((output, _)) => output,
        Either::Right((never, _)) => match never {},
    }
}

/// Sleeps, measured in timer ticks (about 100 Hz under QEMU, not calibrated).
///
/// It's `Sync`: its sleepers are behind an [`IrqMutex`], so a sleep can wait
/// on it from any processor.
pub struct Timer {
    sleepers: IrqMutex<Sleepers>,
}

/// The sleepers waiting on a [`Timer`]: each one's deadline and waker, by
/// the key its [`Sleep`] holds.
#[derive(Default)]
struct Sleepers {
    last_key: u64,
    waiting: BTreeMap<u64, (u64, Waker)>,
}

impl Timer {
    /// Waits for at least `periods` full timer periods.
    pub fn sleep(&self, periods: u64) -> Sleep<'_> {
        // The period under way has partly gone already, so it doesn't count.
        self.until(now() + periods + 1)
    }

    /// Waits for the next tick.
    pub fn next_tick(&self) -> Sleep<'_> {
        self.until(now() + 1)
    }

    /// Waits for `done` to return true, checking it on each tick. Returns
    /// false if it still hasn't after at least `periods` full timer periods.
    pub async fn within(&self, periods: u64, done: impl Fn() -> bool) -> bool {
        let deadline = now() + periods + 1;
        while !done() && now() < deadline {
            self.next_tick().await;
        }
        done()
    }

    fn until(&self, deadline: u64) -> Sleep<'_> {
        Sleep {
            timer: self,
            deadline,
            key: None,
        }
    }

    /// Wakes each sleeper whose deadline has passed, on every tick.
    async fn wake_sleepers(&self, mut ticks: Ticks) -> Infallible {
        loop {
            let now = ticks.next().await;
            self.sleepers
                .lock()
                .waiting
                .values()
                .filter(|&&(deadline, _)| deadline <= now)
                .for_each(|(_, waker)| waker.wake_by_ref());
        }
    }

    /// Records that the sleeper under `key` (a new one, if `None`) waits for
    /// `deadline` with `waker`, and returns its key.
    fn register(&self, key: Option<u64>, deadline: u64, waker: &Waker) -> u64 {
        let mut sleepers = self.sleepers.lock();
        let key = key.unwrap_or_else(|| {
            sleepers.last_key += 1;
            sleepers.last_key
        });
        sleepers.waiting.insert(key, (deadline, waker.clone()));
        key
    }

    fn forget(&self, key: u64) {
        self.sleepers.lock().waiting.remove(&key);
    }
}

/// A wait until the tick count reaches a deadline.
pub struct Sleep<'timer> {
    timer: &'timer Timer,
    deadline: u64,
    /// Its entry in the timer's sleepers, once it has been polled.
    key: Option<u64>,
}

impl Sleep<'_> {
    fn done(&mut self) -> bool {
        let done = now() >= self.deadline;
        if done && let Some(key) = self.key.take() {
            self.timer.forget(key);
        }
        done
    }
}

impl Future for Sleep<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.done() {
            return Poll::Ready(());
        }
        self.key = Some(self.timer.register(self.key, self.deadline, cx.waker()));
        // A tick may have landed, and been served, between the first check
        // and registering.
        if self.done() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl Drop for Sleep<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key {
            self.timer.forget(key);
        }
    }
}

/// The clock's ticks, for the task serving the timer. Each `next` is the
/// latest count once it has advanced: ticks that arrive while the task is
/// busy are coalesced.
struct Ticks {
    seen: u64,
}

impl Ticks {
    async fn next(&mut self) -> u64 {
        future::poll_fn(|cx| {
            if let Some(now) = self.advance() {
                return Poll::Ready(now);
            }
            WAKER.register(cx.waker());
            // A tick may have landed between the first check and registering.
            self.advance().map_or(Poll::Pending, Poll::Ready)
        })
        .await
    }

    fn advance(&mut self) -> Option<u64> {
        let now = now();
        (now != self.seen).then(|| {
            self.seen = now;
            now
        })
    }
}

impl Drop for Ticks {
    fn drop(&mut self) {
        WAKER.take();
    }
}
