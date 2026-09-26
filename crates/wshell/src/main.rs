#![no_main]

use wlib::println;

// The guest entry point (`wasmos_abi::ENTRY`), called by the kernel.
#[no_mangle]
fn main() {
    println!("Hello, world!")
}
