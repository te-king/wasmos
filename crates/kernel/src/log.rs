use core::{cell::OnceCell, fmt::Write};

use crate::arch::{self, Console};

static STDIO_PORT: spin::Mutex<OnceCell<Console>> = spin::Mutex::new(OnceCell::new());

/// Installs a serial port as the global stdio writer.
pub fn install_stdio_port(port: Console) -> Result<(), Console> {
    STDIO_PORT.lock().set(port)
}

#[doc(hidden)]
pub fn _log(args: core::fmt::Arguments) {
    arch::without_interrupts(|| {
        if let Some(writer) = STDIO_PORT.lock().get_mut() {
            // Logging is best-effort: a failing `Display` impl shouldn't panic.
            let _ = writer.write_fmt(args);
        }
    })
}

/// Logs from the panic handler, even if the log lock is already held.
///
/// The holder may be this processor, interrupted mid-log by the panic, so
/// waiting for the lock could hang forever. The kernel is going down, so the
/// message matters more than the lock: if it's held, it's forced open.
pub fn log_panic(args: core::fmt::Arguments) {
    arch::disable_interrupts();
    if STDIO_PORT.is_locked() {
        // SAFETY: Nothing runs after the panic handler, so whoever holds the
        // lock never touches the port again. Another processor holding it
        // mid-write could interleave output, which is acceptable here.
        unsafe { STDIO_PORT.force_unlock() };
    }
    if let Some(writer) = STDIO_PORT.lock().get_mut() {
        let _ = writer.write_fmt(args);
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
