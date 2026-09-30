//! The standard library for wasmos guests.

use wasmos_abi as abi;

/// Raw host imports. Prefer the safe wrappers in the crate root.
pub mod sys {
    #[link(wasm_import_module = "host")]
    unsafe extern "C" {
        /// Writes `len` bytes of UTF-8 text starting at `ptr` to the kernel
        /// log. The kernel traps the guest if the range is out of bounds or
        /// isn't valid UTF-8.
        pub fn wasmos_print(ptr: *const u8, len: usize);
    }
}

// Import and export attributes need string literals, so check them against
// the ABI.
const _: () = assert!(abi::same(abi::MODULE, "host"));
const _: () = assert!(abi::same(abi::PRINT, "wasmos_print"));
const _: () = assert!(abi::same(abi::ENTRY, "main"));

/// Makes `main`, a `fn()`, the guest's entry point: the function the kernel
/// calls to run it, exported as `wasmos_abi::ENTRY` with the C ABI.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(export_name = "main")]
        pub extern "C" fn __wasmos_entry() {
            $main()
        }
    };
}

/// Writes `text` to the kernel log.
pub fn print(text: &str) {
    // SAFETY: `text` is valid UTF-8 and its `len` bytes at `ptr` stay valid
    // for the call, which is all the host reads.
    unsafe { sys::wasmos_print(text.as_ptr(), text.len()) }
}

/// Formats its arguments and writes them to the kernel log.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::print(&::std::format!($($arg)*))
    };
}

/// Like [`print!`], followed by a newline.
#[macro_export]
macro_rules! println {
    () => {
        $crate::print("\n")
    };
    ($($arg:tt)*) => {
        $crate::print(&::std::format!("{}\n", ::std::format_args!($($arg)*)))
    };
}
