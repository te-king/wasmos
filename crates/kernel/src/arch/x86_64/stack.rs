//! Kernel stacks.
//!
//! A stack is only reachable through its [`Top`], which isn't `Copy`, so
//! each stack has one user: a processor running on it or an interrupt stack
//! table slot.

use alloc::boxed::Box;
use core::{arch::asm, mem::ManuallyDrop};

use x86_64::VirtAddr;

/// The stack every processor runs the kernel on. Running a guest takes
/// about 340 KiB of it, in debug and release builds alike.
pub const KERNEL_SIZE: usize = 1024 * 1024;

/// `SIZE` bytes of stack, aligned as the System V ABI expects.
#[repr(C, align(16))]
struct Stack<const SIZE: usize>([u8; SIZE]);

/// The top of a stack that nothing has used yet, where its stack pointer
/// starts: stacks grow down. Owning it means owning the stack.
pub struct Top(VirtAddr);

impl Top {
    pub fn into_addr(self) -> VirtAddr {
        self.0
    }
}

/// Allocates a stack of `SIZE` bytes that is never freed.
///
/// Nothing guards its bottom yet, so overflowing it corrupts whatever the
/// heap put below. A guard page needs the kernel to own the page tables,
/// since the firmware maps its own read-only.
pub fn leak<const SIZE: usize>() -> Top {
    let stack = Box::leak(Box::<Stack<SIZE>>::new_uninit());
    Top(VirtAddr::from_ptr(stack.as_mut_ptr().wrapping_add(1)))
}

/// Runs `f` on `stack`, leaving the current one for good.
///
/// Whatever the current stack holds stays where it is, so references into
/// it remain valid for as long as `f` runs.
pub fn run_on<F: FnOnce() -> !>(stack: Top, f: F) -> ! {
    /// Moves the closure onto the new stack and calls it.
    extern "sysv64" fn enter<F: FnOnce() -> !>(f: *mut F) -> ! {
        // SAFETY: `run_on` passes its closure, which it never touches again,
        // and whose memory stays valid (see above).
        let f = unsafe { f.read() };
        f()
    }

    let mut f = ManuallyDrop::new(f);
    // SAFETY: `Top` guarantees an unused, 16-byte aligned stack that outlives
    // the kernel, so `enter` starts with the alignment the ABI expects once
    // `call` has pushed its return address. `enter` never returns, and `f`
    // is moved out exactly once.
    unsafe {
        asm!(
            "mov rsp, {top}",
            "call {enter}",
            "ud2",
            top = in(reg) stack.into_addr().as_u64(),
            enter = sym enter::<F>,
            in("rdi") (&raw mut f).cast::<F>(),
            options(noreturn),
        )
    }
}
