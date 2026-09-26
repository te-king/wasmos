use std::env::VarError;
use std::path::Path;
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};

// QEMU's isa-debug-exit device exits with `(value << 1) | 1`, where `value` is
// what the kernel writes to the port (see `QemuExitCode` in the kernel crate).
const QEMU_EXIT_SUCCESS: i32 = (0x10 << 1) | 1;
const QEMU_EXIT_FAILED: i32 = (0x11 << 1) | 1;

// How long QEMU may run before it is killed. Override (in seconds) with
// `WASMOS_TIMEOUT`; a value of 0 disables the timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

// Processors given to QEMU, so the kernel has application processors to find.
const CPUS: &str = "4";

fn main() -> Result<()> {
    let kernel = std::env!("KERNEL_PATH");
    let timeout = timeout()?;

    // Download (and verify) the OVMF firmware on first run, then reuse the cache.
    let ovmf = Prebuilt::fetch(
        Source::LATEST,
        concat!(env!("CARGO_MANIFEST_DIR"), "/target/ovmf"),
    )
    .context("failed to fetch OVMF firmware")?;

    // Create a temporary directory to store the EFI boot files
    let dir = tempfile::Builder::new().prefix("kernel").tempdir()?;

    // Create the EFI boot directory
    let efi_boot = dir.path().join("EFI").join("BOOT");
    std::fs::create_dir_all(&efi_boot)?;

    // Copy the kernel to the EFI boot directory
    std::fs::copy(kernel, efi_boot.join("BOOTX64.EFI"))?;

    let mut cmd = std::process::Command::new("qemu-system-x86_64");
    cmd.args(["-nodefaults", "-display", "none", "-serial", "stdio"]);
    cmd.args(["-smp", CPUS]);
    cmd.args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]);
    cmd.arg("-drive")
        .arg(pflash(&ovmf.get_file(Arch::X64, FileType::Code)));
    cmd.arg("-drive")
        .arg(pflash(&ovmf.get_file(Arch::X64, FileType::Vars)));
    cmd.args([
        "-drive",
        &format!("format=raw,file=fat:rw:{}", dir.path().display()),
    ]);
    let mut child = cmd.spawn()?;
    let status = wait_with_timeout(&mut child, timeout)?;

    // Clean up the temporary directory
    dir.close()?;

    match status.code() {
        Some(QEMU_EXIT_SUCCESS) => Ok(()),
        Some(QEMU_EXIT_FAILED) => bail!("kernel reported failure"),
        Some(code) => bail!("QEMU exited unexpectedly with status {code}"),
        None => bail!("QEMU was terminated by a signal"),
    }
}

/// A read-only flash drive for one of the OVMF firmware images.
fn pflash(path: &Path) -> String {
    format!("if=pflash,format=raw,readonly=on,file={}", path.display())
}

fn timeout() -> Result<Option<Duration>> {
    match std::env::var("WASMOS_TIMEOUT") {
        Ok(secs) => {
            let secs: u64 = secs
                .parse()
                .with_context(|| format!("invalid WASMOS_TIMEOUT {secs:?}"))?;
            Ok((secs != 0).then(|| Duration::from_secs(secs)))
        }
        Err(VarError::NotPresent) => Ok(Some(DEFAULT_TIMEOUT)),
        Err(err) => Err(err).context("invalid WASMOS_TIMEOUT"),
    }
}

/// Waits for `child` to exit, killing it if it is still running after `timeout`.
fn wait_with_timeout(child: &mut Child, timeout: Option<Duration>) -> Result<ExitStatus> {
    let Some(timeout) = timeout else {
        return Ok(child.wait()?);
    };

    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            bail!("kernel timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
