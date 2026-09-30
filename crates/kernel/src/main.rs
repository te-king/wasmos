#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use futures_util::{StreamExt, future::join};
use timer::Ticks;
use wasmi::{Engine, Error, Module, Store};

mod arch;
mod executor;
mod host;
mod log;
mod timer;

const WSHELL: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_WSHELL"));

/// The kernel proper, once every processor is up. `ticks` is the clock.
pub async fn kernel_main(ticks: Ticks) -> Result<(), Error> {
    run_guest(WSHELL)?;
    join(example_task(), tick_task(ticks)).await;
    Ok(())
}

/// Instantiates a guest module and runs its entry point to completion.
fn run_guest(wasm: &[u8]) -> Result<(), Error> {
    let engine = Engine::default();
    let mut store = Store::new(&engine, ());
    let module = Module::new(&engine, wasm)?;
    let instance = host::linker(&engine)?.instantiate_and_start(&mut store, &module)?;
    instance
        .get_typed_func::<(), ()>(&store, wasmos_abi::ENTRY)?
        .call(&mut store, ())
}

async fn async_number() -> u32 {
    42
}

async fn example_task() {
    let number = async_number().await;
    logln!("async number: {}", number);
}

async fn tick_task(ticks: Ticks) {
    let seen: Vec<u64> = ticks.take(3).collect().await;
    logln!("timer: ticks {:?}", seen);
}
