use std::path::PathBuf;

fn main() {
    let kernel = PathBuf::from(std::env::var_os("CARGO_BIN_FILE_KERNEL_kernel").unwrap());
    println!("cargo:rustc-env=KERNEL_PATH={}", kernel.display());
}
