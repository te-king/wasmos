#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use futures_util::{
    StreamExt,
    future::{OptionFuture, join3},
    stream,
};
use thiserror::Error;

mod arch;
mod executor;
mod guest;
mod host;
mod log;
mod sync;
mod timer;
mod work;

use arch::CpuId;
use timer::Timer;

const WSHELL: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_WSHELL"));

/// Where the kernel runs a second shell, if that processor has started.
const SECOND_SHELL: CpuId = CpuId::nth(1);

/// Why the kernel proper failed.
#[derive(Debug, Error)]
pub enum Error {
    /// A guest failed, trapping or failing to load.
    #[error("cpu {cpu}: guest: {error}")]
    Guest { cpu: CpuId, error: wasmi::Error },
}

/// The kernel proper, on the bootstrap processor, once the boot sequence
/// has started the timer and the other processors.
pub async fn kernel_main(timer: &'static Timer) -> Result<(), Error> {
    // A second shell on another processor shows guests running there too.
    let there: OptionFuture<_> = work::spawn_on(SECOND_SHELL, guest::run(WSHELL, timer)).into();
    let (here, (), there) = join3(guest::run(WSHELL, timer), tick_task(timer), there).await;
    here.map_err(|error| Error::Guest {
        cpu: CpuId::BSP,
        error,
    })?;
    there.transpose().map_err(|error| Error::Guest {
        cpu: SECOND_SHELL,
        error,
    })?;
    Ok(())
}

async fn tick_task(timer: &Timer) {
    let seen: Vec<u64> = stream::iter(0..3)
        .then(|_| async {
            timer.next_tick().await;
            timer::now()
        })
        .collect()
        .await;
    logln!("timer: ticks {seen:?}");
}
