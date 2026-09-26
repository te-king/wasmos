#![feature(abi_x86_interrupt)]
#![no_main]
#![no_std]

extern crate alloc;

use sync::{executor::SimpleExecutor, task::Task};
use wasmi::{
    core::{Trap, TrapCode},
    Caller, Engine, Func, Linker, Module, Store,
};

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
        |caller: Caller<'_, _>, offset: u32, length: u32| -> Result<(), Trap> {
            // Bad input from the guest traps the guest rather than panicking the kernel.
            let memory = caller
                .get_export("memory")
                .and_then(|export| export.into_memory())
                .ok_or_else(|| Trap::new("wasmos_print: guest has no 'memory' export"))?;

            let mut buffer = alloc::vec![0u8; length as usize];
            memory
                .read(caller, offset as usize, &mut buffer)
                .map_err(|_| TrapCode::MemoryOutOfBounds)?;
            let s = core::str::from_utf8(&buffer)
                .map_err(|_| Trap::new("wasmos_print: string is not valid UTF-8"))?;
            logln!("{}", s);
            Ok(())
        },
    );

    linker.define("host", "wasmos_print", wasmos_print).unwrap();

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
