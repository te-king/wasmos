//! The boot sequence as a chain of typestates.
//!
//! Each stage is a token that only the previous stage can produce, so the
//! steps can only run in order:
//!
//! ```text
//! BootServices --exit()--> Heap<Bsp> --on_kernel_stack()--> Heap<Bsp>
//! Heap<R> --init_cpu()--> Interrupts<R>
//! Interrupts<Bsp> --start_clock()--> Clock
//! ```
//!
//! Code that needs a stage asks for its token. For example, `smp::discover`
//! takes `&BootServices`, so it can't run once `exit` has consumed it.
//!
//! A token's role says which processor it's on. The bootstrap processor
//! ([`Bsp`]) comes from the firmware. Application processors ([`Ap`]) are
//! only started once it has a clock, so they begin at `Heap`, made by the
//! trampoline with their id, and have no way to reach `Clock`.

use uart_16550::{Config, Uart16550Tty};
use uefi::mem::memory_map::MemoryType;
use x86_64::instructions::interrupts;

use super::{
    cpu::{self, CpuId, Local},
    gdt, int, mem, stack,
};
use crate::{log, logln};

/// Boot services are available.
pub struct BootServices(());

/// Boot services have been exited. The serial log and the full heap are up.
pub struct Heap<R>(R);

/// This processor has its own descriptor table and task state segment, its
/// per-CPU block (the token holds the proof, a [`Local`]), and interrupts
/// configured and enabled.
pub struct Interrupts<R>(R, Local);

/// The bootstrap processor, which the firmware runs the kernel on.
pub struct Bsp(());

/// An application processor, started by the bootstrap processor.
pub struct Ap(CpuId);

/// Which processor a stage is on.
pub trait Role {
    fn id(&self) -> CpuId;
}

impl Role for Bsp {
    fn id(&self) -> CpuId {
        CpuId::BSP
    }
}

impl Role for Ap {
    fn id(&self) -> CpuId {
        self.0
    }
}

/// This processor's local APIC timer drives the kernel's clock
/// ([`crate::timer`]). Only the bootstrap processor gets here.
pub struct Clock(Local);

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
    pub fn exit(self) -> Heap<Bsp> {
        // SAFETY: Consuming the token means nothing can use boot services
        // afterwards, and nothing borrowing it can still be alive. Code that
        // borrows it (like `smp::discover`) closes any protocol it opens.
        let memory_map =
            unsafe { uefi::boot::exit_boot_services(Some(MemoryType::RUNTIME_SERVICES_DATA)) };
        // The firmware's interrupt table stays loaded until `init_cpu`
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
        Heap(Bsp(()))
    }
}

impl Heap<Bsp> {
    /// Continues on a fresh kernel stack, for good.
    ///
    /// The firmware's stack is too small for the kernel (128 KiB under
    /// OVMF, where running a guest takes about 340 KiB) and has nothing
    /// guarding its bottom, below which the allocator may have claimed
    /// memory: overflowing it silently corrupts the heap. Application
    /// processors start on a kernel stack, so they have no need for this.
    pub fn on_kernel_stack(self, f: impl FnOnce(Heap<Bsp>) -> !) -> ! {
        stack::run_on(stack::leak::<{ stack::KERNEL_SIZE }>(), move || f(self))
    }
}

impl Heap<Ap> {
    /// The first stage on an application processor, which is only started
    /// after the bootstrap processor has exited boot services.
    ///
    /// # Safety
    /// Must be called once, on an application processor that has just
    /// entered the kernel, with the id the bootstrap processor gave it.
    pub unsafe fn application_processor(id: CpuId) -> Self {
        Heap(Ap(id))
    }
}

impl<R: Role> Heap<R> {
    /// Brings this processor online: its own descriptor table and task
    /// state segment, its per-CPU block, then the interrupt table, its local
    /// APIC (with the timer stopped) and interrupts.
    ///
    /// The order matters: the interrupt table's gates name the new table's
    /// selectors and interrupt stacks, and its handlers need the per-CPU
    /// block.
    pub fn init_cpu(self) -> Interrupts<R> {
        let id = self.0.id();
        // SAFETY: The heap is up, this is 64-bit ring 0 with interrupts
        // disabled (by `exit`, or the trampoline), and no interrupt table
        // naming the old selectors is loaded until below.
        unsafe { gdt::load_own() };
        // SAFETY: The heap is up, and consuming `Heap` means this runs once.
        let local = unsafe { cpu::init(id, int::local_apic()) };
        int::INTERRUPT_TABLE.load();
        // SAFETY: The interrupt table is loaded before any source of
        // interrupts is enabled.
        unsafe { int::install_local_apic(local) };
        interrupts::enable();
        logln!("cpu {id}: online");
        Interrupts(self.0, local)
    }
}

impl Interrupts<Bsp> {
    /// Starts this processor's local APIC timer as the kernel's clock. Every
    /// timer interrupt counts as a tick, so only the bootstrap processor
    /// runs its timer.
    pub fn start_clock(self) -> Clock {
        // SAFETY: Interrupts are set up on this processor, so the timer
        // handler has its interrupt table entry and per-CPU block.
        unsafe { int::start_timer(self.1) };
        Clock(self.1)
    }
}

impl Clock {
    /// This processor's proof of its per-CPU block.
    pub(super) fn local(&self) -> Local {
        self.0
    }
}
