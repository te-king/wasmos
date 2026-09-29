//! Discovery and startup of the platform's processors.
//!
//! The processors are enumerated through UEFI's MP Services protocol, which
//! is only available before boot services are exited. The application
//! processors are started afterwards, with INIT and startup IPIs into the
//! [`Trampoline`].

use alloc::vec::Vec;
use core::{error, fmt, future, iter};

use futures_util::{StreamExt, TryStreamExt, stream};
use uefi::{
    boot,
    proto::pi::mp::{MpServices, ProcessorInformation},
};

use super::{
    boot::{Ap, BootServices, Clock, Heap},
    cpu::{CpuId, Local},
    int,
    trampoline::Trampoline,
};
use crate::timer::{self, Ticks};

/// How long to wait for a processor to enter the kernel, in timer periods.
/// It takes well under one; this is about a second.
const START_TIMEOUT: u64 = 100;

/// A processor found at boot.
#[derive(Clone, Copy, Debug)]
pub struct Processor {
    /// Local APIC ID, which is how inter-processor interrupts address it.
    pub apic_id: u64,
    pub is_enabled: bool,
    pub is_healthy: bool,
    pub package: u32,
    pub core: u32,
    pub thread: u32,
}

/// The processors found at boot. There is always exactly one bootstrap
/// processor, and it comes first, so a processor's position is its id.
#[derive(Clone, Debug)]
pub struct Processors {
    pub bsp: Processor,
    pub aps: Vec<Processor>,
}

/// The processors as log lines: a summary, then one per processor, marking
/// the bootstrap processor.
pub struct Listing<'a>(pub &'a Processors);

/// Why processor discovery failed.
#[derive(Debug)]
pub enum DiscoveryError {
    /// The MP Services protocol is missing or one of its calls failed.
    Firmware(uefi::Error),
    /// The firmware didn't report exactly one bootstrap processor.
    BspCount(usize),
}

/// Why the application processors couldn't all be started.
#[derive(Debug)]
pub enum StartError {
    /// No page below 1 MiB was free for the trampoline.
    Trampoline(uefi::Error),
    /// The local APIC can't address the processor in its current mode.
    Unaddressable { id: CpuId, apic_id: u64 },
    /// The processor didn't enter the kernel in time.
    Timeout { id: CpuId, apic_id: u64 },
}

impl Processors {
    /// All processors, bootstrap processor first, with their ids.
    pub fn iter(&self) -> impl Iterator<Item = (CpuId, &Processor)> {
        (0..)
            .map(CpuId::nth)
            .zip(iter::once(&self.bsp).chain(&self.aps))
    }

    /// The application processors the kernel can start, with their ids.
    pub fn startable(&self) -> impl Iterator<Item = (CpuId, &Processor)> {
        self.iter()
            .filter(|&(id, ap)| id != CpuId::BSP && ap.is_enabled && ap.is_healthy)
    }

    /// How many processors are enabled.
    pub fn enabled(&self) -> usize {
        self.iter().filter(|(_, cpu)| cpu.is_enabled).count()
    }
}

impl From<&ProcessorInformation> for Processor {
    fn from(info: &ProcessorInformation) -> Self {
        Processor {
            apic_id: info.processor_id,
            is_enabled: info.is_enabled(),
            is_healthy: info.is_healthy(),
            package: info.location.package,
            core: info.location.core,
            thread: info.location.thread,
        }
    }
}

impl From<uefi::Error> for DiscoveryError {
    fn from(err: uefi::Error) -> Self {
        DiscoveryError::Firmware(err)
    }
}

impl fmt::Display for Processor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "apic {}, package {} core {} thread {}{}{}",
            self.apic_id,
            self.package,
            self.core,
            self.thread,
            if self.is_enabled { "" } else { ", disabled" },
            if self.is_healthy { "" } else { ", unhealthy" },
        )
    }
}

/// A one-line summary, e.g. "4 processors, 4 enabled".
impl fmt::Display for Processors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = 1 + self.aps.len();
        let plural = if total == 1 { "" } else { "s" };
        write!(f, "{total} processor{plural}, {} enabled", self.enabled())
    }
}

