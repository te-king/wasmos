//! The code an application processor runs when it starts, and the page it
//! runs from.
//!
//! A startup IPI starts a processor in 16-bit real mode, at the start of a
//! page below 1 MiB. The trampoline takes it through protected mode into
//! long mode, with the bootstrap processor's page tables, then calls
//! [`enter`] on a fresh stack. Everything it needs is in a [`Handoff`] in
//! the same page, which the bootstrap processor writes in two parts: what
//! every processor shares, once, then what is the next one's own before
//! each start:
//!
//! ```text
//! Trampoline --arm(main)--> Armed --launch(id)--> Launch --land()--> Armed
//! ```

use core::{
    arch::global_asm,
    mem::offset_of,
    ptr::{self, NonNull},
    sync::atomic::{AtomicBool, Ordering},
};

use uefi::{
    boot::{self, AllocateType},
    mem::memory_map::MemoryType,
};
use x86_64::{
    VirtAddr,
    registers::{
        control::{Cr0, Cr3, Cr4, Cr4Flags},
        model_specific::{Efer, EferFlags},
    },
    structures::{DescriptorTablePointer, gdt::DescriptorFlags},
};

use super::{
    boot::{Ap, BootServices, Heap},
    cpu::CpuId,
    gdt,
    mem::PAGE_SIZE,
    stack,
};

/// Where the [`Handoff`] sits in the trampoline page, after the code.
const HANDOFF: usize = 0x800;
const _: () = assert!(HANDOFF + size_of::<Handoff>() <= PAGE_SIZE);

/// Positions in [`GDT`], the trampoline's own descriptor table.
const CODE32: usize = 1;
const DATA: usize = 2;
const CODE64: usize = 3;

/// Null, then flat 32-bit code, data and 64-bit code segments, each at its
/// position, so a [`selector`] of the position names it.
const GDT: [u64; 4] = {
    let mut gdt = [0; 4];
    gdt[CODE32] = DescriptorFlags::KERNEL_CODE32.bits();
    gdt[DATA] = DescriptorFlags::KERNEL_DATA.bits();
    gdt[CODE64] = DescriptorFlags::KERNEL_CODE64.bits();
    gdt
};

/// The selector for position `index` of [`GDT`].
const fn selector(index: usize) -> u16 {
    (index * size_of::<u64>()) as u16
}

/// A far pointer (`m16:32`), the operand of an indirect far jump.
#[derive(Clone, Copy)]
#[repr(C, packed)]
struct FarPointer {
    offset: u32,
    selector: u16,
}

/// Everything a starting processor needs.
#[repr(C)]
struct Handoff {
    /// The same for every processor. Written once, by `arm`.
    shared: Shared,
    /// The processor being started. Written before each start, by `launch`.
    next: Next,
}

#[repr(C)]
struct Shared {
    // Read by the trampoline code.
    /// A copy of [`GDT`], which real mode can only reach within the page.
    gdt: [u64; 4],
    /// Loaded in real mode, which only reads the limit and 24 bits of base.
    gdt_pointer: DescriptorTablePointer,
    protected_mode: FarPointer,
    long_mode: FarPointer,
    registers: ControlRegisters,
    enter: unsafe extern "sysv64" fn(*const Handoff) -> !,

    // Read by `enter`.
    main: fn(Heap<Ap>) -> !,
}

#[repr(C)]
struct Next {
    /// Read by the trampoline code.
    stack_top: u64,

    // Read by `enter`.
    id: CpuId,
    /// Set once the processor no longer needs the handoff.
    arrived: AtomicBool,
}

