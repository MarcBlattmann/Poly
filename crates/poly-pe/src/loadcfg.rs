//! The load configuration directory (spec "The Load Configuration
//! Structure (Image Only)"). Only parsed: the fields the spec lists up to
//! `GuardFlags`. The structure's own `Size` field says which fields exist;
//! fields past it read as zero.

use crate::headers::Pe;
use crate::read::{bytes, ptr_at, u16_at, u32_at};
use crate::{Error, dir};

/// `IMAGE_LOAD_CONFIG_DIRECTORY32/64` up to `GuardFlags`. Addresses are VAs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadConfig {
    /// The structure's size as it says (`Characteristics` in older headers).
    pub size: u32,
    pub time_date_stamp: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub global_flags_clear: u32,
    pub global_flags_set: u32,
    pub critical_section_default_timeout: u32,
    pub de_commit_free_block_threshold: u64,
    pub de_commit_total_free_threshold: u64,
    pub lock_prefix_table: u64,
    pub maximum_allocation_size: u64,
    pub virtual_memory_threshold: u64,
    pub process_affinity_mask: u64,
    pub process_heap_flags: u32,
    pub csd_version: u16,
    pub dependent_load_flags: u16,
    pub edit_list: u64,
    /// VA of the `/GS` security cookie the loader initialises.
    pub security_cookie: u64,
    /// x86 only: VA and count of the safe exception handler table.
    pub se_handler_table: u64,
    pub se_handler_count: u64,
    pub guard_cf_check_function_pointer: u64,
    pub guard_cf_dispatch_function_pointer: u64,
    pub guard_cf_function_table: u64,
    pub guard_cf_function_count: u64,
    /// `IMAGE_GUARD_*`, see [`crate::guard_flags`].
    pub guard_flags: u32,
}

impl<'a> Pe<'a> {
    /// The load configuration directory, if the image has one.
    pub fn load_config(&self) -> Result<Option<LoadConfig>, Error> {
        let Some(d) = self.info().directory(dir::LOAD_CONFIG) else { return Ok(None) };
        let size = self.view.u32(d.virtual_address)?;
        if size < 4 {
            return Err(Error::Malformed("load config smaller than its size field"));
        }
        // The structure must be in the data as long as it says it is.
        let b = bytes(self.view.rest(d.virtual_address)?, 0, size as usize).ok_or(Error::Truncated)?;
        let wide = self.info().bits().wide();
        let p = if wide { 8 } else { 4 };
        let u16 = |o| u16_at(b, o).unwrap_or(0);
        let u32 = |o| u32_at(b, o).unwrap_or(0);
        // A pointer-sized field exists only if it fits entirely.
        let ptr = |o| ptr_at(b, o, wide).unwrap_or(0);
        // Offsets from the spec's table: PE32 / PE32+.
        let o = |pe32: usize, pe64: usize| if wide { pe64 } else { pe32 };
        Ok(Some(LoadConfig {
            size,
            time_date_stamp: u32(4),
            major_version: u16(8),
            minor_version: u16(10),
            global_flags_clear: u32(12),
            global_flags_set: u32(16),
            critical_section_default_timeout: u32(20),
            de_commit_free_block_threshold: ptr(24),
            de_commit_total_free_threshold: ptr(24 + p),
            lock_prefix_table: ptr(24 + 2 * p),
            maximum_allocation_size: ptr(24 + 3 * p),
            virtual_memory_threshold: ptr(24 + 4 * p),
            process_affinity_mask: ptr(24 + 5 * p),
            process_heap_flags: u32(o(48, 72)),
            csd_version: u16(o(52, 76)),
            dependent_load_flags: u16(o(54, 78)),
            edit_list: ptr(o(56, 80)),
            security_cookie: ptr(o(60, 88)),
            se_handler_table: ptr(o(64, 96)),
            se_handler_count: ptr(o(68, 104)),
            guard_cf_check_function_pointer: ptr(o(72, 112)),
            guard_cf_dispatch_function_pointer: ptr(o(76, 120)),
            guard_cf_function_table: ptr(o(80, 128)),
            guard_cf_function_count: ptr(o(84, 136)),
            guard_flags: u32(o(88, 144)),
        }))
    }
}
