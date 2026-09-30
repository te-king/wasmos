#![no_main]

use wlib::println;

wlib::entry!(main);

fn main() {
    println!("Hello, world!");
    wlib::sleep(5);
    println!("Slept for 5 ticks");
}
