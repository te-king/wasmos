use core::fmt::{Arguments, Write};

use spin::Once;

use crate::{
    arch::{self, Console},
    sync::IrqMutex,
};

/// The kernel log's port: installed once, then locked for each write.
static STDIO_PORT: Once<IrqMutex<Console>> = Once::new();

/// Installs `port` as the kernel log. Only the first port counts, and the
/// boot sequence only installs one (`exit` consumes its token).
pub fn install(port: Console) {
    STDIO_PORT.call_once(|| IrqMutex::new(port));
}

#[doc(hidden)]
pub fn _log(args: Arguments) {
    if let Some(port) = STDIO_PORT.get() {
        write(&mut port.lock(), args);
    }
}

/// Logs from the panic handler, even if the log lock is already held.
///
/// The holder may be this processor, interrupted mid-log by the panic, so
/// waiting for the lock could hang forever. The kernel is going down, so the
/// message matters more than the lock: if it's held, the port is written
/// without it.
pub fn log_panic(args: Arguments) {
    arch::disable_interrupts();
    if let Some(port) = STDIO_PORT.get() {
        if port.is_locked() {
            // SAFETY: Nothing runs after the panic handler, so if this
            // processor holds the lock, the holder never touches the port
            // again. Another processor holding it mid-write could interleave
            // output, which is acceptable here.
            write(unsafe { &mut *port.data_ptr() }, args);
        } else {
            write(&mut port.lock(), args);
        }
    }
}

/// Writes to the log's port. Logging is best-effort: a failing `Display`
/// impl shouldn't panic.
fn write(port: &mut Console, args: Arguments) {
    let _ = port.write_fmt(args);
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
