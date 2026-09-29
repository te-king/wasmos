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

`cargo run` exits 0 only if the kernel wrote `QemuExitCode::Success` to the isa-debug-exit port. It fails if the kernel reports failure (including any panic), if QEMU exits some other way, or on timeout. QEMU runs with `-no-reboot`, so a triple fault on any processor ends the run straight away ("the machine reset or shut down") instead of rebooting in a loop until the timeout.

Lint and format. The kernel and the wasm crates must be checked against their own targets. CI runs all of these and fails on any warning:

```sh
cargo fmt --all
cargo fmt --all --check
cargo clippy -p kernel --target x86_64-unknown-uefi -- -D warnings
cargo clippy -p wlib -p wshell --target wasm32-unknown-unknown -- -D warnings
cargo clippy -p wasmos -p wasmos-abi -- -D warnings
```

### Testing

There is no unit-test harness. The test is booting: after formatting and clippy, CI (`.github/workflows/ci.yml`) runs `cargo build --locked` and `cargo run --locked`, which fails on any kernel panic, reported failure or hang. The futures `kernel_main` joins (e.g. `tick_task`) act as boot-time smoke tests.

To check a specific behaviour, temporarily inject code and boot, then revert. Examples used here: a `panic!` to test the failure path, a read from an unmapped address (e.g. `0x7000_0000_0000`) or `mov rsp, <unmapped>; push rax` to test fault reports and the double-fault stack, a `hlt` loop with a short `WASMOS_TIMEOUT` to test hangs, `asm!("int 32")` to fire the timer handler, a bad pointer passed to `wlib::sys::wasmos_print` from wshell to test guest traps, a `loop {}` in wshell to check that the other futures still finish while a guest runs, and not setting `arrived` in `trampoline::enter` to test the processor startup timeout. To try other processor counts or topologies (e.g. `-smp 1`, `-smp 8,sockets=2,cores=2,threads=2`), temporarily change the runner's QEMU arguments in `src/main.rs`.

The runner downloads OVMF firmware (via `ovmf-prebuilt`, SHA-256 pinned) into `target/ovmf` on first run, so `cargo clean` forces a re-download.

## Architecture

