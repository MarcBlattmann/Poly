//! NtAllocateVirtualMemory / NtFreeVirtualMemory round trip in the
//! current process (Learn: "NtAllocateVirtualMemory",
//! "NtFreeVirtualMemory"): a committed read-write region chosen by the
//! system starts on an allocation-granularity boundary (64 KiB,
//! `SYSTEM_INFO.dwAllocationGranularity`), its size is rounded up to whole
//! pages, it reads back what was written and starts zeroed; releasing it
//! with a size of 0 releases the whole region and reports its size; a
//! second release is refused (the status is an observation).
#![no_std]
#![no_main]

use core::ffi::c_void;
use poly_wtest::*;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtAllocateVirtualMemory(
        process: HANDLE,
        base_address: *mut *mut c_void,
        zero_bits: usize,
        region_size: *mut usize,
        allocation_type: u32,
        protect: u32,
    ) -> NTSTATUS;
    fn NtFreeVirtualMemory(
        process: HANDLE,
        base_address: *mut *mut c_void,
        region_size: *mut usize,
        free_type: u32,
    ) -> NTSTATUS;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut base: *mut c_void = core::ptr::null_mut();
    let mut size: usize = 0x1001;
    // SAFETY: `base` and `size` are live locals the call writes.
    let status = unsafe {
        NtAllocateVirtualMemory(NT_CURRENT_PROCESS, &mut base, 0, &mut size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE)
    };
    if !t.expect_status("allocate", status, STATUS_SUCCESS) {
        return;
    }
    t.check("base_not_null", !base.is_null(), 0);
    t.expect("base_on_64k", (base as usize % 0x10000) as i64, 0);
    t.expect("size_rounded_to_pages", size as i64, 0x2000);

    let bytes = base.cast::<u8>();
    // SAFETY: the region is committed read-write for 0x2000 bytes.
    let (zero_first, zero_last) = unsafe { (bytes.read_volatile(), bytes.add(0x1fff).read_volatile()) };
    t.check("starts_zeroed", zero_first == 0 && zero_last == 0, i64::from(zero_first) | i64::from(zero_last));
    // SAFETY: as above; every offset is inside the region.
    let sum = unsafe {
        for i in 0..0x2000usize {
            bytes.add(i).write_volatile((i % 251) as u8);
        }
        (0..0x2000usize).map(|i| u64::from(bytes.add(i).read_volatile())).sum::<u64>()
    };
    let want: u64 = (0..0x2000u64).map(|i| i % 251).sum();
    t.expect("reads_back", sum as i64, want as i64);

    let mut free_base = base;
    let mut free_size: usize = 0;
    // SAFETY: releasing the region allocated above; locals are written.
    let status = unsafe { NtFreeVirtualMemory(NT_CURRENT_PROCESS, &mut free_base, &mut free_size, MEM_RELEASE) };
    t.expect_status("release", status, STATUS_SUCCESS);
    t.check("release_reports_base", free_base == base, 0);
    t.expect("release_reports_size", free_size as i64, 0x2000);

    let mut again_base = base;
    let mut again_size: usize = 0;
    // SAFETY: as above; the region is gone, so the call only reports.
    let status = unsafe { NtFreeVirtualMemory(NT_CURRENT_PROCESS, &mut again_base, &mut again_size, MEM_RELEASE) };
    t.check("second_release_fails", status < 0, 0);
    t.value_hex("second_release_status", u64::from(status as u32));
}
