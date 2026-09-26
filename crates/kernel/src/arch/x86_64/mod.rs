use uart_16550::{Config, Uart16550Tty};
use uefi::{boot, entry, mem::memory_map::MemoryType, Status};
use x86_64::instructions::interrupts;

use crate::{kernel_main, log, logln, qemu};

mod cpu;
mod int;
mod io;
mod mem;
mod panic;
mod smp;

#[entry]
fn main() -> Status {
    // MP Services is a boot service, so this has to happen first.
    let processors = smp::discover();

    // SAFETY: Discovery closed the protocol it opened, and nothing else has
    // used boot services, so no references to boot-services resources remain.
    let memory_map = unsafe { boot::exit_boot_services(Some(MemoryType::RUNTIME_SERVICES_DATA)) };

    unsafe {
        let serial = Uart16550Tty::new_port(0x03f8, Config::default()).unwrap();
        log::install_stdio_port(serial).unwrap();
        mem::install_memory_map(memory_map);
        cpu::init(0, int::local_apic());
        int::install_interrupt_table();
        int::disable_legacy_pic();
        int::install_local_apic();
    }
    interrupts::enable();
    cpu::with(|cpu| logln!("cpu {}: online", cpu.id));
    match processors {
        Ok(processors) => processors.log(),
        Err(err) => logln!("smp: MP Services unavailable: {:?}", err),
    }

    match kernel_main() {
        Ok(_) => Status::SUCCESS,
        Err(_) => {
            qemu::exit_qemu(qemu::QemuExitCode::Failed);
            Status::UNSUPPORTED
        }
    }
}
