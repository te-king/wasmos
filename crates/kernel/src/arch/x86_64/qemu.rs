//! Ending a run under QEMU, through the debug-exit device the runner adds
//! (see `wasmos_abi::qemu`).

use wasmos_abi::qemu as device;
use x86_64::instructions::port::Port;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = device::SUCCESS,
    Failed = device::FAILURE,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    // SAFETY: The runner puts QEMU's isa-debug-exit device at this port,
    // where this write ends the emulator. No standard device lives there
    // on other machines.
    unsafe { Port::new(device::PORT).write(exit_code as u32) }
}
