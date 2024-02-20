use uart_16550::SerialPort;
use uefi::{
    entry,
    table::{boot::MemoryType, Boot, SystemTable},
    Handle, Status,
};
use x2apic::lapic::xapic_base;
use x86_64::instructions::interrupts::enable_and_hlt;

use crate::{kernel_main, log};

mod int;
mod io;
mod mem;
mod panic;

#[entry]
fn main(handle: Handle, system_table: SystemTable<Boot>) -> Status {
    let (_system_table, memory_map) =
        system_table.exit_boot_services(MemoryType::RUNTIME_SERVICES_DATA);

    unsafe {
        log::install_stdio_port(SerialPort::new(0x03f8)).unwrap();
        mem::install_memory_map(memory_map);
        int::install_interrupt_table();
        int::install_local_apic();
    }

    match kernel_main() {
        Ok(_) => Status::SUCCESS,
        Err(_) => Status::UNSUPPORTED,
    }
}
