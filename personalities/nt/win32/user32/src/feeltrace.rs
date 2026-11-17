//! TEMP-FEEL: a temporary trace of how popups are made (removed before commit).

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyDebugPrint(text: *const u8, length: usize) -> u32;
}

pub fn log(text: &str) {
    // SAFETY: the service reads `text.len()` bytes.
    unsafe { PolyDebugPrint(text.as_ptr(), text.len()) };
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn QueryPerformanceCounter(out: *mut i64) -> i32;
}

pub fn us() -> u64 {
    let mut t = 0i64;
    // SAFETY: our i64.
    unsafe { QueryPerformanceCounter(&mut t) };
    t as u64 / 10
}

pub fn msg(how: &str, m: u32, w: u64) {
    if matches!(m, 0x100 | 0x102 | 0x201) {
        log(&alloc::format!("feel-t: {how} msg={m:#x} w={w:#x} us={}", us()));
    }
}
