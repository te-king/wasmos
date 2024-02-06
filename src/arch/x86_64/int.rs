use spin::Lazy;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

use crate::logln;

static INTERRUPT_TABLE: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut idt = InterruptDescriptorTable::new();
    idt.double_fault.set_handler_fn(double_fault_handler);
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt
});

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    panic!(
        "EXCEPTION: DOUBLE FAULT\n{:#?}\n{}",
        stack_frame, error_code
    );
}

extern "x86-interrupt" fn breakpoint_handler(
    //
    stack_frame: InterruptStackFrame,
) {
    logln!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

/// Installs the default interrupt table for the current processor.
///
/// # Safety
/// This function is unsafe as it is up to the caller to make sure
/// the running environment supports spurrious interrupts.
pub unsafe fn install_interrupt_table() {
    INTERRUPT_TABLE.load();
}
