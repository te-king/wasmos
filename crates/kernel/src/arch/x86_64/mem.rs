use talc::{TalcLock, source::Claim};
use uefi::mem::memory_map::{MemoryMap, MemoryMapOwned, MemoryType};

use crate::sync::RawIrqMutex;

/// UEFI memory map pages are always 4 KiB, whatever the architecture.
pub const PAGE_SIZE: usize = 4096;

/// Size of the heap available from the start, before boot services are
/// exited and the rest of memory is added by [`install_memory_map`].
const EARLY_HEAP_SIZE: usize = 1024 * 1024;

/// Backs the heap until the memory map is installed. Being a static, it's
/// part of the kernel image, which the firmware loads as loader data, so it
/// survives exiting boot services and never overlaps the conventional memory
/// claimed later.
static mut EARLY_HEAP: [u8; EARLY_HEAP_SIZE] = [0; EARLY_HEAP_SIZE];

/// The allocator claims `EARLY_HEAP` the first time it runs out of memory,
/// so it can be used from the very start of the kernel. Its lock holds off
/// interrupts, so a handler can allocate and free too.
#[global_allocator]
static ALLOCATOR: TalcLock<RawIrqMutex, Claim> =
    // SAFETY: Nothing else refers to `EARLY_HEAP`, so the allocator has it
    // to itself for the life of the kernel.
    TalcLock::new(unsafe { Claim::array(&raw mut EARLY_HEAP) });

/// Gives the allocator every free region in `memory_map`.
///
/// # Safety
/// `memory_map` must be the one returned by exiting boot services, so that
/// its conventional regions are free, and nothing may have used them since.
pub unsafe fn install_memory_map(memory_map: MemoryMapOwned) {
    free_regions(&memory_map).for_each(|(base, size)| {
        // SAFETY: The caller guarantees the conventional regions are free,
        // so nothing else uses this memory while the allocator owns it.
        unsafe { ALLOCATOR.lock().claim(base, size) }.unwrap();
    });
}

/// The start and size of each region in `memory_map` that the allocator can
/// have: conventional memory, except a region at address 0, where a
/// pointer would be null.
fn free_regions(memory_map: &MemoryMapOwned) -> impl Iterator<Item = (*mut u8, usize)> + '_ {
    memory_map
        .entries()
        .filter(|region| region.ty == MemoryType::CONVENTIONAL && region.phys_start != 0)
        .map(|region| {
            (
                region.phys_start as *mut u8,
                region.page_count as usize * PAGE_SIZE,
            )
        })
}
