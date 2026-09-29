//! Per-processor data, the kernel's equivalent of thread-local storage.
//!
//! Each processor owns one [`Cpu`] block and points its GS segment base at
//! it, so `gs:[0]` always leads to the current processor's block. A block is
//! only ever touched by its own processor, which is why its contents don't
//! need to be `Send` or `Sync`.

use alloc::boxed::Box;
use core::{arch::asm, cell::RefCell, fmt};

use x2apic::lapic::LocalApic;
use x86_64::{VirtAddr, instructions::interrupts, registers::model_specific::GsBase};

/// A processor's logical index: the bootstrap processor is 0, and the
/// application processors follow in the order the firmware lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuId(u32);

impl CpuId {
    pub const BSP: CpuId = CpuId(0);

    /// The `index`th processor the firmware lists, counting the bootstrap
    /// processor as 0.
    pub const fn nth(index: u32) -> Self {
        CpuId(index)
    }
}

impl fmt::Display for CpuId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Data owned by a single processor.
#[repr(C)]
pub struct Cpu {
    /// Points back at this block. Must stay the first field: [`with`] finds
    /// the block by reading `gs:[0]`.
    this: *const Cpu,
    pub id: CpuId,
    /// This processor's local APIC.
    pub lapic: RefCell<LocalApic>,
}

/// Allocates the current processor's block and points its GS base at it.
///
/// # Safety
/// Must be called exactly once on each processor, after the heap is set up
/// and before anything on that processor calls [`with`]. Nothing may change
/// the GS base afterwards.
pub unsafe fn init(id: CpuId, lapic: LocalApic) {
    // Allocated first, so the block is built whole, pointing at itself.
    let block = Box::leak(Box::<Cpu>::new_uninit());
    let this = block.as_ptr();
    let cpu = block.write(Cpu {
        this,
        id,
        lapic: RefCell::new(lapic),
    });
    GsBase::write(VirtAddr::from_ptr(cpu));
}

/// Runs `f` with the current processor's data.
///
/// Interrupts are disabled while `f` runs, so an interrupt handler on this
/// processor can't find one of its `RefCell`s already borrowed. And because
/// `f` is a plain closure, the reference can't be held across an `.await`,
/// after which a task could be resumed on a different processor.
pub fn with<R>(f: impl FnOnce(&Cpu) -> R) -> R {
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

/// Runs `f` with the current processor's local APIC.
pub fn with_lapic<R>(f: impl FnOnce(&mut LocalApic) -> R) -> R {
    with(|cpu| f(&mut cpu.lapic.borrow_mut()))
}
