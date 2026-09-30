//! Per-processor data, the kernel's equivalent of thread-local storage.
//!
//! Each processor owns one [`Cpu`] block and points its GS segment base at
//! it, so `gs:[0]` always leads to the current processor's block. A block is
//! only ever touched by its own processor, which is why its contents don't
//! need to be `Send` or `Sync`.
//!
//! Code running on the processor reaches its block through the [`Local`]
//! handle it was brought online with. Interrupt handlers, which can't be
//! handed one, go through the GS base instead ([`with_lapic`]).

use alloc::boxed::Box;
use core::{arch::asm, cell::RefCell, ops::Deref, ptr};

use x2apic::lapic::LocalApic;
use x86_64::{VirtAddr, instructions::interrupts, registers::model_specific::GsBase};

/// Data owned by a single processor.
#[repr(C)]
pub struct Cpu {
    /// Points back at this block. Must stay the first field: [`with`] finds
    /// the block by reading `gs:[0]`.
    this: *const Cpu,
    /// Logical index of this processor. The bootstrap processor is 0.
    pub id: u32,
    /// This processor's local APIC. Interrupt handlers use it too, so it's
    /// only reached through [`with_lapic`].
    lapic: RefCell<LocalApic>,
}

/// The current processor's [`Cpu`] block, and proof that it exists.
///
/// It's neither `Send` nor `Sync` (the block isn't `Sync`), so it can't
/// leave its processor, and neither can a future that holds it.
pub struct Local(&'static Cpu);

impl Deref for Local {
    type Target = Cpu;

    fn deref(&self) -> &Cpu {
        self.0
    }
}

/// Allocates the current processor's block and points its GS base at it.
///
/// # Safety
/// Must be called exactly once on each processor, after the heap is set up
/// and before anything on that processor calls [`with_lapic`]. Nothing may
/// change the GS base afterwards.
pub unsafe fn init(id: u32, lapic: LocalApic) -> Local {
    let cpu = Box::leak(Box::new(Cpu {
        this: ptr::null(),
        id,
        lapic: RefCell::new(lapic),
    }));
    cpu.this = &raw const *cpu;
    GsBase::write(VirtAddr::from_ptr(cpu));
    Local(cpu)
}

/// Runs `f` with the current processor's local APIC.
///
/// Interrupts are disabled while `f` runs, so an interrupt handler on this
/// processor can't find the APIC already borrowed. And because `f` is a
/// plain closure, the borrow can't be held across an `.await`.
///
/// Calling it before [`init`] on this processor is undefined behaviour. The
/// boot sequence rules that out: `init` runs before the interrupt table,
/// whose handlers use this, is loaded.
pub fn with_lapic<R>(f: impl FnOnce(&mut LocalApic) -> R) -> R {
    with(|cpu| f(&mut cpu.lapic.borrow_mut()))
}

/// Runs `f` with the current processor's block, found through its GS base,
/// with interrupts disabled.
fn with<R>(f: impl FnOnce(&Cpu) -> R) -> R {
    interrupts::without_interrupts(|| {
        let cpu: *const Cpu;
        // SAFETY: `init` pointed this processor's GS base at its `Cpu` block,
        // which starts with a pointer to itself and is never freed.
        unsafe {
            asm!("mov {}, gs:[0]", out(reg) cpu, options(nostack, readonly, preserves_flags));
            f(&*cpu)
        }
    })
}
