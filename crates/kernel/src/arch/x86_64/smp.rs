//! Discovery and startup of the platform's processors.
//!
//! Everything needed to start the application processors is gathered by
//! [`prepare`] while boot services are available: the processors, which are
//! enumerated through UEFI's MP Services protocol, their IPI destinations,
//! and the [`Trampoline`] page they start in. [`Startup::start`] starts them
//! once boot services are exited, with INIT and startup IPIs.

use alloc::vec::Vec;
use core::{fmt, iter};

use thiserror::Error;
use uefi::{
    boot,
    proto::pi::mp::{MpServices, ProcessorInformation},
};

use super::{
    boot::{Ap, BootServices, Heap},
    cpu::{CpuId, Local},
    int::{self, Ipi, IpiDestination},
    trampoline::{Armed, Trampoline},
};
use crate::{timer::Timer, work};

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

/// The processors found at boot, and what's needed to start them.
pub struct Startup {
    pub processors: Processors,
    /// The page the application processors start in and the ones to start,
    /// or `None` if there are none.
    launch: Option<(Trampoline, Vec<Target>)>,
}

/// An application processor to start.
#[derive(Clone, Copy)]
struct Target {
    id: CpuId,
    apic_id: u64,
    dest: IpiDestination,
}

/// Why the application processors can't be started.
#[derive(Debug, Error)]
pub enum PrepareError {
    #[error("MP Services unavailable: {0:?}")]
    Firmware(#[from] uefi::Error),
    #[error("firmware reported {0} bootstrap processors")]
    BspCount(usize),
    /// The local APIC can't address the processor in its current mode.
    #[error("cpu {id}: apic {apic_id} can't be addressed")]
    Unaddressable { id: CpuId, apic_id: u64 },
    #[error("no page below 1 MiB for the trampoline: {0:?}")]
    Trampoline(uefi::Error),
}

/// An application processor didn't enter the kernel in time.
#[derive(Debug, Error)]
#[error("cpu {id}: apic {apic_id} didn't start")]
pub struct Timeout {
    id: CpuId,
    apic_id: u64,
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

/// The processors as log lines: a summary, then one per processor, marking
/// the bootstrap processor.
impl fmt::Display for Processors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = 1 + self.aps.len();
        let plural = if total == 1 { "" } else { "s" };
        let enabled = self.iter().filter(|(_, cpu)| cpu.is_enabled).count();
        writeln!(f, "smp: {total} processor{plural}, {enabled} enabled")?;
        self.iter().try_for_each(|(id, cpu)| {
            let role = if id == CpuId::BSP { " (bsp)" } else { "" };
            writeln!(f, "smp: cpu {id}{role}: {cpu}")
        })
    }
}

/// Finds the processors, and reserves the trampoline if there are
/// application processors to start.
pub fn prepare(firmware: &BootServices) -> Result<Startup, PrepareError> {
    let processors = discover(firmware)?;
    let targets = processors
        .startable()
        .map(|(id, ap)| {
            let apic_id = ap.apic_id;
            int::ipi_destination(apic_id)
                .map(|dest| Target { id, apic_id, dest })
                .ok_or(PrepareError::Unaddressable { id, apic_id })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let launch = (!targets.is_empty())
        .then(|| Trampoline::reserve(firmware))
        .transpose()
        .map_err(PrepareError::Trampoline)?
        .map(|trampoline| (trampoline, targets));
    Ok(Startup { processors, launch })
}

/// Enumerates the processors through UEFI's MP Services protocol.
///
/// Needs boot services, hence the token. The protocol is closed again before
/// this returns.
fn discover(_: &BootServices) -> Result<Processors, PrepareError> {
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

    Ok(Processors {
        bsp: bsp.into(),
        aps: aps.into_iter().map(Processor::from).collect(),
    })
}

impl Startup {
    /// Starts the application processors, one at a time, each running
    /// `main` with its first boot stage, which carries its id. Makes every
    /// processor a mailbox for work, and opens each one's once it has
    /// started.
    ///
    /// Needs the timer, since the delays between IPIs are timed in ticks.
    pub async fn start(
        self,
        local: Local,
        timer: &Timer,
        main: fn(Heap<Ap>) -> !,
    ) -> Result<(), Timeout> {
        work::init(self.processors.iter().count());
        let Some((trampoline, targets)) = self.launch else {
            return Ok(());
        };
        let mut trampoline = trampoline.arm(main);
        for target in targets {
            trampoline = start_one(local, timer, trampoline, target).await?;
            work::open(target.id);
        }
        Ok(())
    }
}

/// Starts one processor with the INIT, startup, startup IPI sequence, and
/// gives the trampoline back once the processor has let go of it.
async fn start_one(
    local: Local,
    timer: &Timer,
    trampoline: Armed,
    Target { id, apic_id, dest }: Target,
) -> Result<Armed, Timeout> {
    let launch = trampoline.launch(id);

    // SAFETY: `dest` is an application processor that the kernel hasn't
    // started, so it is parked by the firmware, running nothing of ours.
    unsafe { int::send_ipi(local, dest, Ipi::Init) };
    // Intel asks for 10 ms here, about one timer period.
    timer.sleep(1).await;
    // Intel's sequence sends a second startup IPI in case the first is
    // missed. A processor that has already started ignores it.
    for periods in [1, START_TIMEOUT] {
        // SAFETY: `launch` put the trampoline in the page at `vector`.
        unsafe { int::send_ipi(local, dest, Ipi::Startup(launch.vector())) };
        if timer.within(periods, || launch.arrived()).await {
            break;
        }
    }
    launch.land().ok_or(Timeout { id, apic_id })
}
