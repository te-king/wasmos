//! Guests: wasm programs, each run as a future.
//!
//! A guest runs on fuel, which wasmi burns roughly one unit per
//! instruction. It gets a [`SLICE`] at a time, and once that's spent it
//! yields to the executor before carrying on, so a guest stuck in a loop
//! can't stall the rest of the kernel. Only a call to a host function, or
//! the start function, runs without yielding.
//!
//! A host function that has to wait returns a [`Request`], which suspends
//! the guest too: the kernel awaits it, then resumes the guest with the
//! host function's results.

use alloc::vec::Vec;

use wasmi::{
    CompilationMode, Config, Engine, Error, Func, Module, ResumableCall, ResumableCallHostTrap,
    ResumableCallOutOfFuel, Store, Val,
};

use crate::{
    executor,
    host::{self, Request},
    timer::Timer,
};

/// The fuel a guest runs on between yields. Under QEMU without KVM, wasmi
/// gets through about 300,000 a second, so this is about 35 ms; on hardware
/// it's far less, and a smaller slice would spend more time resuming.
const SLICE: u64 = 10_000;

/// The fuel a guest's start function gets. It can't be suspended, so
/// running out traps the guest.
const START_FUEL: u64 = 100 * SLICE;

/// Runs a guest module's entry point to completion, yielding between
/// slices, and sleeping on `timer` when it asks to.
pub async fn run(wasm: &[u8], timer: &Timer) -> Result<(), Error> {
    let (mut store, entry) = instantiate(wasm)?;
    let mut suspended = suspension(entry.call_resumable(&mut store, &[], &mut [])?)?;
    while let Some(call) = suspended {
        let resumed = match call {
            Suspended::OutOfFuel(call) => {
                executor::yield_now().await;
                store.set_fuel(refill(call.required_fuel()))?;
                call.resume(&mut store, &mut [])?
            }
            Suspended::Request(call, request) => {
                let results = fulfil(request, timer).await;
                call.resume(&mut store, &results, &mut [])?
            }
        };
        suspended = suspension(resumed)?;
    }
    Ok(())
}

/// A guest's call that has stopped short of finishing, to be resumed.
enum Suspended {
    /// It ran out of fuel.
    OutOfFuel(ResumableCallOutOfFuel),
    /// A host function returned a request, whose results it resumes with.
    Request(ResumableCallHostTrap, Request),
}

/// What's left of `call` to resume, or nothing once it has finished. A
/// host function's error is either a request, which suspends the guest, or
/// bad guest input, which traps it.
fn suspension(call: ResumableCall) -> Result<Option<Suspended>, Error> {
    match call {
        ResumableCall::Finished => Ok(None),
        ResumableCall::OutOfFuel(call) => Ok(Some(Suspended::OutOfFuel(call))),
        ResumableCall::HostTrap(call) => match call.host_error().downcast_ref::<Request>() {
            Some(&request) => Ok(Some(Suspended::Request(call, request))),
            None => Err(call.into_host_error()),
        },
    }
}

/// Does what `request` asks, and gives the results its host function
/// returns.
async fn fulfil(request: Request, timer: &Timer) -> Vec<Val> {
    match request {
        Request::Sleep(ticks) => {
            timer.sleep(ticks).await;
            Vec::new()
        }
    }
}

/// The fuel for the next slice of a guest whose next step needs `required`.
/// A single step can need more than a slice (bulk memory operations cost
/// fuel per byte), and a guest refilled with less would never progress.
fn refill(required: u64) -> u64 {
    SLICE.max(required)
}

/// A fresh instance of `wasm`, whose start function has run, and its entry
/// point. Its store holds the fuel for the entry point's first slice.
fn instantiate(wasm: &[u8]) -> Result<(Store<()>, Func), Error> {
    // Compiled eagerly, when the module is created: compiling a function
    // lazily, on its first call, burns fuel too, and running out there is an
    // error rather than a call that can be resumed.
    let engine = Engine::new(
        Config::default()
            .consume_fuel(true)
            .compilation_mode(CompilationMode::Eager),
    );
    let module = Module::new(&engine, wasm)?;
    let mut store = Store::new(&engine, ());
    store.set_fuel(START_FUEL)?;
    let instance = host::linker(&engine)?.instantiate_and_start(&mut store, &module)?;
    // Typed, to check the entry point's signature.
    let entry = *instance
        .get_typed_func::<(), ()>(&store, wasmos_abi::ENTRY)?
        .func();
    store.set_fuel(SLICE)?;
    Ok((store, entry))
}
