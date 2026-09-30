//! The interfaces between the wasmos kernel, its wasm guests and its runner.
//!
//! The kernel defines these host functions and the guest library imports
//! them. Guests declare imports with attributes, which need string literals,
//! so `wlib` checks its literals against these constants with [`same`].
#![no_std]

/// The wasm import module the host functions live in.
pub const MODULE: &str = "host";

/// `wasmos_print(ptr: i32, len: i32)`: writes `len` bytes of UTF-8 text,
/// starting at `ptr` in the guest's `memory` export, to the kernel log.
pub const PRINT: &str = "wasmos_print";

/// `wasmos_sleep(ticks: i64)`: returns once at least `ticks` full timer
/// periods (about 10 ms each under QEMU, not calibrated) have passed. The
/// guest is suspended meanwhile, so the rest of the kernel runs.
pub const SLEEP: &str = "wasmos_sleep";

/// The function every guest exports and the kernel calls to run it:
/// `main()`, taking and returning nothing.
pub const ENTRY: &str = "main";

/// QEMU's isa-debug-exit device, which the runner adds and the kernel ends a
/// run through: writing a code to its port exits QEMU with status
/// `(code << 1) | 1`.
pub mod qemu {
    /// The I/O port the runner puts the device at.
    pub const PORT: u16 = 0xf4;
    /// The code the kernel writes when it succeeded.
    pub const SUCCESS: u32 = 0x10;
    /// The code the kernel writes when it failed.
    pub const FAILURE: u32 = 0x11;

    /// QEMU's exit status once the kernel has written `code`.
    pub const fn status(code: u32) -> i32 {
        ((code << 1) | 1) as i32
    }
}

/// Compares two strings at compile time, for checking import attributes.
pub const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
