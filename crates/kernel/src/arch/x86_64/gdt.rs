//! The kernel's segment descriptor tables.
//!
//! Each processor gets its own table, for its own task state segment, but
//! every table starts with the same [`SEGMENTS`]. So a selector means the
//! same thing on every processor, which the interrupt table relies on: its
//! entries all name [`KERNEL_CODE`].

use alloc::boxed::Box;
use core::array;

use x86_64::{
    PrivilegeLevel, VirtAddr,
    instructions::tables::load_tss,
    registers::segmentation::{CS, DS, ES, SS, Segment, SegmentSelector},
    structures::{
        gdt::{Descriptor, DescriptorFlags, GlobalDescriptorTable},
        tss::TaskStateSegment,
    },
};

use super::stack;

/// The segments every table starts with: null, then flat 64-bit kernel
/// code and data. Their accessed bits are preset, so the processor never
/// writes to a table (the boot table is a `static`).
const SEGMENTS: [u64; 3] = [
    0,
    DescriptorFlags::KERNEL_CODE64.bits(),
    DescriptorFlags::KERNEL_DATA.bits(),
];

pub const KERNEL_CODE: SegmentSelector = SegmentSelector::new(1, PrivilegeLevel::Ring0);
pub const KERNEL_DATA: SegmentSelector = SegmentSelector::new(2, PrivilegeLevel::Ring0);
/// The processor's task state segment, which follows [`SEGMENTS`] and takes
/// two entries.
const TASK_STATE: SegmentSelector =
    SegmentSelector::new(SEGMENTS.len() as u16, PrivilegeLevel::Ring0);

/// Just [`SEGMENTS`], for processors on their way into the kernel that
/// don't have a table of their own yet.
static BOOT: GlobalDescriptorTable<{ SEGMENTS.len() }> =
    GlobalDescriptorTable::from_raw_entries(&SEGMENTS);

/// The task state segment's interrupt stacks, for exceptions that must not
/// run on the stack they interrupted: a double fault can come from
/// overflowing it, and an NMI or machine check can arrive anywhere.
///
/// Each stack serves exactly one of these, and none of them nests (the
/// processor holds off another NMI until the first returns, and a second
/// double fault or machine check shuts it down), so a handler never finds
/// its stack in use.
#[derive(Clone, Copy)]
#[repr(u16)]
pub enum InterruptStack {
    DoubleFault,
    NonMaskable,
    MachineCheck,
}

impl InterruptStack {
    const ALL: [Self; 3] = [Self::DoubleFault, Self::NonMaskable, Self::MachineCheck];

    /// Its interrupt stack table slot, as `EntryOptions::set_stack_index`
    /// takes it.
    pub const fn index(self) -> u16 {
        self as u16
    }
}

const INTERRUPT_STACK_SIZE: usize = 32 * 1024;

/// A task state segment with each of `stacks` in its slot of the interrupt
/// stack table.
fn task_state(stacks: &[(InterruptStack, VirtAddr)]) -> TaskStateSegment {
    let mut tss = TaskStateSegment::new();
    // Assigned whole: the segment is packed, so its fields can't be borrowed.
    tss.interrupt_stack_table = array::from_fn(|slot| {
        stacks
            .iter()
            .find(|(stack, _)| usize::from(stack.index()) == slot)
            .map_or(VirtAddr::zero(), |&(_, top)| top)
    });
    tss
}

/// The table for a processor whose task state segment is `tss`.
fn table(tss: &'static TaskStateSegment) -> GlobalDescriptorTable<{ SEGMENTS.len() + 2 }> {
    let Descriptor::SystemSegment(low, high) = Descriptor::tss_segment(tss) else {
        unreachable!("a task state segment's descriptor is a system segment");
    };
    let [null, code, data] = SEGMENTS;
    GlobalDescriptorTable::from_raw_entries(&[null, code, data, low, high])
}

/// Switches this processor to the boot table.
///
/// # Safety
/// The processor must be in 64-bit mode, in ring 0, with interrupts
/// disabled. Anything that names the old table's selectors, such as an
/// interrupt table, stops working.
pub unsafe fn load_boot() {
    BOOT.load();
    // SAFETY: The caller guarantees 64-bit ring 0, where these are valid.
    unsafe { load_segments() };
}

/// Gives this processor its own table and task state segment, with fresh
/// interrupt stacks. They are never freed.
///
/// # Safety
/// As for [`load_boot`]. The heap must be up.
pub unsafe fn load_own() {
    let stacks =
        InterruptStack::ALL.map(|slot| (slot, stack::leak::<INTERRUPT_STACK_SIZE>().into_addr()));
    let tss = Box::leak(Box::new(task_state(&stacks)));
    Box::leak(Box::new(table(tss))).load();
    // SAFETY: The caller guarantees 64-bit ring 0, and the table just
    // loaded holds this task state segment, which nothing has loaded yet.
    unsafe {
        load_segments();
        load_tss(TASK_STATE);
    }
}

/// Reloads the segment registers from the current table. FS and GS are
/// left alone: reloading GS would clear the base that `cpu` depends on.
///
/// # Safety
/// The current table must start with [`SEGMENTS`], and the processor must
/// be in 64-bit mode, in ring 0.
unsafe fn load_segments() {
    // SAFETY: The caller guarantees these name flat ring 0 segments.
    unsafe {
        CS::set_reg(KERNEL_CODE);
        SS::set_reg(KERNEL_DATA);
        DS::set_reg(KERNEL_DATA);
        ES::set_reg(KERNEL_DATA);
    }
}
