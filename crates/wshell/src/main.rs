#![no_main]

// use wlib::hello;
use wlib::println;

#[no_mangle]
fn main() {
    println!("Hello, world!")
}
