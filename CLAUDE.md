# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

wasmos is an experimental bare-metal kernel that boots via UEFI on x86_64 and runs WebAssembly programs (interpreted with wasmi) as its applications.

## Commands

The toolchain is a pinned nightly (`rust-toolchain.toml`); rustup installs it and both targets automatically. Nightly is required for cargo's `bindeps` and the `x86-interrupt` ABI. When bumping it, move to a specific dated nightly, never floating `nightly`, since unstable-feature churn has broken the build before.

```sh
cargo build                # builds the runner, which builds the kernel and wshell as artifact deps
cargo run                  # boots the kernel in QEMU (needs qemu-system-x86_64)
cargo run --release        # wasmi uses a different dispatch loop when optimised; check both
WASMOS_TIMEOUT=10 cargo run  # QEMU is killed after N seconds (default 60, 0 = no limit)
```

`cargo run` exits 0 only if the kernel wrote `QemuExitCode::Success` to the isa-debug-exit port. It fails if the kernel reports failure (including any panic), if QEMU exits some other way, or on timeout.

Lint and format. The kernel and the wasm crates must be checked against their own targets:

```sh
cargo fmt --all
cargo clippy -p kernel --target x86_64-unknown-uefi
cargo clippy -p wlib -p wshell --target wasm32-unknown-unknown
cargo clippy -p wasmos
```

### Testing

There is no unit-test harness. The test is booting: CI (`.github/workflows/ci.yml`) runs `cargo build --locked` and `cargo run --locked`, which fails on any kernel panic, reported failure or hang. Tasks spawned in `kernel_main` (e.g. `tick_task`) act as boot-time smoke tests.

To check a specific behaviour, temporarily inject code and boot, then revert. Examples used here: a `panic!` to test the failure path, a `hlt` loop with a short `WASMOS_TIMEOUT` to test hangs, `asm!("int 32")` to fire the timer handler, and a bad pointer passed to `wlib::wasmos_print` from wshell to test guest traps.

The runner downloads OVMF firmware (via `ovmf-prebuilt`, SHA-256 pinned) into `target/ovmf` on first run, so `cargo clean` forces a re-download.

## Architecture

### Workspace and build pipeline
- The root package (`src/main.rs`, `build.rs`) is the **host runner**, not the kernel. `build.rs` receives the kernel's `.efi` path through a cargo artifact dependency (`bindeps`, enabled in `.cargo/config.toml`). The runner copies it to `EFI/BOOT/BOOTX64.EFI` on a temporary FAT drive and boots QEMU with `-smp 4` and OVMF as read-only pflash.
- `crates/kernel` is the UEFI kernel (`x86_64-unknown-uefi`, `no_std`). It depends on `crates/wshell` as a `wasm32-unknown-unknown` artifact and embeds it with `include_bytes!`.
- `crates/wshell` is the first guest program. `crates/wlib` is the guest-side standard library: it declares the host imports (wasm import module `"host"`) and the `print!`/`println!` macros.
- **Guest ABI:** host functions are defined in `kernel_main` (`crates/kernel/src/main.rs`) and declared by name in `wlib`. The names and signatures must match on both sides. Host functions must turn bad guest input into a wasm trap (`Err(wasmi::Error)`), never a kernel panic.
- `x86_64-unknown-uefi` is a Windows-style (COFF) target: `#[thread_local]` fails to link (`_tls_index`), which is why per-CPU data uses the GS base instead.

### Boot sequence (`crates/kernel/src/arch/x86_64/mod.rs`)
The order is load-bearing:
1. `smp::discover()` while boot services exist (UEFI MP Services).
2. `exit_boot_services`.
3. Serial port, then the memory map is added to the allocator.
4. `cpu::init(0, ...)`, which needs the heap.
5. IDT, mask the legacy PIC, enable the LAPIC (this starts its timer), enable interrupts.
6. `kernel_main`.

Consequences:
- Before `exit_boot_services`, allocation is served only by a 1 MiB static early heap (`mem.rs`, talc `Claim` source). A panic there is silent, because the serial port isn't up yet.
- `cpu::with` before `cpu::init` on that CPU is undefined behaviour. That is why `init` runs before the IDT is loaded.

### Per-CPU data (`cpu.rs`)
- Each CPU's `Cpu` block is reached through its GS base (`gs:[0]` holds a self-pointer).
- Access is only through `cpu::with(|cpu| ...)`, which disables interrupts and, being a closure, can't be held across an `.await`.
- The block's contents need not be `Send`/`Sync`. The LAPIC handle (x2apic's `LocalApic` is deliberately `!Send`) lives there.

### Interrupts and async
- Handlers (`int.rs`) do the minimum: record the event, wake a waker, EOI. The spurious handler must not EOI.
- Anything a handler wakes must be interrupt-safe (lock-free). Logic belongs in async tasks.
- `timer::ticks()` is a single-consumer `futures::Stream` of tick counts (about 100 Hz under QEMU, not calibrated). Fan-out to many waiters belongs in a task that owns the stream.
- `sync::executor::SimpleExecutor` currently busy-polls with a no-op waker.

### Dependency notes
- `wasmi` is built with `default-features = false`. Keep `validate` (otherwise guest modules aren't validated) and `auto-dispatch` (otherwise unoptimised builds use tail-call dispatch that grows the kernel stack on every wasm instruction).

## Code style

The owner prefers **async**, **pure data / immutability**, and **functional style**:
- Keep side effects (hardware access, logging, global state) at the edges.
- Pass plain data between pure functions.
- Model event sources as futures/streams rather than callbacks.
- Prefer iterator/stream combinators and expression-oriented code over mutable loops where it reads naturally.

Other conventions in this codebase:
- Every `unsafe` block gets a `// SAFETY:` comment, and every `unsafe fn` a `# Safety` doc section.
- Comments explain why, not what.
- `macro_rules!` expansions must not end in a trailing `;`: using such a macro in expression position is a hard error on current nightly.
- Build and boot both debug and release before committing. Keep dependency updates one crate per commit, each verified by a boot.
