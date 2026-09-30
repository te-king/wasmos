#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    crate::log::log_panic(format_args!("Kernel panic at: {:?}\n", info));
    super::qemu::exit(super::qemu::QemuExitCode::Failed)
}
