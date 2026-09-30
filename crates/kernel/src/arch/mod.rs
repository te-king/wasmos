//! Architecture support.
//!
//! Each architecture has its own module, which owns the entry point, the boot
//! sequence, interrupt handling and the panic handler. The rest of the kernel
//! only uses the interface re-exported here, never an architecture's crates
//! directly, so a new architecture must provide exactly these items:
//!
//! - `Console`: the serial port type the kernel log writes to.
//! - `Clock`: proof that the kernel's clock is running, so `timer::tick`
//!   is being called. Only the boot sequence can make one.
//! - `interrupts_enabled`, `enable_interrupts`, `disable_interrupts`: this
//!   processor's interrupt masking, which `sync::IrqMutex` saves and
//!   restores.
//! - `wait_for_interrupt`: the executor's idle loop.

#[cfg(target_arch = "x86_64")]
#[path = "x86_64/mod.rs"]
mod imp;

#[cfg(not(target_arch = "x86_64"))]
compile_error!("wasmos only supports x86_64 so far (aarch64 is planned)");

pub use imp::{
    Clock, Console, disable_interrupts, enable_interrupts, interrupts_enabled, wait_for_interrupt,
};
