use core::fmt::Display;

use thiserror::Error;
use uart_16550::{Uart16550Tty, backend::PioBackend};
use uefi::{Status, entry, runtime::ResetType};
use x86_64::instructions::{hlt, interrupts};

use crate::{executor::Executor, kernel_main, log, logln, timer, work};

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
pub use cpu::CpuId;
pub use int::{WakeTarget, wake};

/// The kernel log's serial port: COM1, through port I/O.
pub type Console = Uart16550Tty<PioBackend>;

/// Whether this processor's interrupts are enabled.
pub fn interrupts_enabled() -> bool {
    interrupts::are_enabled()
}

/// Enables this processor's interrupts.
pub fn enable_interrupts() {
    interrupts::enable();
}

/// Disables this processor's interrupts.
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
    let startup = smp::prepare(&firmware);
    firmware.exit(move |heap| bsp_main(heap, startup))
}

/// Where the bootstrap processor goes once it has left the firmware.
fn bsp_main(heap: boot::Heap<boot::Bsp>, startup: Result<smp::Startup, smp::PrepareError>) -> ! {
    let mut clock = heap.init_cpu().start_clock();
    let local = clock.local();

    if let Ok(startup) = &startup {
        log!("{}", startup.processors);
    }

    finish(
        Executor::new(WakeTarget::current(local)).block_on(timer::serve(
            &mut clock,
            async |timer| {
                startup?.start(local, timer, ap_main).await?;
                kernel_main(timer).await?;
                Ok::<_, KernelError>(())
            },
        )),
    )
}

/// Why the kernel failed.
#[derive(Debug, Error)]
enum KernelError {
    /// The application processors couldn't be prepared for starting.
    #[error("smp: {0}")]
    Prepare(#[from] smp::PrepareError),
    #[error(transparent)]
    Start(#[from] smp::Timeout),
    #[error(transparent)]
    Kernel(#[from] crate::Error),
}

/// Where each application processor goes once it has entered the kernel.
fn ap_main(heap: boot::Heap<boot::Ap>) -> ! {
    let local = heap.init_cpu().local();
    // Its timer is stopped, so it sleeps until a job comes, with a wake-up.
    match Executor::new(WakeTarget::current(local)).block_on(work::serve(local.id())) {}
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
            fail()
        }
    }
}

/// Ends the kernel in failure, once the failure has been logged: under QEMU
/// the debug-exit port ends the emulator, and elsewhere this processor halts.
fn fail() -> ! {
    qemu::exit_qemu(qemu::QemuExitCode::Failed);
    halt()
}

/// Stops this processor for good.
fn halt() -> ! {
    interrupts::disable();
    // An NMI can still wake it.
    loop {
        hlt();
    }
}
