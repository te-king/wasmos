#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use x86_64::instructions::hlt;

    crate::log::log_panic(format_args!("Kernel panic at: {:?}\n", info));
    super::qemu::exit_qemu(super::qemu::QemuExitCode::Failed);
    loop {
        hlt()
    }
}
