//! Discovery of the platform's processors.
//!
//! The processors are enumerated through UEFI's MP Services protocol, which
//! is only available before boot services are exited.

use alloc::vec::Vec;
use core::{fmt, iter};

use uefi::{
    boot,
    proto::pi::mp::{MpServices, ProcessorInformation},
};

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
/// processor, and it comes first, so a processor's position in [`iter`]
/// is its logical index (the one passed to `cpu::init`).
///
/// [`iter`]: Processors::iter
#[derive(Clone, Debug)]
pub struct Processors {
    pub bsp: Processor,
    pub aps: Vec<Processor>,
    /// How many processors the firmware reported as enabled.
    pub enabled: usize,
}

/// Why processor discovery failed.
#[derive(Debug)]
pub enum DiscoveryError {
    /// The MP Services protocol is missing or one of its calls failed.
    Firmware(uefi::Error),
    /// The firmware didn't report exactly one bootstrap processor.
    BspCount(usize),
}

impl Processors {
    /// All processors, bootstrap processor first.
    pub fn iter(&self) -> impl Iterator<Item = &Processor> {
        iter::once(&self.bsp).chain(&self.aps)
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
        write!(f, "{total} processor{plural}, {} enabled", self.enabled)
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

/// Enumerates the processors through UEFI's MP Services protocol.
///
/// Must be called before boot services are exited. The protocol is closed
/// again before this returns.
pub fn discover() -> Result<Processors, DiscoveryError> {
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
        enabled: count.enabled,
    })
}
