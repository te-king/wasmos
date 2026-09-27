use core::fmt::Display;

use uart_16550::{backend::PioBackend, Uart16550Tty};
use uefi::{entry, Status};
use x86_64::instructions::interrupts;

use crate::{executor, kernel_main, logln};

mod boot;
mod cpu;
mod int;
mod mem;
mod panic;
mod qemu;
mod smp;

/// The kernel log's serial port: COM1, through port I/O.
pub type Console = Uart16550Tty<PioBackend>;

/// Runs `f` with this processor's interrupts disabled, then restores them.
pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    interrupts::without_interrupts(f)
}

/// Disables this processor's interrupts for good, e.g. when panicking.
pub fn disable_interrupts() {
    interrupts::disable();
}

/// Halts until the next interrupt, unless `ready()` returns true.
///
/// `ready` runs with interrupts disabled, and `enable_and_hlt` re-enables
/// them atomically with the halt, so an interrupt that would make it true
/// can't slip in between the check and the halt. Interrupts are enabled when
/// this returns.
pub fn wait_for_interrupt(ready: impl FnOnce() -> bool) {
    interrupts::disable();
    if ready() {
        interrupts::enable();
    } else {
        interrupts::enable_and_hlt();
    }
}

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
