use x86_64::instructions::port::Port;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    // SAFETY: The runner puts QEMU's isa-debug-exit device at port 0xf4,
    // where this write ends the emulator. No standard device lives there
    // on other machines.
    unsafe { Port::new(0xf4).write(exit_code as u32) }
}
