use talc::{ErrOnOom, Span, Talc, Talck};
use uefi::table::boot::{MemoryMap, MemoryType};

#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, ErrOnOom> = Talc::new(ErrOnOom).lock();

/// Register the memory map with the memory allocator
///
/// # Safety
/// This function assumes all conventional secions in the memory map
/// are available for the allocator, and that the memory map is valid.
/// This function should be called immediately after creating the memory map to reduce
/// the chance the memory layout has changed.
pub unsafe fn install_memory_map(memory_map: MemoryMap) {
    let conventional = memory_map
        .entries()
        .filter(|m| m.ty == MemoryType::CONVENTIONAL);

    for region in conventional {
        let start = region.phys_start as usize;
        let span = Span::from_base_size(start as *mut _, region.page_count as usize * 4096);
        ALLOCATOR.lock().claim(span).unwrap();
    }
}
