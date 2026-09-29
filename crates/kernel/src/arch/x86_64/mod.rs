use alloc::boxed::Box;
use core::{error::Error, fmt::Display};

use uart_16550::{Uart16550Tty, backend::PioBackend};
use uefi::{Status, entry, runtime::ResetType};
use x86_64::instructions::{hlt, interrupts};

use crate::{executor::Executor, kernel_main, logln};

mod boot;
mod cpu;
mod exception;
mod gdt;
mod int;
mod mem;
mod panic;
mod qemu;
mod smp;
mod stack;
mod trampoline;

pub use boot::Clock;

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
    let trampoline = trampoline::Trampoline::reserve(&firmware);
    firmware
        .exit()
        .on_kernel_stack(move |heap| bsp_main(heap, processors, trampoline))
}

/// Where the bootstrap processor goes once it has left the firmware.
fn bsp_main(
    heap: boot::Heap<boot::Bsp>,
    processors: Result<smp::Processors, smp::DiscoveryError>,
    trampoline: uefi::Result<trampoline::Trampoline>,
) -> ! {
    let interrupts = heap.init_cpu().enable_interrupts();
    logln!("cpu {}: online", interrupts.id());
    let mut clock = interrupts.start_clock();

    match &processors {
        Ok(processors) => {
            logln!("smp: {processors}");
            for (id, cpu) in processors.iter().enumerate() {
                let role = if id == 0 { " (bsp)" } else { "" };
                logln!("smp: cpu {id}{role}: {cpu}");
            }
        }
        Err(err) => logln!("smp: {err}"),
    }

    finish(Executor::new().block_on(async {
        // Without discovery, the kernel carries on with this processor.
        if let Ok(processors) = &processors {
            let trampoline = trampoline.map_err(smp::StartError::Trampoline)?;
            smp::start(&mut clock, processors, trampoline, ap_main).await?;
        }
        kernel_main(&mut clock).await?;
        Ok::<_, Box<dyn Error>>(())
    }))
}

/// Where each application processor goes once it has entered the kernel.
fn ap_main(heap: boot::Heap<boot::Ap>) -> ! {
    let interrupts = heap.init_cpu().enable_interrupts();
    logln!("cpu {}: online", interrupts.id());
    // Its timer is stopped and nothing sends it IPIs yet, so this sleeps.
    loop {
        hlt();
    }
}

/// Ends the kernel with its result. This is the only place that decides
/// whether the kernel succeeded.
///
/// Boot services are gone, so there is no firmware left to return to. Under
/// QEMU the debug-exit port ends the emulator. Elsewhere, success powers the
/// machine off and failure halts, leaving the log readable.
fn finish(result: Result<(), impl Display>) -> ! {
    match result {
        Ok(()) => {
            qemu::exit_qemu(qemu::QemuExitCode::Success);
            uefi::runtime::reset(ResetType::SHUTDOWN, Status::SUCCESS, None)
        }
        Err(err) => {
            logln!("kernel: {err}");
            qemu::exit_qemu(qemu::QemuExitCode::Failed);
            halt()
        }
    }
}

/// Stops this processor for good.
fn halt() -> ! {
    interrupts::disable();
    // An NMI can still wake it.
    loop {
        hlt();
    }
}