### Workspace and build pipeline
- The root package (`src/main.rs`, `build.rs`) is the **host runner**, not the kernel. `build.rs` receives the kernel's `.efi` path through a cargo artifact dependency (`bindeps`, enabled in `.cargo/config.toml`). The runner copies it to `EFI/BOOT/BOOTX64.EFI` on a temporary FAT drive and boots QEMU with `-smp 4`, `-no-reboot` and OVMF as read-only pflash.
- `crates/kernel` is the UEFI kernel (`x86_64-unknown-uefi`, `no_std`). It depends on `crates/wshell` as a `wasm32-unknown-unknown` artifact and embeds it with `include_bytes!`.
- `crates/wshell` is the first guest program. `crates/wlib` is the guest-side standard library: raw host imports in `wlib::sys`, safe wrappers (`wlib::print`) and the `print!`/`println!` macros.
- **Guests** (`guest.rs`) run as futures: `guest::run` meters fuel (wasmi's `consume_fuel`) and, each time a slice runs out, yields to the executor (`executor::yield_now`) before resuming the call. So a guest stuck in a loop can't stall the kernel, only a host function or a start function (which gets a fixed budget, then traps) runs without yielding. `kernel_main` joins the shell with the other smoke-test futures.
- **Guest ABI:** `crates/abi` (`wasmos-abi`) holds the import module name, host function names and guest entry point as constants. The kernel defines host functions in `crates/kernel/src/host.rs` using them. `wlib` must use string literals in its import attributes, so it checks them against the constants with compile-time `assert!`s: renaming one side without the other fails the build. Host functions must turn bad guest input into a wasm trap (`Err(wasmi::Error)`), never a kernel panic. They run without yielding, so anything unbounded must be split up: `wasmos_print` logs in pieces of at most 256 bytes, since the log holds off interrupts while it writes.
- `x86_64-unknown-uefi` is a Windows-style (COFF) target: `#[thread_local]` fails to link (`_tls_index`), which is why per-CPU data uses the GS base instead.

### Architecture layer (`crates/kernel/src/arch/`)
- `arch/mod.rs` picks the architecture module with `#[cfg(target_arch)]` and re-exports the only interface the rest of the kernel may use: `Console` (the log's serial port type), `without_interrupts`, `disable_interrupts` and `wait_for_interrupt` (the executor's idle).
- Code outside `arch/` must not use `x86_64`, `x2apic` or other architecture crates directly. Those are `cfg(target_arch = "x86_64")` dependencies in `crates/kernel/Cargo.toml`. If neutral code needs something new, add it to the interface.
- Each architecture module owns its entry point, boot sequence, interrupt handling, panic handler and emulator exit (`arch/x86_64/qemu.rs`).
- aarch64 is planned: `arch/aarch64/mod.rs` is a placeholder. Building for another target currently stops at a `compile_error!` in `arch/mod.rs`, after all the portable dependencies have compiled.

### Boot sequence (`crates/kernel/src/arch/x86_64/boot.rs`, `mod.rs`)
The order is load-bearing, so it is enforced with typestates. Each stage is a zero-sized token that only the previous stage can produce, and each transition consumes it:

```rust
let firmware = unsafe { boot::BootServices::start() };
let processors = smp::discover(&firmware);   // needs boot services (UEFI MP Services)
let trampoline = Trampoline::reserve(&firmware); // a page below 1 MiB, also needs boot services
firmware
    .exit()               // exit boot services, mask legacy PIC, serial log, memory map -> allocator
    .on_kernel_stack(|heap| bsp_main(heap, processors, trampoline)) // leave the firmware's stack for good
// in bsp_main:
let clock = heap
    .init_cpu(0)          // own GDT + TSS (interrupt stacks), per-CPU block + LAPIC handle (needs the heap)
    .enable_interrupts()  // IDT, enable LAPIC with its timer stopped, sti
    .start_clock();       // BSP only: its LAPIC timer drives `timer`
finish(executor::block_on(async {
    smp::start(&clock, processors, trampoline, ap_main).await?; // times IPIs in ticks
    kernel_main().await?;
    ..
}))
```

- Anything that needs a stage should take its token (as `smp::discover` takes `&BootServices`) rather than rely on call order. The `unsafe` steps live inside the transitions, each with its own `SAFETY` comment.
- `finish` turns the `Result` of starting the processors and running `kernel_main` into the QEMU exit code. It's the only place the kernel decides success or failure. It never returns: after `exit()` there is no firmware to return to, so outside QEMU it powers off (runtime `ResetSystem`) on success and halts on failure. The panic handler halts too.
- Before `exit()`, allocation is served only by a 1 MiB static early heap (`mem.rs`, talc `Claim` source). A panic there is silent, because the serial port isn't up yet.
- `cpu::with` before `cpu::init` on that CPU is undefined behaviour. The typestates guarantee it (the IDT is only loaded after `init_cpu`), on application processors too.
- The BSP leaves the firmware's stack straight after `exit()` (`Heap::on_kernel_stack`). That stack is 128 KiB under OVMF with no guard page, and the allocator claims the conventional memory right below it. Running a guest takes about 310 KiB of stack, so staying on it silently corrupted the heap.
- Stacks (`stack.rs`) are 1 MiB (`KERNEL_SIZE`) on every processor. `stack::leak` returns a `Top`, which isn't `Copy`, so each stack has one user, and `stack::run_on` can safely switch to it. Nothing guards their bottoms yet: that needs the kernel to own the page tables, which OVMF maps read-only.
- Application processors run the same chain from `Heap` to `Interrupts`. They are started after `exit()`, so `trampoline::enter` makes their `Heap` token (`Heap::application_processor`, unsafe) and passes it to `ap_main`. They never reach `Clock`: `start_clock` asserts it's on the BSP (logical ID 0).

### Application processors (`smp.rs`, `trampoline.rs`)
- The long-term goal is for every processor to join an async executor on startup. For now each one sets up its per-CPU block and interrupts in `ap_main`, logs `cpu N: online` and halts. Nothing wakes it yet: its timer is stopped and nothing sends IPIs.
- UEFI MP Services only runs code on application processors until boot services are exited, after which the firmware parks them again. So it is only used for discovery. The kernel starts them itself with INIT and startup IPIs.
- A startup IPI starts a processor in real mode at a page below 1 MiB. The trampoline code (`global_asm!`) is copied into that page and takes the processor through protected mode into long mode. It uses the BSP's CR0, CR3, CR4 and EFER (minus PCIDE and LMA, which can't be set yet), then calls `trampoline::enter` on a fresh kernel stack. That switches to the kernel's boot GDT (`gdt::load_boot`), so the processor stops depending on the handoff before it signals arrival, and calls `ap_main(heap, id)`.
- The trampoline code only addresses memory relative to its page. Its data is a `repr(C)` `Handoff` at a fixed offset in the same page, whose field offsets the assembly gets from `offset_of!` `const` operands.
- `smp::start` is async. Processors start one at a time, reusing the handoff, and each signals `arrived` once it no longer needs it. A processor that doesn't arrive within about a second fails the boot, and after that the trampoline must not be prepared again, since the processor might still turn up.

