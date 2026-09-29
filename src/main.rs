use std::env::{self, VarError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};
use tempfile::TempDir;

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
    let timeout = timeout(env::var("WASMOS_TIMEOUT"))?;
    let (code, vars) = firmware()?;
    let drive = boot_drive(Path::new(env!("KERNEL_PATH")))?;

    let mut qemu = Command::new("qemu-system-x86_64")
        .args(qemu_args(&code, &vars, drive.path()))
        .spawn()?;
    let status = wait_with_timeout(&mut qemu, timeout)?;

    drive.close()?;
    outcome(status.code())
}

/// A temporary directory laid out as a UEFI boot drive, with `kernel` as
/// its default boot loader.
fn boot_drive(kernel: &Path) -> Result<TempDir> {
    let dir = tempfile::Builder::new().prefix("kernel").tempdir()?;
    let efi_boot = dir.path().join("EFI").join("BOOT");
    fs::create_dir_all(&efi_boot)?;
    fs::copy(kernel, efi_boot.join("BOOTX64.EFI"))?;
    Ok(dir)
}

/// QEMU's arguments for booting the drive in directory `drive`, with the
/// OVMF images `code` and `vars`.
fn qemu_args(code: &Path, vars: &Path, drive: &Path) -> Vec<String> {
    [
        "-nodefaults",
        "-display",
        "none",
        "-serial",
        "stdio",
        "-smp",
        CPUS,
        // A reset stops QEMU rather than rebooting into the kernel again. On
        // a triple fault (on any processor) the kernel can't report
        // anything, and would otherwise boot in a loop until the timeout.
        "-no-reboot",
        "-device",
        "isa-debug-exit,iobase=0xf4,iosize=0x04",
    ]
    .map(String::from)
    .into_iter()
    .chain([
        "-drive".into(),
        pflash(code),
        "-drive".into(),
        pflash(vars),
        // Writable only because QEMU's IDE disks can't be read-only. The
        // kernel never writes to it.
        "-drive".into(),
        format!("format=raw,file=fat:rw:{}", drive.display()),
    ])
    .collect()
}

/// What QEMU's exit status says about the kernel.
fn outcome(code: Option<i32>) -> Result<()> {
    match code {
        Some(QEMU_EXIT_SUCCESS) => Ok(()),
        Some(QEMU_EXIT_FAILED) => bail!("kernel reported failure"),
        Some(0) => bail!("the machine reset or shut down, e.g. on a triple fault"),
        Some(code) => bail!("QEMU exited unexpectedly with status {code}"),
        None => bail!("QEMU was terminated by a signal"),
    }
}

/// The OVMF code and variable store images: the files `WASMOS_OVMF_CODE` and
/// `WASMOS_OVMF_VARS` name (a distribution's OVMF, say), or else the pinned
/// prebuilt, downloaded and verified into `target/ovmf` on first run.
fn firmware() -> Result<(PathBuf, PathBuf)> {
    match (
        env::var_os("WASMOS_OVMF_CODE"),
        env::var_os("WASMOS_OVMF_VARS"),
    ) {
        (Some(code), Some(vars)) => Ok((code.into(), vars.into())),
        (None, None) => {
            let ovmf = Prebuilt::fetch(
                Source::LATEST,
                concat!(env!("CARGO_MANIFEST_DIR"), "/target/ovmf"),
            )
            .context("failed to fetch OVMF firmware (or set WASMOS_OVMF_CODE and _VARS)")?;
            Ok((
                ovmf.get_file(Arch::X64, FileType::Code),
                ovmf.get_file(Arch::X64, FileType::Vars),
            ))
        }
        _ => bail!("set both WASMOS_OVMF_CODE and WASMOS_OVMF_VARS, or neither"),
    }
}

/// A read-only flash drive for one of the OVMF firmware images.
fn pflash(path: &Path) -> String {
    format!("if=pflash,format=raw,readonly=on,file={}", path.display())
}

/// The timeout that `var`, the value of `WASMOS_TIMEOUT`, sets.
fn timeout(var: Result<String, VarError>) -> Result<Option<Duration>> {
    match var {
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
