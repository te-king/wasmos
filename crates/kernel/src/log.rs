use core::fmt::{self, Write};

use crate::arch::{self, Console};

static STDIO_PORT: spin::Mutex<Option<Console>> = spin::Mutex::new(None);

/// Installs a serial port as the global stdio writer.
pub fn install_stdio_port(port: Console) {
    *STDIO_PORT.lock() = Some(port);
}

#[doc(hidden)]
pub fn _log(args: fmt::Arguments) {
    arch::without_interrupts(|| write(&mut STDIO_PORT.lock(), args))
}

/// Logs from the panic handler, even if the log lock is already held.
///
/// The holder may be this processor, interrupted mid-log by the panic, so
/// waiting for the lock could hang forever. The kernel is going down, so the
/// message matters more than the lock: if it's held, it's forced open.
pub fn log_panic(args: fmt::Arguments) {
    arch::disable_interrupts();
    if STDIO_PORT.is_locked() {
        // SAFETY: Nothing runs after the panic handler, so whoever holds the
        // lock never touches the port again. Another processor holding it
        // mid-write could interleave output, which is acceptable here.
        unsafe { STDIO_PORT.force_unlock() };
    }
    write(&mut STDIO_PORT.lock(), args)
}

/// Writes to the port, if one is installed. Logging is best-effort: a
/// failing `Display` impl shouldn't panic.
fn write(port: &mut Option<Console>, args: fmt::Arguments) {
    if let Some(port) = port {
        let _ = port.write_fmt(args);
    }
}

/// Logs a message to the kernel log.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::_log(format_args!($($arg)*))
    };
}

/// Logs a message to the kernel log, followed by a newline.
#[macro_export]
macro_rules! logln {
    () => ($crate::log!("\n"));
    ($fmt:expr) => ($crate::log!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::log!(concat!($fmt, "\n"), $($arg)*));
}