### Per-CPU data (`cpu.rs`)
- Each CPU's `Cpu` block is reached through its GS base (`gs:[0]` holds a self-pointer).
- Access is only through `cpu::with(|cpu| ...)`, which disables interrupts and, being a closure, can't be held across an `.await`.
- The block's contents need not be `Send`/`Sync`. The LAPIC handle (x2apic's `LocalApic` is deliberately `!Send`) lives there.

### Descriptor tables and exceptions (`gdt.rs`, `exception.rs`)
- The kernel owns its GDT; nothing uses the firmware's after `init_cpu`. Every processor has its own table (for its own TSS), but all start with the same `SEGMENTS`, built with `from_raw_entries`, so `KERNEL_CODE`/`KERNEL_DATA` mean the same thing everywhere. IDT entries name `KERNEL_CODE` explicitly (`exception::gate`) rather than copying whatever CS holds.
- Double fault, NMI and machine check each run on their own 32 KiB interrupt stack (`gdt::InterruptStack`), so a double fault from a bad stack pointer is reported instead of triple faulting. The TSS must be loaded before the IDT, which the typestates guarantee (`init_cpu` before `enable_interrupts`).
- A breakpoint logs and resumes. Every other exception becomes a `Fault` (plain data with a `Display` impl) and panics as `cpu N: <fault> at <rip>`, with CR2 and the error code where the processor gives them.

### Interrupts and async
- Handlers (`int.rs`) do the minimum: record the event, wake a waker, EOI. The spurious handler must not EOI.
- Anything a handler wakes must be interrupt-safe (lock-free). Logic belongs in async tasks.
- `timer::ticks()` is a single-consumer `futures::Stream` of tick counts (about 100 Hz under QEMU, not calibrated). Every timer interrupt counts as a tick, so only the BSP runs its LAPIC timer (`start_clock`). `install_local_apic` stops the timer that x2apic's `enable` starts, which it does on application processors too. Fan-out to many waiters belongs in a task that owns the stream.
- `executor::block_on` runs one root future and halts the processor (`enable_and_hlt`) while it's pending. There is no task spawning: concurrency comes from composing futures (`join`, `select`, `FuturesUnordered`).
- Its waker only sets a static flag, so it never allocates or frees, even when woken from an interrupt handler. Keep it that way: freeing memory inside a handler could deadlock on the allocator lock.
- A wake from another processor won't interrupt a halted one. Once other processors run tasks, that needs an IPI.

### Dependency notes
- x2apic's IPI functions write `dest` into the upper half of the ICR as is, which is only right in x2APIC mode. In xAPIC mode the APIC ID belongs in the top byte, so always go through `int::ipi_destination`. Otherwise an IPI meant for APIC 1 goes to APIC 0, the BSP. QEMU without KVM gives xAPIC mode.
- `wasmi` is built with `default-features = false`. Keep `validate` (otherwise guest modules aren't validated) and `auto-dispatch` (otherwise unoptimised builds use tail-call dispatch that grows the kernel stack on every wasm instruction). Guests are compiled eagerly (`CompilationMode::Eager`): a lazy compile burns fuel, and running out there is a plain error rather than a resumable call. Running a guest takes about 310 KiB of kernel stack.

## Code style

The owner prefers **async**, **pure data / immutability**, and **functional style**:
- Keep side effects (hardware access, logging, global state) at the edges.
- Pass plain data between pure functions.
- Model event sources as futures/streams rather than callbacks.
- Prefer iterator/stream combinators and expression-oriented code over mutable loops where it reads naturally.

Other conventions in this codebase:
- Every `unsafe` block gets a `// SAFETY:` comment, and every `unsafe fn` a `# Safety` doc section. All crates are edition 2024, so the body of an `unsafe fn` isn't an unsafe context: wrap each unsafe operation in its own narrow `unsafe {}` block, whose `SAFETY` comment points back to the caller's obligations.
- Comments explain why, not what.
- `macro_rules!` expansions must not end in a trailing `;`: using such a macro in expression position is a hard error on current nightly.
- Build and boot both debug and release before committing. Keep dependency updates one crate per commit, each verified by a boot.
