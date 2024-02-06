use core::{cell::OnceCell, fmt::Write};

use uart_16550::SerialPort;

static STDIO_PORT: spin::Mutex<OnceCell<SerialPort>> = spin::Mutex::new(OnceCell::new());

/// Installs a serial port as the global stdio writer.
pub fn install_stdio_port(port: SerialPort) -> Result<(), SerialPort> {
    STDIO_PORT.lock().set(port)
}

#[doc(hidden)]
pub fn _log(args: core::fmt::Arguments) {
    if let Some(writer) = STDIO_PORT.lock().get_mut() {
        writer.write_fmt(args).unwrap();
    }
}

/// Logs a message to the kernel log.
#[macro_export]
macro_rules! log {
	($($arg:tt)*) => {
		$crate::log::_log(format_args!($($arg)*));
	};
}

/// Logs a message to the kernel log, followed by a newline.
#[macro_export]
macro_rules! logln {
	() => ($crate::log!("\n"));
	($fmt:expr) => ($crate::log!(concat!($fmt, "\n")));
	($fmt:expr, $($arg:tt)*) => ($crate::log!(concat!($fmt, "\n"), $($arg)*));
}
