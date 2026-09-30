//! Discovery and startup of the platform's processors.
//!
//! Everything needed to start the application processors is gathered by
//! [`prepare`] while boot services are available: the processors, which are
//! enumerated through UEFI's MP Services protocol, and the [`Trampoline`]
//! page they start in. [`Startup::start`] starts them once boot services
//! are exited, with INIT and startup IPIs.

use alloc::vec::Vec;
use core::{error, fmt, iter};

use uefi::{
    boot,
    proto::pi::mp::{MpServices, ProcessorInformation},
};

use super::{
    boot::{Ap, BootServices},
    int::{self, IpiDestination},
    trampoline::{Armed, Trampoline},
};
use crate::timer::Ticks;

/// How long to wait for a processor to enter the kernel, in timer ticks.
/// It takes well under a tick; this is about a second.
const START_TIMEOUT: usize = 100;

/// A processor found at boot.
#[derive(Clone, Copy, Debug)]
pub struct Processor {
    /// Logical index, which its per-CPU block gets. The bootstrap processor
    /// is 0, and the rest follow in the firmware's order.
    pub id: u32,
    /// Local APIC ID, which is how inter-processor interrupts address it.
    pub apic_id: u64,
    pub is_enabled: bool,
    pub is_healthy: bool,
    pub package: u32,
    pub core: u32,
    pub thread: u32,
}

/// The processors found at boot, and what's needed to start them.
pub struct Startup {
    /// Every processor, the bootstrap processor first.
    pub processors: Vec<Processor>,
    /// The page the application processors start in and the ones to start,
    /// or `None` if there are none.
    launch: Option<(Trampoline, Vec<Target>)>,
}

/// An application processor to start.
#[derive(Clone, Copy)]
struct Target {
    processor: Processor,
    dest: IpiDestination,
}

/// Why the application processors can't be started.
#[derive(Debug)]
pub enum PrepareError {
    /// The MP Services protocol is missing or one of its calls failed.
    Firmware(uefi::Error),
    /// The firmware didn't report exactly one bootstrap processor.
    BspCount(usize),
    /// The local APIC can't address the processor in its current mode.
    Unaddressable(Processor),
    /// No page below 1 MiB was free for the trampoline.
    Trampoline(uefi::Error),
}

/// An application processor didn't enter the kernel in time.
#[derive(Debug)]
pub struct Timeout(pub Processor);

impl Processor {
    fn new(id: u32, info: &ProcessorInformation) -> Self {
        Processor {
            id,
            apic_id: info.processor_id,
            is_enabled: info.is_enabled(),
            is_healthy: info.is_healthy(),
            package: info.location.package,
            core: info.location.core,
            thread: info.location.thread,
        }
    }

    pub fn is_bsp(&self) -> bool {
        self.id == 0
    }
}

impl From<uefi::Error> for PrepareError {
    fn from(err: uefi::Error) -> Self {
        PrepareError::Firmware(err)
    }
}

/// E.g. "cpu 1: apic 1, package 0 core 1 thread 0".
impl fmt::Display for Processor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cpu {}{}: apic {}, package {} core {} thread {}{}{}",
            self.id,
            if self.is_bsp() { " (bsp)" } else { "" },
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
impl fmt::Display for Startup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.processors.len();
        let enabled = self.processors.iter().filter(|p| p.is_enabled).count();
        let plural = if total == 1 { "" } else { "s" };
        write!(f, "{total} processor{plural}, {enabled} enabled")
    }
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrepareError::Firmware(err) => write!(f, "MP Services unavailable: {err:?}"),
            PrepareError::BspCount(count) => {
                write!(f, "firmware reported {count} bootstrap processors")
            }
            PrepareError::Unaddressable(p) => {
                write!(f, "cpu {}: apic {} can't be addressed", p.id, p.apic_id)
            }
            PrepareError::Trampoline(err) => {
                write!(f, "no page below 1 MiB for the trampoline: {err:?}")
            }
        }
    }
}

impl fmt::Display for Timeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cpu {}: apic {} didn't start", self.0.id, self.0.apic_id)
    }
}

impl error::Error for PrepareError {}
impl error::Error for Timeout {}

/// Finds the processors, and reserves the trampoline if there are
/// application processors to start.
pub fn prepare(firmware: &BootServices) -> Result<Startup, PrepareError> {
    let processors = discover(firmware)?;
    let targets = processors
        .iter()
        .filter(|p| !p.is_bsp() && p.is_enabled && p.is_healthy)
        .map(|&processor| {
            int::ipi_destination(processor.apic_id)
                .map(|dest| Target { processor, dest })
                .ok_or(PrepareError::Unaddressable(processor))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let launch = (!targets.is_empty())
        .then(|| Trampoline::reserve(firmware))
        .transpose()
        .map_err(PrepareError::Trampoline)?
        .map(|trampoline| (trampoline, targets));
    Ok(Startup { processors, launch })
}

/// Enumerates the processors through UEFI's MP Services protocol, the
/// bootstrap processor first.
///
/// Needs boot services, hence the token. The protocol is closed again before
/// this returns.
fn discover(_: &BootServices) -> Result<Vec<Processor>, PrepareError> {
    let handle = boot::get_handle_for_protocol::<MpServices>()?;
    let mp = boot::open_protocol_exclusive::<MpServices>(handle)?;

    let count = mp.get_number_of_processors()?;
    let infos = (0..count.total)
        .map(|index| mp.get_processor_info(index))
        .collect::<uefi::Result<Vec<_>>>()?;

    let (bsps, aps): (Vec<_>, Vec<_>) = infos.iter().partition(|info| info.is_bsp());
    let [bsp] = bsps[..] else {
        return Err(PrepareError::BspCount(bsps.len()));
    };

    Ok(iter::once(bsp)
        .chain(aps)
        .zip(0..)
        .map(|(info, id)| Processor::new(id, info))
        .collect())
}

impl Startup {
    /// Starts the application processors, one at a time, each running
    /// `main` with its first boot stage.
    ///
    /// Needs the clock, since the delays between IPIs are timed in ticks.
    /// Gives up at the first processor that doesn't start, consuming the
    /// trampoline: that processor might still arrive and read its handoff,
    /// so it must not be prepared again.
    pub async fn start(self, ticks: &mut Ticks, main: fn(Ap) -> !) -> Result<(), Timeout> {
        let Some((trampoline, targets)) = self.launch else {
            return Ok(());
        };
        let mut trampoline = trampoline.arm(main);
        for target in targets {
            start_one(ticks, &mut trampoline, target).await?;
        }
        Ok(())
    }
}

/// Starts one processor with the INIT, startup, startup IPI sequence.
async fn start_one(
    ticks: &mut Ticks,
    trampoline: &mut Armed,
    Target { processor, dest }: Target,
) -> Result<(), Timeout> {
    let trampoline = trampoline.prepare(processor.id);

    // SAFETY: `dest` is an application processor that the kernel hasn't
    // started, so it is parked by the firmware, running nothing of ours.
    unsafe { int::send_init(dest) };
    // Two ticks is at least one full tick, about 10 ms.
    ticks.sleep(2).await;
    // Intel's sequence sends a second startup IPI in case the first is
    // missed. A processor that has already started ignores it.
    for limit in [2, START_TIMEOUT] {
        // SAFETY: `trampoline` is prepared to start this processor from the
        // page at `vector`.
        unsafe { int::send_startup(dest, trampoline.vector()) };
        if ticks.within(limit, || trampoline.arrived()).await {
            return Ok(());
        }
    }
    Err(Timeout(processor))
}
