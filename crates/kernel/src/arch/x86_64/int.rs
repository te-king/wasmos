use spin::LazyLock;
use x2apic::lapic::{xapic_base, LocalApic, LocalApicBuilder};
use x86_64::{
    instructions::port::Port,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame},
};

use super::cpu;
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
pub unsafe fn local_apic() -> LocalApic {
    LocalApicBuilder::new()
        .timer_vector(InterruptIndex::Timer as usize)
        .error_vector(InterruptIndex::Error as usize)
        .spurious_vector(InterruptIndex::Spurious as usize)
        .set_xapic_base(xapic_base())
        .build()
        .unwrap()
}

/// Enables the current processor's local APIC, which also starts its timer.
pub unsafe fn install_local_apic() {
    cpu::with(|cpu| unsafe { cpu.lapic.borrow_mut().enable() });
}

/// Masks every line of the legacy 8259 PICs, which the firmware may have
/// left enabled, so that only the local APIC delivers interrupts.
pub unsafe fn disable_legacy_pic() {
    Port::<u8>::new(0x21).write(0xFF);
    Port::<u8>::new(0xA1).write(0xFF);
}

/// The interrupt table defines a set of functions that get called when
/// an interrupt is triggered.
static INTERRUPT_TABLE: LazyLock<InterruptDescriptorTable> = LazyLock::new(|| {
    let mut idt = InterruptDescriptorTable::new();
    idt.double_fault.set_handler_fn(double_fault_handler);
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt[InterruptIndex::Timer as u8].set_handler_fn(timer_handler);
    idt[InterruptIndex::Error as u8].set_handler_fn(error_handler);
    idt[InterruptIndex::Spurious as u8].set_handler_fn(spurious_handler);
    idt
});

/// Installs the default interrupt table for the current processor.
pub fn install_interrupt_table() {
    INTERRUPT_TABLE.load();
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    panic!(
        "EXCEPTION: DOUBLE FAULT\n{:#?}\n{}",
        stack_frame, error_code
    );
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    logln!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn timer_handler(_stack_frame: InterruptStackFrame) {
    crate::timer::tick();
    end_of_interrupt();
}

extern "x86-interrupt" fn error_handler(stack_frame: InterruptStackFrame) {
    logln!("ERROR:\n{:#?}", stack_frame);
    end_of_interrupt();
}

extern "x86-interrupt" fn spurious_handler(stack_frame: InterruptStackFrame) {
    // No end-of-interrupt: a spurious interrupt isn't marked in service, so
    // an EOI here would retire some other interrupt instead.
    logln!("SPURIOUS:\n{:#?}", stack_frame);
}

/// Tells the current processor's local APIC that the interrupt being handled
/// is finished, so it can deliver the next one.
fn end_of_interrupt() {
    // SAFETY: Only called at the end of handlers for interrupts that the
    // local APIC delivered (never for spurious interrupts; see above).
    cpu::with(|cpu| unsafe { cpu.lapic.borrow_mut().end_of_interrupt() });
}
