use anyhow::Result;
use tempdir::TempDir;

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
    let _ = cmd.spawn()?.wait()?;

    // Clean up the temporary directory
    dir.close()?;

    Ok(())
}
