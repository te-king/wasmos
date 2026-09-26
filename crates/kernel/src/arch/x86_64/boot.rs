//! The boot sequence as a chain of typestates.
//!
//! Each stage is a zero-sized token that only the previous stage can produce,
//! so the steps can only run in order:
//!
//! ```text
//! BootServices --exit()--> Heap --init_cpu()--> PerCpu --enable_interrupts()--> Interrupts
//! ```
//!
//! Code that needs a stage asks for its token. For example, `smp::discover`
//! takes `&BootServices`, so it can't run once `exit` has consumed it.

use uart_16550::{Config, Uart16550Tty};
use uefi::mem::memory_map::MemoryType;
use x86_64::instructions::interrupts;

use super::{cpu, int, mem};
use crate::log;

/// Boot services are available.
pub struct BootServices(());

/// Boot services have been exited. The serial log and the full heap are up.
pub struct Heap(());

/// This processor's per-CPU block exists, so `cpu::with` is sound.
pub struct PerCpu(());

/// Interrupts are configured and enabled on this processor.
pub struct Interrupts(());

impl BootServices {
    /// The first stage.
    ///
    /// # Safety
    /// Must be called once, at the UEFI entry point, before anything has
    /// used boot services.
    pub unsafe fn start() -> Self {
        BootServices(())
    }

    /// Exits boot services, then brings up the serial log and gives all
    /// conventional memory to the allocator.
    pub fn exit(self) -> Heap {
        // SAFETY: Consuming the token means nothing can use boot services
        // afterwards, and nothing borrowing it can still be alive. Code that
        // borrows it (like `smp::discover`) closes any protocol it opens.
        let memory_map =
            unsafe { uefi::boot::exit_boot_services(Some(MemoryType::RUNTIME_SERVICES_DATA)) };

        // SAFETY: COM1 is the standard serial port, and nothing else drives it.
        let serial = unsafe { Uart16550Tty::new_port(0x03f8, Config::default()) }.unwrap();
        log::install_stdio_port(serial).unwrap();

        // SAFETY: The memory map was just returned by exiting boot services,
        // so its conventional regions are free for the allocator.
        unsafe { mem::install_memory_map(memory_map) };
        Heap(())
    }
}

impl Heap {
    /// Sets up this processor's per-CPU block, including its local APIC.
    pub fn init_cpu(self, id: u32) -> PerCpu {
        // SAFETY: The heap is up, and consuming `Heap` means this runs once,
        // before anything could call `cpu::with`.
        unsafe { cpu::init(id, int::local_apic()) };
        PerCpu(())
    }
}

impl PerCpu {
    /// Loads the interrupt table, masks the legacy PIC, enables the local
    /// APIC (which starts its timer) and enables interrupts.
    pub fn enable_interrupts(self) -> Interrupts {
        int::install_interrupt_table();
        // SAFETY: The per-CPU block that the handlers reach through
        // `cpu::with` exists, and the IDT is loaded before any source of
        // interrupts is enabled.
        unsafe {
            int::disable_legacy_pic();
            int::install_local_apic();
        }
        interrupts::enable();
        Interrupts(())
    }
}
