//! The interface between the wasmos kernel and its wasm guests.
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

/// The function every guest exports and the kernel calls to run it:
/// `main()`, taking and returning nothing.
pub const ENTRY: &str = "main";

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
