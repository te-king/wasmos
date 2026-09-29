//! Host functions: the kernel's side of the guest ABI (see `wasmos_abi`).
//!
//! Bad input from a guest traps the guest (an `Err` from the host function)
//! rather than panicking the kernel.

use wasmi::{Caller, Engine, Error, Extern, Linker, TrapCode};
use wasmos_abi as abi;

use crate::log;

/// A linker with every host function defined.
pub fn linker<T: 'static>(engine: &Engine) -> Result<Linker<T>, Error> {
    let mut linker = Linker::new(engine);
    linker.func_wrap(abi::MODULE, abi::PRINT, print::<T>)?;
    Ok(linker)
}

/// Writes UTF-8 text from guest memory to the kernel log.
fn print<T: 'static>(caller: Caller<'_, T>, ptr: u32, len: u32) -> Result<(), Error> {
    let memory = caller
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| Error::new("wasmos_print: guest has no 'memory' export"))?;

    let start = ptr as usize;
    let bytes = start
        .checked_add(len as usize)
        .and_then(|end| memory.data(&caller).get(start..end))
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    let text = core::str::from_utf8(bytes)
        .map_err(|_| Error::new("wasmos_print: text is not valid UTF-8"))?;

    log!("{text}");
    Ok(())
}
