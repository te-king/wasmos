use anyhow::{bail, Result};
use tempdir::TempDir;

// QEMU's isa-debug-exit device exits with `(value << 1) | 1`, where `value` is
// what the kernel writes to the port (see `QemuExitCode` in the kernel crate).
const QEMU_EXIT_SUCCESS: i32 = (0x10 << 1) | 1;
const QEMU_EXIT_FAILED: i32 = (0x11 << 1) | 1;

fn main() -> Result<()> {
    let kernel = std::env!("KERNEL_PATH");

    // Create a temporary directory to store the EFI boot files
    let dir = TempDir::new("kernel")?;

    // Create the EFI boot directory
    let efi_boot = dir.path().join("EFI").join("BOOT");
    std::fs::create_dir_all(&efi_boot)?;

    // Copy the kernel to the EFI boot directory
    std::fs::copy(&kernel, efi_boot.join("BOOTX64.EFI")).unwrap();

    let mut cmd = std::process::Command::new("qemu-system-x86_64");
    cmd.args(["-nodefaults", "-display", "none", "-serial", "stdio"]);
    cmd.args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]);
    cmd.arg("-bios").arg(ovmf_prebuilt::ovmf_pure_efi());
    cmd.args([
        "-drive",
        &format!("format=raw,file=fat:rw:{}", dir.path().display()),
    ]);
    let status = cmd.status()?;

    // Clean up the temporary directory
    dir.close()?;

    match status.code() {
        Some(QEMU_EXIT_SUCCESS) => Ok(()),
        Some(QEMU_EXIT_FAILED) => bail!("kernel reported failure"),
        Some(code) => bail!("QEMU exited unexpectedly with status {code}"),
        None => bail!("QEMU was terminated by a signal"),
    }
}
