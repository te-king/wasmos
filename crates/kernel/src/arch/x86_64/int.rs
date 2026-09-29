use core::{
    arch::x86_64::__cpuid,
    sync::atomic::{Ordering, fence},
};

use spin::LazyLock;
use x2apic::lapic::{LocalApic, LocalApicBuilder, xapic_base};
use x86_64::{
    instructions::port::Port,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

use super::{cpu, exception};
use crate::logln;

#[repr(u8)]
enum InterruptIndex {
    Timer = 32,
    Error = 33,
    Spurious = 34,
}

/// Builds a handle to the current processor's local APIC, which controls
/// interrupt handling for that processor. It belongs in the processor's
/// [`cpu::Cpu`] block.
///
/// # Safety
/// Reads the `IA32_APIC_BASE` MSR, so it must run in kernel mode on the
/// processor the handle is for.
pub unsafe fn local_apic() -> LocalApic {
    // SAFETY: The caller guarantees kernel mode on the target processor.
    let xapic_base = unsafe { xapic_base() };
    LocalApicBuilder::new()
        .timer_vector(InterruptIndex::Timer as usize)
        .error_vector(InterruptIndex::Error as usize)
        .spurious_vector(InterruptIndex::Spurious as usize)
        .set_xapic_base(xapic_base)
        .build()
        .unwrap()
}

/// Enables the current processor's local APIC, with its timer stopped.
///
/// # Safety
/// The processor's per-CPU block must exist and the interrupt table must be
/// loaded, since interrupts can arrive as soon as the APIC is enabled.
pub unsafe fn install_local_apic() {
    cpu::with(|cpu| {
        let mut lapic = cpu.lapic.borrow_mut();
        // SAFETY: The caller guarantees interrupts can be handled.
        unsafe {
            // `enable` also starts the timer, on application processors
            // too, but only the clock's processor may run it.
            lapic.enable();
            lapic.disable_timer();
        }
    });
}

/// Starts the current processor's local APIC timer.
///
/// # Safety
/// The local APIC must be enabled, by [`install_local_apic`].
pub unsafe fn start_timer() {
    // SAFETY: The caller guarantees the timer's interrupts can be handled.
    cpu::with(|cpu| unsafe { cpu.lapic.borrow_mut().enable_timer() });
}

/// A processor's address for inter-processor interrupts, in the form that
/// x2apic's IPI functions take.
#[derive(Clone, Copy, Debug)]
pub struct IpiDestination(u32);

/// The IPI destination of the processor with local APIC ID `apic_id`, or
/// `None` if the local APIC's mode can't address it.
///
/// x2apic writes the destination into the upper half of the ICR as is. That
/// is the whole field in x2APIC mode, but xAPIC mode only reads its top
/// byte, so there the ID has to be shifted into place.
pub fn ipi_destination(apic_id: u64) -> Option<IpiDestination> {
    if has_x2apic() {
        u32::try_from(apic_id).ok().map(IpiDestination)
    } else {
        u8::try_from(apic_id)
            .ok()
            .map(|id| IpiDestination(u32::from(id) << 24))
    }
}

/// Whether the local APIC is in x2APIC mode, which x2apic's builder picks
/// exactly when the processor supports it.
fn has_x2apic() -> bool {
    __cpuid(1).ecx & (1 << 21) != 0
}

/// Sends an INIT IPI, which resets `dest` and leaves it waiting for a
/// startup IPI.
///
/// # Safety
/// `dest` must not be running anything: INIT stops it wherever it is.
pub unsafe fn send_init(dest: IpiDestination) {
    // SAFETY: The caller guarantees that resetting `dest` is harmless.
    cpu::with(|cpu| unsafe { cpu.lapic.borrow_mut().send_init_ipi(dest.0) });
}

/// Sends a startup IPI, which starts `dest` in real mode at the start of
/// page `vector`, if it is waiting for one.
///
/// Everything written before this call is visible to `dest` when it starts.
///
/// # Safety
/// That page must hold code for a starting processor to run.
pub unsafe fn send_startup(dest: IpiDestination, vector: u8) {
    // In x2APIC mode the IPI is sent by a WRMSR, which doesn't wait for
    // earlier stores (such as the startup code's data) to become visible.
    fence(Ordering::SeqCst);
    // SAFETY: The caller guarantees the page holds startup code.
    cpu::with(|cpu| unsafe { cpu.lapic.borrow_mut().send_sipi(vector, dest.0) });
}

/// Masks every line of the legacy 8259 PICs, which the firmware may have
/// left enabled, so that only the local APIC delivers interrupts.
///
/// # Safety
/// Writes to the PICs' I/O ports, so nothing else may be driving them.
pub unsafe fn disable_legacy_pic() {
    // SAFETY: 0x21 and 0xA1 are the PICs' data ports, where writing all ones
    // masks every line; the caller guarantees nothing else drives them.
    unsafe {
        Port::<u8>::new(0x21).write(0xFF);
        Port::<u8>::new(0xA1).write(0xFF);
    }
}

/// Every processor's interrupt table: the exception handlers, plus the
/// local APIC's interrupts.
static INTERRUPT_TABLE: LazyLock<InterruptDescriptorTable> = LazyLock::new(|| {
    let mut idt = exception::table();
    exception::gate(&mut idt[InterruptIndex::Timer as u8], timer_handler);
    exception::gate(&mut idt[InterruptIndex::Error as u8], error_handler);
    exception::gate(&mut idt[InterruptIndex::Spurious as u8], spurious_handler);
    idt
});

/// Loads the interrupt table on the current processor.
///
/// Its gates run handlers on the kernel's code segment and the task state
/// segment's interrupt stacks, so it takes the `PerCpu` stage's word (see
/// `boot`) that this processor has its own descriptor table loaded.
pub fn install_interrupt_table() {
    INTERRUPT_TABLE.load();
}

extern "x86-interrupt" fn timer_handler(_stack_frame: InterruptStackFrame) {
    crate::timer::tick();
    end_of_interrupt();
}

extern "x86-interrupt" fn error_handler(stack_frame: InterruptStackFrame) {
    logln!("ERROR:\n{stack_frame:#?}");
    end_of_interrupt();
}

extern "x86-interrupt" fn spurious_handler(stack_frame: InterruptStackFrame) {
    // No end-of-interrupt: a spurious interrupt isn't marked in service, so
    // an EOI here would retire some other interrupt instead.
    logln!("SPURIOUS:\n{stack_frame:#?}");
}

/// Tells the current processor's local APIC that the interrupt being handled
/// is finished, so it can deliver the next one.
fn end_of_interrupt() {
    // SAFETY: Only called at the end of handlers for interrupts that the
    // local APIC delivered (never for spurious interrupts; see above).
    cpu::with(|cpu| unsafe { cpu.lapic.borrow_mut().end_of_interrupt() });
}
