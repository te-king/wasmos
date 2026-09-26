use uart_16550::{Config, Uart16550Tty};
use uefi::{boot, entry, mem::memory_map::MemoryType, Status};

use crate::{kernel_main, log, qemu};

mod int;
mod io;
mod mem;
mod panic;

#[entry]
fn main() -> Status {
    // SAFETY: Nothing has used boot services yet, so no references to
    // boot-services resources remain.
    let memory_map = unsafe { boot::exit_boot_services(Some(MemoryType::RUNTIME_SERVICES_DATA)) };

    unsafe {
        let serial = Uart16550Tty::new_port(0x03f8, Config::default()).unwrap();
        log::install_stdio_port(serial).unwrap();
        mem::install_memory_map(memory_map);
        int::install_interrupt_table();
        int::install_local_apic();
    }

    match kernel_main() {
        Ok(_) => Status::SUCCESS,
        Err(_) => {
            qemu::exit_qemu(qemu::QemuExitCode::Failed);
            Status::UNSUPPORTED
        }
    }
}
