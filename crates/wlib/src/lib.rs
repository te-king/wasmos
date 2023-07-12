#[link(wasm_import_module = "host")]
extern "C" {
    pub fn hello(value: i32);
    pub fn wasmos_print(offset: *const u8, length: usize);
}

// implement print using wasmos_print
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        unsafe {
            let s = format!($($arg)*);
            $crate::wasmos_print(s.as_ptr(), s.len());
        }
    };
}

// implement println using print
#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n");
    };
    ($($arg:tt)*) => {
        $crate::print!("{}\n", format_args!($($arg)*));
    };
}
