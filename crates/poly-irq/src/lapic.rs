//! The local APIC (SDM Vol. 3A chapter 11).
//!
//! In xAPIC mode the registers are a 4 KiB memory-mapped page (default
//! physical address 0xfee0_0000, see `IA32_APIC_BASE`), one 32-bit register
//! every 16 bytes (§11.4.1, table 11-1 "Local APIC Register Address Map"). In
//! x2APIC mode the same registers are MSRs 0x800 + offset / 16 (§11.12.1.2,
//! table 11-6), the ID is 32 bits wide and the interrupt command register is
//! one 64-bit MSR. [`ApicRegisters`] hides that difference, with [`XApic`]
//! over [`Mmio`] and [`X2Apic`] over [`Msr`]; [`LocalApic`] holds everything
//! else and is the same code in both modes.
//!
//! The local APIC is per CPU: every CPU programs its own, and in xAPIC mode
//! each CPU reaches its own through the same physical address.

use core::hint::spin_loop;
use core::sync::atomic::{Ordering, fence};

use crate::time::{
    Calibration, CalibrationConfig, CalibrationError, CalibrationReport, Counter, calibrate_report, mul_div,
};
use crate::{CpuFeatures, DeliveryMode, Mmio, Msr, Polarity, Trigger};

mod fields;
mod local;
mod registers;

pub use fields::*;
pub use local::*;
pub use registers::*;

#[cfg(test)]
mod tests;
