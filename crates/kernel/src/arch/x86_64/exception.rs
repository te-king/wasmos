//! Processor exceptions.
//!
//! A breakpoint is logged and resumed. Every other exception is a bug the
//! kernel can't recover from: its handler turns what the processor reports
//! into a [`Fault`] and panics with it.

use core::fmt;

use x86_64::{
    registers::control::Cr2,
    structures::idt::{
        Entry, EntryOptions, HandlerFuncType, InterruptDescriptorTable, InterruptStackFrame,
        PageFaultErrorCode,
    },
};

use super::{
    cpu::Local,
    gdt::{self, InterruptStack},
};
use crate::logln;

/// An exception the kernel can't recover from, as plain data.
struct Fault {
    name: &'static str,
    frame: InterruptStackFrame,
    cause: Cause,
}

/// What the processor reported beyond the interrupted state.
enum Cause {
    Unknown,
    /// An error code, whose meaning depends on the exception. For segment
    /// and table faults it's the selector involved.
    ErrorCode(u64),
    /// The address the processor failed to access (CR2), and why.
    PageFault(u64, PageFaultErrorCode),
    /// A double fault's error code is always zero, but CR2 still holds the
    /// last page fault's address, which is the culprit when the double
    /// fault came from overflowing a stack.
    DoubleFault {
        last_page_fault: u64,
    },
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rip = self.frame.instruction_pointer.as_u64();
        write!(f, "{} at {rip:#x}", self.name)?;
        match self.cause {
            Cause::Unknown => Ok(()),
            Cause::ErrorCode(code) => write!(f, ", error code {code:#x}"),
            Cause::PageFault(address, code) => write!(f, ", accessing {address:#x} ({code:?})"),
            Cause::DoubleFault { last_page_fault } => {
                write!(f, ", last page fault at {last_page_fault:#x}")
            }
        }?;
        write!(f, "\n{:#?}", self.frame)
    }
}

/// Panics with `fault`, naming the processor it happened on.
fn fatal(fault: Fault) -> ! {
    // SAFETY: The interrupt table is only loaded after `cpu::init`
    // (`enable_interrupts` takes the `PerCpu` token).
    let id = unsafe { Local::assume() }.id();
    panic!("cpu {id}: {fault}")
}

/// Defines handlers that panic with a [`Fault`], for exceptions whose
/// handlers take an error code (`with code`) or don't.
macro_rules! fatal_handlers {
    ($($handler:ident: $name:literal),* $(,)?) => {$(
        extern "x86-interrupt" fn $handler(frame: InterruptStackFrame) {
            fatal(Fault { name: $name, frame, cause: Cause::Unknown })
        }
    )*};
    (with code $($handler:ident: $name:literal),* $(,)?) => {$(
        extern "x86-interrupt" fn $handler(frame: InterruptStackFrame, code: u64) {
            fatal(Fault { name: $name, frame, cause: Cause::ErrorCode(code) })
        }
    )*};
}

fatal_handlers! {
    divide_error: "divide error",
    debug: "debug exception",
    non_maskable_interrupt: "non-maskable interrupt",
    overflow: "overflow",
    bound_range_exceeded: "bound range exceeded",
    invalid_opcode: "invalid opcode",
    device_not_available: "device not available",
    x87_floating_point: "x87 floating-point exception",
    simd_floating_point: "SIMD floating-point exception",
    virtualization: "virtualization exception",
    hv_injection_exception: "hypervisor injection exception",
}

fatal_handlers! { with code
    invalid_tss: "invalid TSS",
    segment_not_present: "segment not present",
    stack_segment_fault: "stack-segment fault",
    general_protection_fault: "general protection fault",
    alignment_check: "alignment check",
    cp_protection_exception: "control protection exception",
    vmm_communication_exception: "VMM communication exception",
    security_exception: "security exception",
}

extern "x86-interrupt" fn page_fault(frame: InterruptStackFrame, code: PageFaultErrorCode) {
    let cause = Cause::PageFault(Cr2::read_raw(), code);
    fatal(Fault {
        name: "page fault",
        frame,
        cause,
    })
}

extern "x86-interrupt" fn double_fault(frame: InterruptStackFrame, _: u64) -> ! {
    let cause = Cause::DoubleFault {
        last_page_fault: Cr2::read_raw(),
    };
    fatal(Fault {
        name: "double fault",
        frame,
        cause,
    })
}

extern "x86-interrupt" fn machine_check(frame: InterruptStackFrame) -> ! {
    fatal(Fault {
        name: "machine check",
        frame,
        cause: Cause::Unknown,
    })
}

extern "x86-interrupt" fn breakpoint(frame: InterruptStackFrame) {
    logln!("EXCEPTION: BREAKPOINT\n{frame:#?}");
}

/// Sets `entry`'s handler, to run in the kernel's code segment.
pub fn gate<F: HandlerFuncType>(entry: &mut Entry<F>, handler: F) -> &mut EntryOptions {
    // SAFETY: `KERNEL_CODE` is the kernel's 64-bit code segment in every
    // processor's descriptor table.
    unsafe {
        entry
            .set_handler_fn(handler)
            .set_code_selector(gdt::KERNEL_CODE)
    }
}

/// Like [`gate`], with the handler on its own interrupt stack.
fn gate_on<F: HandlerFuncType>(entry: &mut Entry<F>, handler: F, stack: InterruptStack) {
    // SAFETY: Each `InterruptStack` serves one exception, which doesn't
    // nest, so the handler never finds its stack in use.
    unsafe { gate(entry, handler).set_stack_index(stack.index()) };
}

/// An interrupt table with a handler for every exception.
pub fn table() -> InterruptDescriptorTable {
    let mut idt = InterruptDescriptorTable::new();
    gate(&mut idt.divide_error, divide_error);
    gate(&mut idt.debug, debug);
    gate_on(
        &mut idt.non_maskable_interrupt,
        non_maskable_interrupt,
        InterruptStack::NonMaskable,
    );
    gate(&mut idt.breakpoint, breakpoint);
    gate(&mut idt.overflow, overflow);
    gate(&mut idt.bound_range_exceeded, bound_range_exceeded);
    gate(&mut idt.invalid_opcode, invalid_opcode);
    gate(&mut idt.device_not_available, device_not_available);
    gate_on(
        &mut idt.double_fault,
        double_fault,
        InterruptStack::DoubleFault,
    );
    gate(&mut idt.invalid_tss, invalid_tss);
    gate(&mut idt.segment_not_present, segment_not_present);
    gate(&mut idt.stack_segment_fault, stack_segment_fault);
    gate(&mut idt.general_protection_fault, general_protection_fault);
    gate(&mut idt.page_fault, page_fault);
    gate(&mut idt.x87_floating_point, x87_floating_point);
    gate(&mut idt.alignment_check, alignment_check);
    gate_on(
        &mut idt.machine_check,
        machine_check,
        InterruptStack::MachineCheck,
    );
    gate(&mut idt.simd_floating_point, simd_floating_point);
    gate(&mut idt.virtualization, virtualization);
    gate(&mut idt.cp_protection_exception, cp_protection_exception);
    gate(&mut idt.hv_injection_exception, hv_injection_exception);
    gate(
        &mut idt.vmm_communication_exception,
        vmm_communication_exception,
    );
    gate(&mut idt.security_exception, security_exception);
    idt
}
