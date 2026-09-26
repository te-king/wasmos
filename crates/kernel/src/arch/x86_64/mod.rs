use core::fmt::Display;

use uefi::{entry, Status};

use crate::{executor, kernel_main, logln, qemu};

mod boot;
mod cpu;
mod int;
mod io;
mod mem;
mod panic;
mod smp;

#[entry]
fn main() -> Status {
    // SAFETY: This is the UEFI entry point, and nothing has used boot
    // services yet.
    let firmware = unsafe { boot::BootServices::start() };
    let processors = smp::discover(&firmware);
    let _interrupts = firmware.exit().init_cpu(0).enable_interrupts();

    cpu::with(|cpu| logln!("cpu {}: online", cpu.id));
    match &processors {
        Ok(processors) => {
            logln!("smp: {}", processors);
            for (id, cpu) in processors.iter().enumerate() {
                let role = if id == 0 { " (bsp)" } else { "" };
                logln!("smp: cpu {}{}: {}", id, role, cpu);
            }
        }
        Err(err) => logln!("smp: {}", err),
    }

    report(executor::block_on(kernel_main()))
}

/// Reports the kernel's result, to QEMU through its debug-exit port and to
/// the firmware as the entry point's status. This is the only place that
/// decides whether the kernel succeeded.
fn report(result: Result<(), impl Display>) -> Status {
    match result {
        Ok(()) => {
            qemu::exit_qemu(qemu::QemuExitCode::Success);
            Status::SUCCESS
        }
        Err(err) => {
            logln!("kernel: {}", err);
            qemu::exit_qemu(qemu::QemuExitCode::Failed);
            Status::UNSUPPORTED
        }
    }
}
