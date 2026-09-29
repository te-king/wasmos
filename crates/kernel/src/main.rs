#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use futures_util::{StreamExt, future::join3};
use wasmi::Error;

mod arch;
mod executor;
mod guest;
mod host;
mod log;
mod timer;

const WSHELL: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_WSHELL"));

pub async fn kernel_main() -> Result<(), Error> {
    let (shell, (), ()) = join3(guest::run(WSHELL), example_task(), tick_task()).await;
    shell
}

async fn async_number() -> u32 {
    42
}

async fn example_task() {
    let number = async_number().await;
    logln!("async number: {number}");
}

async fn tick_task() {
    let ticks = timer::ticks().expect("nothing else is using the timer");
    let seen: Vec<u64> = ticks.take(3).collect().await;
    logln!("timer: ticks {seen:?}");
}
