#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use futures_util::{StreamExt, future::join};
use wasmi::Error;

mod arch;
mod executor;
mod guest;
mod host;
mod log;
mod sync;
mod timer;

const WSHELL: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_WSHELL"));

/// The kernel proper, once the boot sequence has started the clock.
pub async fn kernel_main(clock: &mut arch::Clock) -> Result<(), Error> {
    let (shell, ()) = join(guest::run(WSHELL), tick_task(timer::ticks(clock))).await;
    shell
}

async fn tick_task(ticks: timer::Ticks<'_>) {
    let seen: Vec<u64> = ticks.take(3).collect().await;
    logln!("timer: ticks {seen:?}");
}