// The trampoline code. It is copied into the trampoline page and runs from
// there, never from where it is linked, so it only addresses memory
// relative to the page: through DS in real mode, and through EBX (the
// page's address) after that.
global_asm!(
    ".globl wasmos_trampoline_start",
    ".globl wasmos_trampoline_protected",
    ".globl wasmos_trampoline_long",
    ".globl wasmos_trampoline_end",
    "wasmos_trampoline_start:",
    ".code16",
    "cli",
    "cld",
    // A startup IPI sets CS to the page's segment, so the page starts at
    // CS * 16.
    "mov ax, cs",
    "mov ds, ax",
    "movzx ebx, ax",
    "shl ebx, 4",
    "lgdt [{gdt_pointer}]",
    "mov eax, cr0",
    "or eax, 1",
    "mov cr0, eax",
    "jmp fword ptr [{protected_mode}]",
    "wasmos_trampoline_protected:",
    ".code32",
    "mov ax, {data}",
    "mov ds, ax",
    "mov es, ax",
    "mov ss, ax",
    // Long mode needs PAE and the page tables before it can be enabled, and
    // enabling paging with LME set is what activates it.
    "mov eax, [ebx + {cr4}]",
    "mov cr4, eax",
    "mov eax, [ebx + {cr3}]",
    "mov cr3, eax",
    "mov ecx, 0xC0000080", // IA32_EFER
    "mov eax, [ebx + {efer}]",
    "mov edx, [ebx + {efer} + 4]",
    "wrmsr",
    "mov eax, [ebx + {cr0}]",
    "mov cr0, eax",
    "jmp fword ptr [ebx + {long_mode}]",
    "wasmos_trampoline_long:",
    ".code64",
    // The upper halves of registers are undefined after the mode switch.
    "mov ebx, ebx",
    "mov rsp, [rbx + {stack_top}]",
    "lea rdi, [rbx + {handoff}]",
    "call [rbx + {enter}]",
    "ud2",
    "wasmos_trampoline_end:",
    gdt_pointer = const HANDOFF + offset_of!(Handoff, shared.gdt_pointer),
    protected_mode = const HANDOFF + offset_of!(Handoff, shared.protected_mode),
    data = const selector(DATA),
    cr4 = const HANDOFF + offset_of!(Handoff, shared.registers.cr4),
    cr3 = const HANDOFF + offset_of!(Handoff, shared.registers.cr3),
    efer = const HANDOFF + offset_of!(Handoff, shared.registers.efer),
    cr0 = const HANDOFF + offset_of!(Handoff, shared.registers.cr0),
    long_mode = const HANDOFF + offset_of!(Handoff, shared.long_mode),
    stack_top = const HANDOFF + offset_of!(Handoff, next.stack_top),
    handoff = const HANDOFF,
    enter = const HANDOFF + offset_of!(Handoff, shared.enter),
);

unsafe extern "C" {
    static wasmos_trampoline_start: u8;
    static wasmos_trampoline_protected: u8;
    static wasmos_trampoline_long: u8;
    static wasmos_trampoline_end: u8;
}

/// Offset of a trampoline label from the start of the code.
fn offset_of_label(label: *const u8) -> usize {
    label as usize - (&raw const wasmos_trampoline_start) as usize
}

/// The page application processors start in, holding the trampoline code.
pub struct Trampoline {
    page: NonNull<u8>,
}

impl Trampoline {
    /// Reserves a page below 1 MiB and copies the trampoline code into it.
    ///
    /// The page is loader code, which the allocator never claims, so it
    /// stays reserved after boot services are exited. It has to be code
    /// rather than data: the trampoline keeps running from it once paging
    /// is on, and firmware may map loader data non-executable.
    pub fn reserve(_: &BootServices) -> uefi::Result<Self> {
        // A startup IPI's vector is the page number, so the page must end
        // below 1 MiB.
        let page = boot::allocate_pages(
            AllocateType::MaxAddress(0xF_FFFF),
            MemoryType::LOADER_CODE,
            1,
        )?;
        let start = &raw const wasmos_trampoline_start;
        let len = offset_of_label(&raw const wasmos_trampoline_end);
        assert!(len <= HANDOFF, "trampoline code overlaps its handoff");
        // Zeroed, so the page holds nothing but the code until `arm` and
        // `launch` write its handoff.
        // SAFETY: The page was just allocated, and the code is `len` bytes
        // of the kernel image, which fit in front of the handoff.
        unsafe {
            ptr::write_bytes(page.as_ptr(), 0, PAGE_SIZE);
            ptr::copy_nonoverlapping(start, page.as_ptr(), len);
        }
        Ok(Trampoline { page })
    }

    /// Writes the part of the handoff every processor shares: they all run
    /// `main` with this processor's paging.
    pub fn arm(self, main: fn(Heap<Ap>) -> !) -> Armed {
        let shared = Shared::new(self.address(), ControlRegisters::read().for_startup(), main);
        // SAFETY: The handoff fits in the page (checked at compile time), and
        // no processor has been started from it yet.
        unsafe { (&raw mut (*self.handoff()).shared).write(shared) };
        Armed(self)
    }

    fn address(&self) -> u64 {
        self.page.as_ptr() as u64
    }

    fn handoff(&self) -> *mut Handoff {
        self.page.as_ptr().wrapping_add(HANDOFF).cast()
    }
}

/// A trampoline with the part of its handoff that every processor shares.
pub struct Armed(Trampoline);

impl Armed {
    /// Prepares the trampoline to start processor `id` on a new stack.
    ///
    /// The launch takes the trampoline, and only gives it back once the
    /// processor has let go of it ([`Launch::land`]), so nothing can rewrite
    /// the handoff while a processor might still read it.
    pub fn launch(self, id: CpuId) -> Launch {
        let next = Next::new(stack::leak::<{ stack::KERNEL_SIZE }>(), id);
        // SAFETY: The handoff fits in the page (checked at compile time), and
        // owning the trampoline means no processor is reading the old one.
        unsafe { (&raw mut (*self.0.handoff()).next).write(next) };
        Launch(self)
    }
}

