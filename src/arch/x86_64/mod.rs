use uart_16550::SerialPort;
use uefi::{
    entry,
    table::{boot::MemoryType, Boot, SystemTable},
    Handle, Status,
};

use crate::{kernel_main, log};

use self::int::install_interrupt_table;

mod int;
mod io;
mod mem;

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use x86_64::instructions::hlt;

    use crate::logln;

    logln!("Kernel panic at: {:?}", info);

    loop {
        hlt()
    }
}

#[entry]
fn main(handle: Handle, system_table: SystemTable<Boot>) -> Status {
    unsafe {
        install_interrupt_table();
    }

    let (_system_table, memory_map) =
        system_table.exit_boot_services(MemoryType::RUNTIME_SERVICES_DATA);

    unsafe {
        log::install_stdio_port(SerialPort::new(0x03f8)).unwrap();
        mem::install_memory_map(memory_map);
    }

    match kernel_main() {
        Ok(_) => Status::SUCCESS,
        Err(_) => Status::UNSUPPORTED,
    }
}
