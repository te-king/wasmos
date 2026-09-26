//! Discovery of the platform's processors.
//!
//! The processors are enumerated through UEFI's MP Services protocol, which
//! is only available before boot services are exited. The kernel heap isn't
//! set up at that point, so the results are kept in a fixed-size array.

use uefi::{boot, proto::pi::mp::MpServices};

use crate::logln;

/// The most processors the kernel keeps track of.
pub const MAX_CPUS: usize = 256;

/// A processor found at boot.
#[derive(Clone, Copy, Debug, Default)]
pub struct Processor {
    /// Local APIC ID, which is how inter-processor interrupts address it.
    pub apic_id: u64,
    pub is_bsp: bool,
    pub is_enabled: bool,
    pub is_healthy: bool,
    pub package: u32,
    pub core: u32,
    pub thread: u32,
}

/// The processors found at boot, in the firmware's order.
pub struct Processors {
    list: [Processor; MAX_CPUS],
    len: usize,
    /// How many processors the firmware reported, which can exceed
    /// [`MAX_CPUS`].
    total: usize,
    /// How many of those the firmware reported as enabled.
    enabled: usize,
}

impl Processors {
    pub fn iter(&self) -> impl Iterator<Item = &Processor> {
        self.list[..self.len].iter()
    }

    /// Logs the discovered processors.
    pub fn log(&self) {
        let plural = if self.total == 1 { "" } else { "s" };
        logln!("smp: {} processor{}, {} enabled", self.total, plural, self.enabled);
        if self.total > self.len {
            logln!("smp: only tracking the first {}", self.len);
        }
        for (index, cpu) in self.iter().enumerate() {
            logln!(
                "smp: {}: apic {}, package {} core {} thread {}{}{}{}",
                index,
                cpu.apic_id,
                cpu.package,
                cpu.core,
                cpu.thread,
                if cpu.is_bsp { ", bsp" } else { "" },
                if cpu.is_enabled { "" } else { ", disabled" },
                if cpu.is_healthy { "" } else { ", unhealthy" },
            );
        }
    }
}

/// Enumerates the processors through UEFI's MP Services protocol.
///
/// Must be called before boot services are exited. The protocol is closed
/// again before this returns.
pub fn discover() -> uefi::Result<Processors> {
    let handle = boot::get_handle_for_protocol::<MpServices>()?;
    let mp = boot::open_protocol_exclusive::<MpServices>(handle)?;

    let count = mp.get_number_of_processors()?;
    let mut processors = Processors {
        list: [Processor::default(); MAX_CPUS],
        len: count.total.min(MAX_CPUS),
        total: count.total,
        enabled: count.enabled,
    };

    for (index, cpu) in processors.list[..processors.len].iter_mut().enumerate() {
        let info = mp.get_processor_info(index)?;
        *cpu = Processor {
            apic_id: info.processor_id,
            is_bsp: info.is_bsp(),
            is_enabled: info.is_enabled(),
            is_healthy: info.is_healthy(),
            package: info.location.package,
            core: info.location.core,
            thread: info.location.thread,
        };
    }

    Ok(processors)
}
