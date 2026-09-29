use core::fmt::{Arguments, Write};

use spin::{Mutex, Once};

use crate::arch::{self, Console};

/// The kernel log's port: installed once, then locked for each write.
static STDIO_PORT: Once<Mutex<Console>> = Once::new();

/// Installs `port` as the kernel log. Only the first port counts, and the
/// boot sequence only installs one (`exit` consumes its token).
pub fn install(port: Console) {
    STDIO_PORT.call_once(|| Mutex::new(port));
}

#[doc(hidden)]
pub fn _log(args: Arguments) {
    if let Some(port) = STDIO_PORT.get() {
        arch::without_interrupts(|| write(port, args));
    }
}

/// Logs from the panic handler, even if the log lock is already held.
///
/// The holder may be this processor, interrupted mid-log by the panic, so
/// waiting for the lock could hang forever. The kernel is going down, so the
/// message matters more than the lock: if it's held, it's forced open.
pub fn log_panic(args: Arguments) {
    arch::disable_interrupts();
    if let Some(port) = STDIO_PORT.get() {
        if port.is_locked() {
            // SAFETY: Nothing runs after the panic handler, so whoever holds
            // the lock never touches the port again. Another processor
            // holding it mid-write could interleave output, which is
            // acceptable here.
            unsafe { port.force_unlock() };
        }
        write(port, args);
    }
}

/// Writes to the log's port. Logging is best-effort: a failing `Display`
/// impl shouldn't panic.
fn write(port: &Mutex<Console>, args: Arguments) {
    let _ = port.lock().write_fmt(args);
}

/// Logs a message to the kernel log.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::_log(::core::format_args!($($arg)*))
    };
}

/// Logs a message to the kernel log, followed by a newline.
///
/// The message is formatted by its own `format_args!` rather than joined to
/// the newline with `concat!`, which would stop the format string capturing
/// variables (`logln!("{x}")`).
#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {
        $crate::log!("{}\n", ::core::format_args!($($arg)*))
    };
}
