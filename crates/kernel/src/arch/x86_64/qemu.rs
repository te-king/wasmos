use x86_64::instructions::{hlt, interrupts, port::Port};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

/// Reports `code` to QEMU, which exits, then halts this processor for good
/// in case it doesn't: outside QEMU, nothing listens on the port.
pub fn exit(code: QemuExitCode) -> ! {
    // SAFETY: 0xf4 is the isa-debug-exit device the runner gives QEMU. On
    // other machines the port is normally unused, so the write is lost.
    unsafe { Port::new(0xf4).write(code as u32) };
    interrupts::disable();
    loop {
        hlt();
    }
}
