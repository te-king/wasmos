#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use crate::logln;
    use x86_64::instructions::hlt;

    logln!("Kernel panic at: {:?}", info);
    loop {
        hlt()
    }
}
