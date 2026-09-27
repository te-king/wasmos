use talc::{TalcLock, source::Claim};
use uefi::mem::memory_map::{MemoryMap, MemoryMapOwned, MemoryType};

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
/// so it can be used from the very start of the kernel.
#[global_allocator]
static ALLOCATOR: TalcLock<spin::Mutex<()>, Claim> =
    TalcLock::new(unsafe { Claim::array(&raw mut EARLY_HEAP) });

/// Register the memory map with the memory allocator
///
/// # Safety
/// This function assumes all conventional sections in the memory map
/// are available for the allocator, and that the memory map is valid.
/// This function should be called immediately after creating the memory map to reduce
/// the chance the memory layout has changed.
pub unsafe fn install_memory_map(memory_map: MemoryMapOwned) {
    let conventional = memory_map
        .entries()
        .filter(|m| m.ty == MemoryType::CONVENTIONAL)
        .filter(|m| m.phys_start != 0);

    for region in conventional {
        let base = region.phys_start as *mut u8;
        let size = region.page_count as usize * PAGE_SIZE;
        // SAFETY: The caller guarantees the conventional regions are free,
        // so nothing else uses this memory while the allocator owns it.
        unsafe { ALLOCATOR.lock().claim(base, size) }.unwrap();
    }
}