impl fmt::Display for Listing<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "smp: {}", self.0)?;
        self.0.iter().try_for_each(|(id, cpu)| {
            let role = if id == CpuId::BSP { " (bsp)" } else { "" };
            writeln!(f, "smp: cpu {id}{role}: {cpu}")
        })
    }
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiscoveryError::Firmware(err) => write!(f, "MP Services unavailable: {err:?}"),
            DiscoveryError::BspCount(count) => {
                write!(f, "firmware reported {count} bootstrap processors")
            }
        }
    }
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::Trampoline(err) => {
                write!(f, "no page below 1 MiB for the trampoline: {err:?}")
            }
            StartError::Unaddressable { id, apic_id } => {
                write!(f, "cpu {id}: apic {apic_id} can't be addressed")
            }
            StartError::Timeout { id, apic_id } => {
                write!(f, "cpu {id}: apic {apic_id} didn't start")
            }
        }
    }
}

impl error::Error for DiscoveryError {}

impl error::Error for StartError {}

/// Enumerates the processors through UEFI's MP Services protocol.
///
/// Needs boot services, hence the token. The protocol is closed again before
/// this returns.
pub fn discover(_: &BootServices) -> Result<Processors, DiscoveryError> {
    let handle = boot::get_handle_for_protocol::<MpServices>()?;
    let mp = boot::open_protocol_exclusive::<MpServices>(handle)?;

    let count = mp.get_number_of_processors()?;
    let infos = (0..count.total)
        .map(|index| mp.get_processor_info(index))
        .collect::<uefi::Result<Vec<_>>>()?;

    let (bsps, aps): (Vec<_>, Vec<_>) = infos.iter().partition(|info| info.is_bsp());
    let [bsp] = bsps[..] else {
        return Err(DiscoveryError::BspCount(bsps.len()));
    };

    Ok(Processors {
        bsp: bsp.into(),
        aps: aps.into_iter().map(Processor::from).collect(),
    })
}

/// Starts every enabled application processor, one at a time, each running
/// `main` with its first boot stage, which carries its id.
///
/// Needs the clock, since the delays between IPIs are timed in ticks.
pub async fn start(
    clock: &mut Clock,
    processors: &Processors,
    trampoline: Trampoline,
    main: fn(Heap<Ap>) -> !,
) -> Result<(), StartError> {
    let local = clock.local();
    // Each start hands the trampoline and the tick stream on to the next.
    stream::iter(processors.startable())
        .map(Ok)
        .try_fold(
            (trampoline, timer::ticks(clock)),
            |(trampoline, mut ticks), (id, ap)| async move {
                let trampoline = start_one(local, &mut ticks, trampoline, id, ap, main).await?;
                Ok((trampoline, ticks))
            },
        )
        .await
        .map(drop)
}

/// Starts one processor with the INIT, startup, startup IPI sequence, and
/// gives the trampoline back once the processor has let go of it.
async fn start_one(
    local: Local,
    ticks: &mut Ticks<'_>,
    trampoline: Trampoline,
    id: CpuId,
    ap: &Processor,
    main: fn(Heap<Ap>) -> !,
) -> Result<Trampoline, StartError> {
    let apic_id = ap.apic_id;
    let dest = int::ipi_destination(apic_id).ok_or(StartError::Unaddressable { id, apic_id })?;
    let launch = trampoline.launch(id, main);

    // SAFETY: `dest` is an application processor that the kernel hasn't
    // started, so it is parked by the firmware, running nothing of ours.
    unsafe { int::send_init(local, dest) };
    // Intel asks for 10 ms here, about one timer period.
    sleep(ticks, 1).await;
    // Intel's sequence sends a second startup IPI in case the first is
    // missed. A processor that has already started ignores it.
    for periods in [1, START_TIMEOUT] {
        // SAFETY: `launch` put the trampoline in the page at `vector`.
        unsafe { int::send_startup(local, dest, launch.vector()) };
        if wait_until(ticks, periods, || launch.arrived()).await {
            break;
        }
    }
    launch.land().ok_or(StartError::Timeout { id, apic_id })
}

/// Waits for at least `periods` full timer periods.
async fn sleep(ticks: &mut Ticks<'_>, periods: u64) {
    wait_until(ticks, periods, || false).await;
}

/// Waits for `done` to return true, checking it on each tick. Returns false
/// if it still hasn't after at least `periods` full timer periods.
///
/// The wait is bounded by a tick count rather than a number of items from
/// `ticks`, which coalesces missed ticks: its first item can be one that
/// happened before this was called.
async fn wait_until(ticks: &mut Ticks<'_>, periods: u64, done: impl Fn() -> bool) -> bool {
    let deadline = timer::after(timer::now(), periods);
    done() || {
        ticks
            .any(|now| future::ready(now >= deadline || done()))
            .await;
        done()
    }
}
