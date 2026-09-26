#![feature(abi_x86_interrupt)]
#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use futures_util::StreamExt;
use sync::{executor::SimpleExecutor, task::Task};
use wasmi::{Caller, Engine, Error, Func, Linker, Module, Store, TrapCode};

#[path = "arch/x86_64/mod.rs"]
mod arch;

mod log;
mod qemu;
mod sync;
mod timer;

const WSHELL: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_WSHELL"));

pub fn kernel_main() -> Result<(), Error> {
    let engine = Engine::default();
    let mut linker = Linker::<()>::new(&engine);
    let mut store = Store::<()>::new(&engine, ());

    let wasmos_print = Func::wrap(
        &mut store,
        |caller: Caller<'_, _>, offset: u32, length: u32| -> Result<(), Error> {
            // Bad input from the guest traps the guest rather than panicking the kernel.
            let memory = caller
                .get_export("memory")
                .and_then(|export| export.into_memory())
                .ok_or_else(|| Error::new("wasmos_print: guest has no 'memory' export"))?;

            let mut buffer = alloc::vec![0u8; length as usize];
            memory
                .read(caller, offset as usize, &mut buffer)
                .map_err(|_| TrapCode::MemoryOutOfBounds)?;
            let s = core::str::from_utf8(&buffer)
                .map_err(|_| Error::new("wasmos_print: string is not valid UTF-8"))?;
            log!("{}", s);
            Ok(())
        },
    );

    linker.define("host", "wasmos_print", wasmos_print)?;

    let host_hello = Func::wrap(&mut store, |parameter: i32| {
        logln!("Got {} from WebAssembly", parameter);
    });

    linker.define("host", "hello", host_hello)?;

    let module = Module::new(&engine, WSHELL)?;
    let instance = linker.instantiate_and_start(&mut store, &module)?;

    let hello = instance.get_typed_func::<(), ()>(&store, "main")?;
    hello.call(&mut store, ())?;

    let mut executor = SimpleExecutor::new();
    executor.spawn(Task::new(example_task()));
    executor.spawn(Task::new(tick_task()));
    executor.run();

    qemu::exit_qemu(qemu::QemuExitCode::Success);
    Ok(())
}

async fn async_number() -> u32 {
    42
}

async fn example_task() {
    let number = async_number().await;
    logln!("async number: {}", number);
}

async fn tick_task() {
    let ticks = timer::ticks().expect("nothing else is using the timer");
    let seen: Vec<u64> = ticks.take(3).collect().await;
    logln!("timer: ticks {:?}", seen);
}
