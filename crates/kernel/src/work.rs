//! Work for other processors.
//!
//! Each application processor has a mailbox of jobs, which its executor
//! runs one at a time ([`serve`]). [`spawn_on`] hands a future to one of
//! them, and gives back a [`JoinHandle`] to await its output.

use alloc::{boxed::Box, collections::VecDeque, sync::Arc};
use core::{
    convert::Infallible,
    future::{self, Future},
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll},
};

use futures_util::task::AtomicWaker;
use spin::Once;

use crate::{arch::CpuId, sync::IrqMutex};

/// Every processor's mailbox, by id.
static MAILBOXES: Once<Box<[Mailbox]>> = Once::new();

/// A job for a processor to run.
type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

#[derive(Default)]
struct Mailbox {
    /// Whether the processor has started, so it will serve its jobs.
    open: AtomicBool,
    jobs: IrqMutex<VecDeque<Job>>,
    /// The processor's executor, waiting for a job.
    waker: AtomicWaker,
}

/// Makes a closed mailbox for each of `count` processors.
pub fn init(count: usize) {
    MAILBOXES.call_once(|| (0..count).map(|_| Mailbox::default()).collect());
}

/// Opens `cpu`'s mailbox, once it has started: from then on, its jobs are
/// waiting for [`serve`] on it.
///
/// # Panics
/// If [`init`] hasn't made a mailbox for `cpu`.
pub fn open(cpu: CpuId) {
    mailbox(cpu)
        .expect("no mailbox for this processor")
        .open
        .store(true, Ordering::Release);
}

/// Runs `future` on processor `cpu`, after the jobs already handed to it.
/// Returns a handle to await its output, or `None` if `cpu` hasn't started.
pub fn spawn_on<T: Send + 'static>(
    cpu: CpuId,
    future: impl Future<Output = T> + Send + 'static,
) -> Option<JoinHandle<T>> {
    let mailbox = mailbox(cpu).filter(|mailbox| mailbox.open.load(Ordering::Acquire))?;
    let slot = Arc::new(Slot {
        output: IrqMutex::new(None),
        waker: AtomicWaker::new(),
    });
    let sender = slot.clone();
    mailbox.jobs.lock().push_back(Box::pin(async move {
        let output = future.await;
        *sender.output.lock() = Some(output);
        sender.waker.wake();
    }));
    mailbox.waker.wake();
    Some(JoinHandle { slot })
}

/// Runs the jobs handed to processor `cpu`, one at a time, forever. It's
/// the root future of an application processor's executor.
///
/// # Panics
/// If [`init`] hasn't made a mailbox for `cpu`.
pub async fn serve(cpu: CpuId) -> Infallible {
    let mailbox = mailbox(cpu).expect("no mailbox for this processor");
    loop {
        mailbox.next().await.await;
    }
}

fn mailbox(cpu: CpuId) -> Option<&'static Mailbox> {
    MAILBOXES.get()?.get(cpu.index())
}

impl Mailbox {
    async fn next(&self) -> Job {
        future::poll_fn(|cx| {
            if let Some(job) = self.jobs.lock().pop_front() {
                return Poll::Ready(job);
            }
            self.waker.register(cx.waker());
            // A job may have come between the first check and registering.
            self.jobs
                .lock()
                .pop_front()
                .map_or(Poll::Pending, Poll::Ready)
        })
        .await
    }
}

/// The output of a future handed to another processor, once it's ready.
pub struct JoinHandle<T> {
    slot: Arc<Slot<T>>,
}

struct Slot<T> {
    output: IrqMutex<Option<T>>,
    /// The task awaiting the output.
    waker: AtomicWaker,
}

impl<T> Future for JoinHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        if let Some(output) = self.slot.output.lock().take() {
            return Poll::Ready(output);
        }
        self.slot.waker.register(cx.waker());
        // The output may have come between the first check and registering.
        self.slot
            .output
            .lock()
            .take()
            .map_or(Poll::Pending, Poll::Ready)
    }
}
