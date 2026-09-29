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
    PageFault {
        address: u64,
        code: PageFaultErrorCode,
    },
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
            Cause::PageFault { address, code } => {
                write!(f, ", accessing {address:#x} ({code:?})")
            }
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

/// Defines the interrupt table from one list of every exception, in table
/// order. Each names its table entry and handler (they share a name), how
/// the handler comes about, and optionally the interrupt stack it runs on:
///
/// - `fatal entry: "name"`: generated, panicking with a [`Fault`].
/// - `fatal_code entry: "name"`: the same, for an exception with an error
///   code.
/// - `custom entry`: written out below.
macro_rules! exceptions {
    ($($kind:ident $entry:ident $(: $name:literal)? $(on $stack:ident)?,)*) => {
        $(exception_handler!($kind $entry $($name)?);)*

        /// An interrupt table with a handler for every exception.
        pub fn table() -> InterruptDescriptorTable {
            let mut idt = InterruptDescriptorTable::new();
            $(set_gate!(idt, $entry $(, $stack)?);)*
            idt
        }
    };
}

/// One exception's handler, for [`exceptions!`].
macro_rules! exception_handler {
    (fatal $entry:ident $name:literal) => {
        extern "x86-interrupt" fn $entry(frame: InterruptStackFrame) {
            fatal(Fault {
                name: $name,
                frame,
                cause: Cause::Unknown,
            })
        }
    };
    (fatal_code $entry:ident $name:literal) => {
        extern "x86-interrupt" fn $entry(frame: InterruptStackFrame, code: u64) {
            fatal(Fault {
                name: $name,
                frame,
                cause: Cause::ErrorCode(code),
            })
        }
    };
    (custom $entry:ident) => {};
}

/// One entry of the interrupt table, for [`exceptions!`].
macro_rules! set_gate {
    ($idt:ident, $entry:ident) => {
        gate(&mut $idt.$entry, $entry)
    };
    ($idt:ident, $entry:ident, $stack:ident) => {
        gate_on(&mut $idt.$entry, $entry, InterruptStack::$stack)
    };
}

exceptions! {
    fatal divide_error: "divide error",
    fatal debug: "debug exception",
    fatal non_maskable_interrupt: "non-maskable interrupt" on NonMaskable,
    custom breakpoint,
    fatal overflow: "overflow",
    fatal bound_range_exceeded: "bound range exceeded",
    fatal invalid_opcode: "invalid opcode",
    fatal device_not_available: "device not available",
    custom double_fault on DoubleFault,
    fatal_code invalid_tss: "invalid TSS",
    fatal_code segment_not_present: "segment not present",
    fatal_code stack_segment_fault: "stack-segment fault",
    fatal_code general_protection_fault: "general protection fault",
    custom page_fault,
    fatal x87_floating_point: "x87 floating-point exception",
    fatal_code alignment_check: "alignment check",
    custom machine_check on MachineCheck,
    fatal simd_floating_point: "SIMD floating-point exception",
    fatal virtualization: "virtualization exception",
    fatal_code cp_protection_exception: "control protection exception",
    fatal hv_injection_exception: "hypervisor injection exception",
    fatal_code vmm_communication_exception: "VMM communication exception",
    fatal_code security_exception: "security exception",
}

extern "x86-interrupt" fn page_fault(frame: InterruptStackFrame, code: PageFaultErrorCode) {
    let cause = Cause::PageFault {
        address: Cr2::read_raw(),
        code,
    };
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
