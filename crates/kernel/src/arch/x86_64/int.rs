use spin::{LazyLock, Mutex};
use x2apic::lapic::{xapic_base, LocalApic, LocalApicBuilder};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

use crate::logln;

#[repr(u8)]
enum InterruptIndex {
    TIMER = 32,
    ERROR = 33,
    SPURIOUS = 34,
}

/// `LocalApic` is deliberately `!Send`: it drives the APIC of the processor
/// it was built on. The kernel only runs on the bootstrap processor, so the
/// handle never leaves the processor that created it.
struct BspLocalApic(LocalApic);

// SAFETY: see above. This needs per-processor storage instead once other
// processors are started.
unsafe impl Send for BspLocalApic {}

/// The local APIC is a per-processor register that controls the interrupt
/// handling for the processor.
static LAPIC: LazyLock<Mutex<BspLocalApic>> = LazyLock::new(|| unsafe {
    Mutex::new(BspLocalApic(
        LocalApicBuilder::new()
            .timer_vector(InterruptIndex::TIMER as usize)
            .error_vector(InterruptIndex::ERROR as usize)
            .spurious_vector(InterruptIndex::SPURIOUS as usize)
            .set_xapic_base(xapic_base())
            .build()
            .unwrap(),
    ))
});

///
pub unsafe fn install_local_apic() {
    // LAPIC.lock().enable();
}

/// The interrupt table defines a set of functions that get called when
/// an interrupt is triggered.
static INTERRUPT_TABLE: LazyLock<InterruptDescriptorTable> = LazyLock::new(|| {
    let mut idt = InterruptDescriptorTable::new();
    idt.double_fault.set_handler_fn(double_fault_handler);
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt[InterruptIndex::TIMER as u8].set_handler_fn(timer_handler);
    idt[InterruptIndex::ERROR as u8].set_handler_fn(error_handler);
    idt[InterruptIndex::SPURIOUS as u8].set_handler_fn(spurious_handler);
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

extern "x86-interrupt" fn breakpoint_handler(
    //
    stack_frame: InterruptStackFrame,
) {
    logln!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn timer_handler(
    //
    stack_frame: InterruptStackFrame,
) {
    logln!("TIMER:\n{:#?}", stack_frame);
    unsafe { LAPIC.lock().0.end_of_interrupt() };
}

extern "x86-interrupt" fn error_handler(
    //
    stack_frame: InterruptStackFrame,
) {
    logln!("ERROR:\n{:#?}", stack_frame);
    unsafe { LAPIC.lock().0.end_of_interrupt() };
}

extern "x86-interrupt" fn spurious_handler(
    //
    stack_frame: InterruptStackFrame,
) {
    logln!("SPURIOUS:\n{:#?}", stack_frame);
    unsafe { LAPIC.lock().0.end_of_interrupt() };
}
