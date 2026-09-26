//! Discovery of the platform's processors.
//!
//! The processors are enumerated through UEFI's MP Services protocol, which
//! is only available before boot services are exited.

use alloc::vec::Vec;

use uefi::{boot, proto::pi::mp::MpServices};

use crate::logln;

/// A processor found at boot.
#[derive(Clone, Copy, Debug)]
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
    list: Vec<Processor>,
    /// How many processors the firmware reported as enabled.
    enabled: usize,
}

impl Processors {
    pub fn iter(&self) -> impl Iterator<Item = &Processor> {
        self.list.iter()
    }

    /// Logs the discovered processors.
    pub fn log(&self) {
        let total = self.list.len();
        let plural = if total == 1 { "" } else { "s" };
        logln!("smp: {} processor{}, {} enabled", total, plural, self.enabled);
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
    let list = (0..count.total)
        .map(|index| {
            let info = mp.get_processor_info(index)?;
            Ok(Processor {
                apic_id: info.processor_id,
                is_bsp: info.is_bsp(),
                is_enabled: info.is_enabled(),
                is_healthy: info.is_healthy(),
                package: info.location.package,
                core: info.location.core,
                thread: info.location.thread,
            })
        })
        .collect::<uefi::Result<_>>()?;

    Ok(Processors {
        list,
        enabled: count.enabled,
    })
}
