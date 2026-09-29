//! The boot sequence as a chain of typestates.
//!
//! Each stage is a zero-sized token that only the previous stage can produce,
//! so the steps can only run in order:
//!
//! ```text
//! BootServices --exit()--> Heap --init_cpu()--> PerCpu --enable_interrupts()--> Interrupts
//!                                                              (bootstrap only) --start_clock()--> Clock
//! ```
//!
//! Code that needs a stage asks for its token. For example, `smp::discover`
//! takes `&BootServices`, so it can't run once `exit` has consumed it.
//!
//! Application processors are only started once the bootstrap processor has
//! a clock, so they begin at `Heap` and stop at `Interrupts`.

use uart_16550::{Config, Uart16550Tty};
use uefi::mem::memory_map::MemoryType;
use x86_64::instructions::interrupts;

use super::{cpu, gdt, int, mem, stack};
use crate::log;

/// Boot services are available.
pub struct BootServices(());

/// Boot services have been exited. The serial log and the full heap are up.
pub struct Heap(());

/// This processor has its own descriptor table and task state segment, and
/// its per-CPU block exists, so `cpu::with` is sound.
pub struct PerCpu(());

/// Interrupts are configured and enabled on this processor.
pub struct Interrupts(());

/// This processor's local APIC timer drives the kernel's clock
/// ([`crate::timer`]). Only the bootstrap processor gets here.
pub struct Clock(());

impl BootServices {
    /// The first stage.
    ///
    /// # Safety
    /// Must be called once, at the UEFI entry point, before anything has
    /// used boot services.
    pub unsafe fn start() -> Self {
        BootServices(())
    }

    /// Exits boot services with interrupts disabled, masks the legacy PIC,
    /// then brings up the serial log and gives all conventional memory to
    /// the allocator.
    pub fn exit(self) -> Heap {
        // SAFETY: Consuming the token means nothing can use boot services
        // afterwards, and nothing borrowing it can still be alive. Code that
        // borrows it (like `smp::discover`) closes any protocol it opens.
        let memory_map =
            unsafe { uefi::boot::exit_boot_services(Some(MemoryType::RUNTIME_SERVICES_DATA)) };
        // The firmware's interrupt table stays loaded until `enable_interrupts`
        // replaces it, and it names the firmware's selectors, which
        // `init_cpu` replaces first.
        interrupts::disable();

        // SAFETY: With boot services gone, nothing else drives the PICs.
        unsafe { int::disable_legacy_pic() };

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
    /// Continues on a fresh kernel stack, for good.
    ///
    /// The firmware's stack is too small for the kernel (128 KiB under
    /// OVMF, where running a guest takes about 340 KiB) and has nothing
    /// guarding its bottom, below which the allocator may have claimed
    /// memory: overflowing it silently corrupts the heap. Application
    /// processors start on a kernel stack, so only the bootstrap processor
    /// needs this.
    pub fn on_kernel_stack(self, f: impl FnOnce(Heap) -> !) -> ! {
        stack::run_on(stack::leak::<{ stack::KERNEL_SIZE }>(), move || f(self))
    }

    /// The first stage on an application processor, which is only started
    /// after the bootstrap processor has exited boot services.
    ///
    /// # Safety
    /// Must be called once, on an application processor that has just
    /// entered the kernel.
    pub unsafe fn application_processor() -> Self {
        Heap(())
    }

    /// Gives this processor its own descriptor table and task state
    /// segment, then sets up its per-CPU block, including its local APIC.
    pub fn init_cpu(self, id: u32) -> PerCpu {
        // SAFETY: The heap is up, this is 64-bit ring 0 with interrupts
        // disabled (by `exit`, or the trampoline), and the interrupt table
        // that would name the old selectors isn't loaded until
        // `enable_interrupts`.
        unsafe { gdt::load_own() };
        // SAFETY: The heap is up, and consuming `Heap` means this runs once,
        // before anything could call `cpu::with`.
        unsafe { cpu::init(id, int::local_apic()) };
        PerCpu(())
    }
}

impl PerCpu {
    /// Loads the interrupt table, enables the local APIC with its timer
    /// stopped, and enables interrupts.
    ///
    /// The table's gates use this processor's descriptor table and
    /// interrupt stacks, which is why this needs `PerCpu`.
    pub fn enable_interrupts(self) -> Interrupts {
        int::install_interrupt_table();
        // SAFETY: The per-CPU block that the handlers reach through
        // `cpu::with` exists, and the IDT is loaded before any source of
        // interrupts is enabled.
        unsafe { int::install_local_apic() };
        interrupts::enable();
        Interrupts(())
    }
}

impl Interrupts {
    /// Starts this processor's local APIC timer as the kernel's clock.
    ///
    /// # Panics
    /// If this isn't the bootstrap processor. Every timer interrupt counts
    /// as a tick, so only one processor may run its timer.
    pub fn start_clock(self) -> Clock {
        let id = cpu::with(|cpu| cpu.id);
        assert_eq!(id, 0, "only the bootstrap processor runs the clock");
        // SAFETY: Interrupts are set up on this processor, so the timer
        // handler has its interrupt table entry and per-CPU block.
        unsafe { int::start_timer() };
        Clock(())
    }
}
