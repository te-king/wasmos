//! Host functions: the kernel's side of the guest ABI (see `wasmos_abi`).
//!
//! A host function returns an `Err` in two cases. Bad input from a guest
//! traps the guest rather than panicking the kernel. And a [`Request`]
//! suspends the guest while the kernel does something that takes time,
//! then resumes it (see `guest::run`).

use core::{fmt, iter};

use wasmi::{Caller, Engine, Error, Extern, Linker, TrapCode, errors::HostError};
use wasmos_abi as abi;

use crate::log;

/// The most text logged under one hold of the log's lock, which also holds
/// off interrupts. At least 4 bytes, so a piece always fits a character.
const PIECE: usize = 256;

/// A linker with every host function defined.
pub fn linker(engine: &Engine) -> Result<Linker<()>, Error> {
    let mut linker = Linker::new(engine);
    linker.func_wrap(abi::MODULE, abi::PRINT, print)?;
    linker.func_wrap(abi::MODULE, abi::SLEEP, sleep)?;
    Ok(linker)
}

/// Something a guest asked for that takes time. A host function returns it
/// as its error, which suspends the guest; the kernel does the waiting, and
/// resumes the guest with the host function's results.
#[derive(Clone, Copy, Debug)]
pub enum Request {
    /// Wait for at least this many full timer periods. No results.
    Sleep(u64),
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Request::Sleep(ticks) => write!(f, "{}: sleep for {ticks} ticks", abi::SLEEP),
        }
    }
}

impl HostError for Request {}

/// Suspends the guest for at least `ticks` full timer periods.
fn sleep(_: Caller<'_, ()>, ticks: u64) -> Result<(), Error> {
    Err(Error::host(Request::Sleep(ticks)))
}

/// Writes UTF-8 text from guest memory to the kernel log.
fn print(caller: Caller<'_, ()>, ptr: u32, len: u32) -> Result<(), Error> {
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

    pieces(text, PIECE).for_each(|piece| log!("{piece}"));
    Ok(())
}

/// `text` split at character boundaries into pieces of at most `max` bytes.
fn pieces(text: &str, max: usize) -> impl Iterator<Item = &str> {
    iter::successors(
        Some(text.split_at(text.floor_char_boundary(max))),
        move |(_, rest)| (!rest.is_empty()).then(|| rest.split_at(rest.floor_char_boundary(max))),
    )
    .map(|(piece, _)| piece)
}
