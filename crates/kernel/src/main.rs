#![feature(alloc_error_handler, abi_x86_interrupt, lazy_cell)]
#![no_main]
#![no_std]

extern crate alloc;

use sync::{executor::SimpleExecutor, task::Task};
use wasmi::{Caller, Engine, Func, Linker, Module, Store};

#[path = "arch/x86_64/mod.rs"]
mod arch;

mod log;
mod qemu;
mod sync;

const WSHELL: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_WSHELL"));

pub fn kernel_main() -> Result<(), ()> {
    let engine = Engine::default();
    let mut linker = Linker::<()>::new(&engine);
    let mut store = Store::<()>::new(&engine, ());

    let wasmos_print = Func::wrap(
        &mut store,
        |caller: Caller<'_, _>, offset: u32, length: u32| {
            let memory = caller
                .get_export("memory")
                .expect("'memory' export should exist")
                .into_memory()
                .expect("'memory' should be a memory");

            let mut buffer = alloc::vec![0u8; length as usize];
            memory.read(caller, offset as usize, &mut buffer);
            let s = core::str::from_utf8(&buffer).unwrap();
            logln!("{}", s);
        },
    );

    linker.define("host", "wasmos_print", wasmos_print);

    let host_hello = Func::wrap(&mut store, |parameter: i32| {
        logln!("Got {} from WebAssembly", parameter);
    });

    linker.define("host", "hello", host_hello).unwrap();

    // ceate an instance
    let module = Module::new(&engine, WSHELL).unwrap();
    let instance = linker
        .instantiate(&mut store, &module)
        .unwrap()
        .start(&mut store)
        .unwrap();

    let hello = instance.get_typed_func::<(), ()>(&store, "main").unwrap();
    hello.call(&mut store, ()).unwrap();

    let mut executor = SimpleExecutor::new();
    executor.spawn(Task::new(example_task()));
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
