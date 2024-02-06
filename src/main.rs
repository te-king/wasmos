#![feature(alloc_error_handler)]
#![no_main]
#![no_std]

extern crate alloc;

use uefi::prelude::*;
use uefi_services::println;
use wasmi::{Caller, Engine, Func, Linker, Module, Store};

mod qemu;

const WSHELL: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_WSHELL"));

#[entry]
fn main(_handle: Handle, mut system_table: SystemTable<Boot>) -> Status {
    // retrieve memory map, exit boot services, and add to allocator
    // let (mut system_table, memory_map) = system_table //
    // .exit_boot_services();

    uefi_services::init(&mut system_table).unwrap();

    let engine = Engine::default();
    let mut linker = <Linker<()>>::new(&engine);

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
            println!("{}", s);
        },
    );

    linker.define("host", "wasmos_print", wasmos_print);

    let host_hello = Func::wrap(&mut store, |parameter: i32| {
        println!("Got {parameter} from WebAssembly");
    });

    linker.define("host", "hello", host_hello).unwrap();
    // let instance = linker
    //     .instantiate(&mut store, &module)
    //     .unwrap()
    //     .start(&mut store)
    //     .unwrap();
    // let hello = host_hello.typed::<(i32), ()>(&mut store)?;

    // let instance = linker
    // .instantiate(&mut store, &module)?
    // .start(&mut store)
    // .unwrap();
    // let hello = instance.get_typed_func::<(), ()>(&store, "hello").unwrap();

    // ceate an instance
    let module = Module::new(&engine, WSHELL).unwrap();
    let instance = linker
        .instantiate(&mut store, &module)
        .unwrap()
        .start(&mut store)
        .unwrap();

    let hello = instance.get_typed_func::<(), ()>(&store, "main").unwrap();
    hello.call(&mut store, ()).unwrap();

    qemu::exit_qemu(qemu::QemuExitCode::Success);
    Status::SUCCESS
}
