use uart_16550::SerialPort;
use uefi::{
    entry,
    table::{boot::MemoryType, Boot, SystemTable},
    Handle, Status,
};

use crate::{kernel_main, log};

mod io;
mod mem;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[entry]
fn main(_handle: Handle, system_table: SystemTable<Boot>) -> Status {
    let (_system_table, memory_map) =
        system_table.exit_boot_services(MemoryType::RUNTIME_SERVICES_DATA);

    unsafe {
        log::install_stdio_port(SerialPort::new(0x03f8)).unwrap();
        mem::install_memory_map(memory_map);
    }

    match kernel_main() {
        Ok(_) => Status::SUCCESS,
        Err(_) => Status::ABORTED,
    }
}
