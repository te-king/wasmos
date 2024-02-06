use alloc::{sync::Arc, task::Wake};

/// Wakes the given processor using an interrupt when triggered
pub struct ProcessorWaker(u64);

impl Wake for ProcessorWaker {
    fn wake(self: Arc<Self>) {
        todo!("wake the processor using an interrupt")
    }
}
