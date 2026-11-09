//! A diagnostic for `cargo xtask chrome-win run --stage 9` (ADR 0155): calls
//! what Chrome's start-up asks of `version.dll` on Chrome's own files (read
//! from the public Learn pages of `GetFileVersionInfoSizeW`,
//! `GetFileVersionInfoW`, `VerQueryValueW`) and prints every result, so a
//! difference from what Chrome expects shows without a debugger.
#![no_std]
#![no_main]

use core::ffi::c_void;
use poly_wtest::*;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn GetStdHandle(which: u32) -> *mut c_void;
    fn WriteFile(file: *mut c_void, buffer: *const u8, to_write: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
}

#[link(name = "version", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetFileVersionInfoSizeW(name: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(name: *const u16, handle: u32, len: u32, data: *mut u8) -> i32;
    fn VerQueryValueW(block: *const u8, sub: *const u16, buffer: *mut *mut u8, len: *mut u32) -> i32;
}

const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

fn say(bytes: &[u8]) {
    // SAFETY: plain calls with a buffer of ours.
    unsafe {
        let mut n = 0;
        WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), bytes.as_ptr(), bytes.len() as u32, &mut n, core::ptr::null_mut());
    }
}

fn num(prefix: &str, v: u64) {
    let mut d = [0u8; 16];
    for i in 0..16 {
        d[15 - i] = b"0123456789abcdef"[((v >> (i * 4)) & 15) as usize];
    }
    say(prefix.as_bytes());
    say(b"0x");
    say(&d);
    say(b"\n");
}

static mut BLOCK: [u8; 32768] = [0; 32768];

/// ASCII to NUL-terminated UTF-16 in `out`.
fn wide<'a>(s: &str, out: &'a mut [u16; 200]) -> &'a [u16; 200] {
    for (i, b) in s.bytes().enumerate() {
        out[i] = u16::from(b);
    }
    out
}

unsafe fn probe(path: &str) {
    say(b"PROBE file ");
    say(path.as_bytes());
    say(b"\n");
    let mut p = [0u16; 200];
    let p = wide(path, &mut p);
    let mut handle = 7u32;
    // SAFETY: the single thread owns BLOCK; valid strings and buffers.
    unsafe {
        let size = GetFileVersionInfoSizeW(p.as_ptr(), &mut handle);
        num("  size=", u64::from(size));
        num("  handle=", u64::from(handle));
        num("  err=", u64::from(GetLastError()));
        if size == 0 || size as usize > 32768 {
            return;
        }
        let block = core::ptr::addr_of_mut!(BLOCK).cast::<u8>();
        let ok = GetFileVersionInfoW(p.as_ptr(), 0, size, block);
        num("  GetFileVersionInfoW=", ok as u64);
        num("  err=", u64::from(GetLastError()));
        num("  block[0..2]=", u64::from(u16::from_le_bytes([*block, *block.add(1)])));
        for sub in [
            "\\",
            "\\VarFileInfo\\Translation",
            "\\StringFileInfo\\040904b0\\FileVersion",
            "\\StringFileInfo\\040904b0\\ProductVersion",
            "\\StringFileInfo\\040904b0\\CompanyName",
            "\\StringFileInfo\\040904b0\\Nope",
        ] {
            let mut s = [0u16; 200];
            let s = wide(sub, &mut s);
            let mut buf: *mut u8 = 1 as *mut u8;
            let mut len = 99u32;
            let r = VerQueryValueW(block, s.as_ptr(), &mut buf, &mut len);
            say(b"  query ");
            say(sub.as_bytes());
            say(b"\n");
            num("    result=", r as u64);
            num("    err=", u64::from(GetLastError()));
            num("    ptr-offset=", (buf as u64).wrapping_sub(block as u64));
            num("    len=", u64::from(len));
            if r != 0 && sub == "\\" {
                num("    signature=", u64::from(u32::from_le_bytes([*buf, *buf.add(1), *buf.add(2), *buf.add(3)])));
                num(
                    "    productVersionMS=",
                    u64::from(u32::from_le_bytes([*buf.add(16), *buf.add(17), *buf.add(18), *buf.add(19)])),
                );
            }
        }
    }
}

/// The entry point. Not `poly_wtest::main!`: that creates `poly-test.out`
/// beside the image, which the launcher (same directory) holds open.
#[unsafe(no_mangle)]
pub extern "system" fn mainCRTStartup() -> ! {
    // SAFETY: plain calls.
    unsafe {
        probe("C:\\Program Files\\Chrome\\chrome.exe");
        probe("C:\\Program Files\\Chrome\\chrome_elf.dll");
    }
    poly_wtest::exit(0)
}
