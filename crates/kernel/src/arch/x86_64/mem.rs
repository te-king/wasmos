use talc::{source::Manual, TalcLock};
use uefi::mem::memory_map::{MemoryMap, MemoryMapOwned, MemoryType};

#[global_allocator]
static ALLOCATOR: TalcLock<spin::Mutex<()>, Manual> = TalcLock::new(Manual);

/// Register the memory map with the memory allocator
///
/// # Safety
/// This function assumes all conventional secions in the memory map
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
        let size = region.page_count as usize * 4096;
        ALLOCATOR.lock().claim(base, size).unwrap();
    }
}