/// A trampoline prepared to start one processor, which may be running it.
pub struct Launch(Armed);

impl Launch {
    /// The startup IPI vector that starts the processor in the trampoline.
    pub fn vector(&self) -> u8 {
        (self.trampoline().address() / PAGE_SIZE as u64) as u8
    }

    /// Whether the processor has entered the kernel and let go of the
    /// trampoline.
    pub fn arrived(&self) -> bool {
        // SAFETY: `launch` wrote the next processor's part of the handoff,
        // so `arrived` holds a valid `bool`, and since the processor could
        // be running, it is only accessed atomically.
        unsafe {
            (*self.trampoline().handoff())
                .next
                .arrived
                .load(Ordering::Acquire)
        }
    }

    /// The trampoline back, for the next launch, if the processor has let
    /// go of it. A processor that hasn't might still turn up and read its
    /// handoff, so its trampoline can never be used again.
    pub fn land(self) -> Option<Armed> {
        self.arrived().then_some(self.0)
    }

    fn trampoline(&self) -> &Trampoline {
        &(self.0).0
    }
}

/// The control registers a starting processor takes from the bootstrap
/// processor. Laid out for the trampoline code, which reads them from the
/// handoff.
#[derive(Clone, Copy)]
#[repr(C)]
struct ControlRegisters {
    cr0: u64,
    cr3: u64,
    cr4: u64,
    efer: u64,
}

impl ControlRegisters {
    /// This processor's.
    fn read() -> Self {
        ControlRegisters {
            cr0: Cr0::read_raw(),
            cr3: Cr3::read().0.start_address().as_u64(),
            cr4: Cr4::read_raw(),
            efer: Efer::read_raw(),
        }
    }

    /// These registers as a processor that has yet to enter long mode can
    /// load them, from 32-bit code.
    fn for_startup(self) -> Self {
        assert!(self.cr3 < 1 << 32, "page tables are out of 32-bit reach");
        ControlRegisters {
            // PCIDs can only be enabled once long mode is active, and the
            // kernel doesn't use them.
            cr4: self.cr4 & !Cr4Flags::PCID.bits(),
            // LMA is read-only: the processor sets it when paging comes on.
            efer: self.efer & !EferFlags::LONG_MODE_ACTIVE.bits(),
            ..self
        }
    }
}

impl Shared {
    /// The shared part of the handoff for the trampoline page at `page`,
    /// starting processors with `registers` and running `main`.
    fn new(page: u64, registers: ControlRegisters, main: fn(Heap<Ap>) -> !) -> Self {
        let far = |label: *const u8, index| FarPointer {
            offset: (page + offset_of_label(label) as u64) as u32,
            selector: selector(index),
        };
        Shared {
            gdt: GDT,
            gdt_pointer: DescriptorTablePointer {
                limit: (size_of_val(&GDT) - 1) as u16,
                base: VirtAddr::new(page + (HANDOFF + offset_of!(Handoff, shared.gdt)) as u64),
            },
            protected_mode: far(&raw const wasmos_trampoline_protected, CODE32),
            long_mode: far(&raw const wasmos_trampoline_long, CODE64),
            registers,
            enter,
            main,
        }
    }
}

impl Next {
    /// The part of the handoff for starting processor `id` on `stack`.
    fn new(stack: stack::Top, id: CpuId) -> Self {
        Next {
            stack_top: stack.into_addr().as_u64(),
            id,
            arrived: AtomicBool::new(false),
        }
    }
}

/// Where the trampoline enters the kernel, on the new processor's stack.
///
/// # Safety
/// Only the trampoline may call this, with the handoff it started with.
unsafe extern "sysv64" fn enter(handoff: *const Handoff) -> ! {
    // Everything is copied out before signalling arrival, since the
    // bootstrap processor can rewrite the handoff as soon as it sees it.
    // SAFETY: The trampoline passes the handoff that `arm` and `launch`
    // wrote.
    let (id, main) = unsafe { ((*handoff).next.id, (*handoff).shared.main) };

    // The descriptor table in use is the handoff's, so switch to one that
    // stays put until this processor gets its own in `init_cpu`.
    // SAFETY: The trampoline left this processor in 64-bit ring 0 with
    // interrupts disabled, and no interrupt table is loaded yet.
    unsafe { gdt::load_boot() };

    // SAFETY: The handoff is still valid, and `arrived` is atomic.
    unsafe { &(*handoff).next.arrived }.store(true, Ordering::Release);
    // SAFETY: This processor has just entered the kernel, only gets here
    // once, and `id` is the one `launch` was given for it.
    main(unsafe { Heap::application_processor(id) })
}
