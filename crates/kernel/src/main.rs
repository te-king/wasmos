#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use futures_util::{StreamExt, future::join, stream};
use wasmi::Error;

mod arch;
mod executor;
mod guest;
mod host;
mod log;
mod sync;
mod timer;

use timer::Timer;

const WSHELL: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_WSHELL"));

/// The kernel proper, once the boot sequence has started the timer.
pub async fn kernel_main(timer: &Timer) -> Result<(), Error> {
    let (shell, ()) = join(guest::run(WSHELL, timer), tick_task(timer)).await;
    shell
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
