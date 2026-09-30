//! The boot sequence as typestates.
//!
//! Each processor goes from offline to online. The bootstrap processor gets
//! there from the firmware, and application processors from the trampoline:
//!
//! ```text
//! BootServices --exit()--> Bsp --online()--> (Local, Ticks)
//!   (trampoline::enter)    Ap  --online()--> Local
//! ```
//!
//! Each token can only be made by the step before it and is consumed by the
//! step after, so the steps run in order and once. Code that needs a stage
//! asks for its token. For example, `smp::discover` takes `&BootServices`,
//! so it can't run once `exit` has consumed it.

use uart_16550::{Config, Uart16550Tty};
use uefi::mem::memory_map::MemoryType;
use x86_64::instructions::interrupts;

use super::{
    cpu::{self, Local},
    int, mem,
};
use crate::{log, timer::Ticks};

/// Boot services are available.
pub struct BootServices(());

/// The bootstrap processor, offline. Boot services have been exited, and the
/// serial log and the full heap are up.
pub struct Bsp(());

/// An application processor that has just entered the kernel, offline.
pub struct Ap {
    id: u32,
}

impl BootServices {
    /// The first stage.
    ///
    /// # Safety
    /// Must be called once, at the UEFI entry point, before anything has
    /// used boot services.
    pub unsafe fn start() -> Self {
        BootServices(())
    }

    /// Exits boot services, masks the legacy PIC, then brings up the serial
    /// log and gives all conventional memory to the allocator.
    pub fn exit(self) -> Bsp {
        // SAFETY: Consuming the token means nothing can use boot services
        // afterwards, and nothing borrowing it can still be alive. Code that
        // borrows it (like `smp::discover`) closes any protocol it opens.
        let memory_map =
            unsafe { uefi::boot::exit_boot_services(Some(MemoryType::RUNTIME_SERVICES_DATA)) };

        // SAFETY: With boot services gone, nothing else drives the PICs.
        unsafe { int::disable_legacy_pic() };

        // SAFETY: COM1 is the standard serial port, and nothing else drives it.
        let serial = unsafe { Uart16550Tty::new_port(0x03f8, Config::default()) }.unwrap();
        log::install_stdio_port(serial);

        // SAFETY: The memory map was just returned by exiting boot services,
        // so its conventional regions are free for the allocator.
        unsafe { mem::install_memory_map(memory_map) };
        Bsp(())
    }
}

impl Bsp {
    /// Brings the bootstrap processor online, then starts its local APIC
    /// timer as the kernel's clock. It's the only processor that runs its
    /// timer, since every timer interrupt counts as a tick.
    pub fn online(self) -> (Local, Ticks) {
        let local = online(0);
        // SAFETY: Interrupts are set up on this processor, so the timer
        // handler has its interrupt table entry and per-CPU block.
        unsafe { int::start_timer() };
        // SAFETY: Consuming `Bsp` means this runs once.
        (local, unsafe { Ticks::new() })
    }
}

impl Ap {
    /// Application processor `id`, which has just entered the kernel. They
    /// are only started after the bootstrap processor has exited boot
    /// services, so the heap is already up.
    ///
    /// # Safety
    /// Must be called once, on that processor.
    pub unsafe fn arrived(id: u32) -> Self {
        Ap { id }
    }

    /// Brings this application processor online.
    pub fn online(self) -> Local {
        online(self.id)
    }
}

/// Sets up the current processor's per-CPU block, then its interrupt table
/// and local APIC, with the timer stopped, and enables interrupts.
fn online(id: u32) -> Local {
    // SAFETY: The heap is up, and consuming an offline token means this runs
    // once on this processor, before anything could use its block. We're in
    // kernel mode on the processor the local APIC handle is for.
    let local = unsafe { cpu::init(id, int::local_apic()) };
    int::install_interrupt_table();
    // SAFETY: The per-CPU block that the handlers reach through
    // `cpu::with_lapic` exists, and the IDT is loaded before any source of
    // interrupts is enabled.
    unsafe { int::install_local_apic() };
    interrupts::enable();
    local
}
