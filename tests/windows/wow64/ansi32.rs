//! A 32-bit program that uses the ANSI and locale calls the programs of the survey make at start-up
//! (ADR 0186, stage 4): `GetLocaleInfoA`, `GetStringTypeA`/`ExA`/`ExW`, `LCMapStringA`,
//! `CompareStringA`, `GetCPInfoExW`, `EnumCalendarInfoW`, `GetCalendarInfoW`, the `Char*` functions of
//! `user32`, `MulDiv`, `EncodePointer`/`DecodePointer` (a 32-bit rotation), and `VirtualQuery` of the
//! program's own image. One line on standard output per check; the program ends with 0 when all passed,
//! else with `2 + n` for the first check `n` that failed, so the boot test names it.
//!
//! Not part of the x64 test corpus (the oracle pipeline builds `tests/windows/*.rs` for x64 only).

#![no_std]
#![no_main]

extern crate poly_msvc32;

type Handle = isize;

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(file: Handle, buffer: *const u8, length: u32, written: *mut u32, overlapped: *mut u8) -> i32;
    fn ExitProcess(code: u32) -> !;
    fn GetLastError() -> u32;
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn VirtualQuery(address: usize, info: *mut u32, length: usize) -> usize;
    fn GetLocaleInfoA(lcid: u32, kind: u32, buffer: *mut u8, size: i32) -> i32;
    fn GetStringTypeA(lcid: u32, kind: u32, text: *const u8, count: i32, types: *mut u16) -> i32;
    fn GetStringTypeExA(lcid: u32, kind: u32, text: *const u8, count: i32, types: *mut u16) -> i32;
    fn GetStringTypeExW(lcid: u32, kind: u32, text: *const u16, count: i32, types: *mut u16) -> i32;
    fn LCMapStringA(lcid: u32, flags: u32, src: *const u8, count: i32, dest: *mut u8, size: i32) -> i32;
    fn CompareStringA(lcid: u32, flags: u32, a: *const u8, a_len: i32, b: *const u8, b_len: i32) -> i32;
    fn GetCPInfoExW(code_page: u32, flags: u32, info: *mut u8) -> i32;
    fn EnumCalendarInfoW(callback: unsafe extern "system" fn(*const u16) -> i32, lcid: u32, cal: u32, kind: u32) -> i32;
    fn GetCalendarInfoW(lcid: u32, cal: u32, kind: u32, buffer: *mut u16, size: i32, value: *mut u32) -> i32;
    fn InitializeCriticalSection(section: *mut u32);
    fn EnterCriticalSection(section: *mut u32);
    fn LeaveCriticalSection(section: *mut u32);
    fn DeleteCriticalSection(section: *mut u32);
    fn GetCurrentThreadId() -> u32;
    fn VirtualAlloc(address: usize, size: usize, kind: u32, protect: u32) -> usize;
    fn VirtualFree(address: usize, size: usize, kind: u32) -> i32;
    fn MulDiv(number: i32, numerator: i32, denominator: i32) -> i32;
    fn EncodePointer(pointer: usize) -> usize;
    fn DecodePointer(pointer: usize) -> usize;
}

#[link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn CharUpperBuffW(text: *mut u16, count: u32) -> u32;
    fn CharLowerBuffW(text: *mut u16, count: u32) -> u32;
    fn CharUpperBuffA(text: *mut u8, count: u32) -> u32;
    fn CharLowerW(text: *mut u16) -> *mut u16;
    fn CharUpperA(text: *mut u8) -> *mut u8;
    fn CharNextW(text: *const u16) -> *const u16;
    fn IsCharAlphaW(c: u16) -> i32;
    fn IsCharAlphaNumericW(c: u16) -> i32;
}

/// The sub-step a check was at when it failed (printed after the check's name).
static STEP: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn at(step: u32) {
    STEP.store(step, core::sync::atomic::Ordering::Relaxed);
}

const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

fn out(text: &[u8]) -> bool {
    let mut written = 0u32;
    // SAFETY: plain kernel32 calls with valid buffers.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        WriteFile(h, text.as_ptr(), text.len() as u32, &mut written, core::ptr::null_mut()) != 0
            && written == text.len() as u32
    }
}

/// `value` as eight hex digits and a newline, after `label`.
fn out_hex(label: &[u8], value: u32) {
    let mut line = [0u8; 12];
    for (i, digit) in line.iter_mut().take(8).enumerate() {
        let nibble = (value >> (28 - 4 * i)) & 15;
        *digit = if nibble < 10 { b'0' + nibble as u8 } else { b'a' + nibble as u8 - 10 };
    }
    line[8] = 10;
    out(label);
    out(&line[..9]);
}

fn utf16(buffer: &mut [u16], text: &str) -> usize {
    let mut n = 0;
    for u in text.encode_utf16() {
        buffer[n] = u;
        n += 1;
    }
    buffer[n] = 0;
    n
}

/// The units equal the UTF-16 of `text`.
fn units_eq(units: &[u16], text: &str) -> bool {
    let mut i = 0;
    for u in text.encode_utf16() {
        if i >= units.len() || units[i] != u {
            return false;
        }
        i += 1;
    }
    i == units.len()
}

fn locale_a() -> bool {
    let mut buffer = [0u8; 32];
    // SAFETY: documented calls with valid buffers.
    unsafe {
        at(1);
        // LOCALE_SENGLANGUAGE = 0x1001.
        if GetLocaleInfoA(0x409, 0x1001, core::ptr::null_mut(), 0) != 8 {
            return false;
        }
        at(2);
        if GetLocaleInfoA(0x409, 0x1001, buffer.as_mut_ptr(), 32) != 8 || &buffer[..8] != b"English\0" {
            return false;
        }
        at(3);
        // LOCALE_USE_CP_ACP is accepted.
        if GetLocaleInfoA(0x409, 0x1001 | 0x4000_0000, buffer.as_mut_ptr(), 32) != 8 {
            return false;
        }
        at(4);
        // LOCALE_IDEFAULTANSICODEPAGE = 0x1004 as text, then as a number.
        if GetLocaleInfoA(0x409, 0x1004, buffer.as_mut_ptr(), 32) != 5 || &buffer[..5] != b"1252\0" {
            return false;
        }
        at(5);
        let mut number = 0u32;
        if GetLocaleInfoA(0x409, 0x1004 | 0x2000_0000, (&mut number as *mut u32).cast(), 4) != 4 || number != 1252 {
            return false;
        }
        at(6);
        // A buffer too small: 0 and ERROR_INSUFFICIENT_BUFFER (122).
        GetLocaleInfoA(0x409, 0x1001, buffer.as_mut_ptr(), 3) == 0 && GetLastError() == 122
    }
}

fn string_types() -> bool {
    let mut types = [0u16; 8];
    // SAFETY: documented calls with valid buffers.
    unsafe {
        at(1);
        // CT_CTYPE1: C1_UPPER|C1_XDIGIT|C1_ALPHA, C1_LOWER|C1_ALPHA, C1_DIGIT|C1_XDIGIT, C1_SPACE|C1_BLANK.
        if GetStringTypeA(0x409, 1, b"Ag7 ".as_ptr(), 4, types.as_mut_ptr()) == 0
            || types[..4] != [0x1 | 0x80 | 0x100, 0x2 | 0x100, 0x4 | 0x80, 0x8 | 0x40]
        {
            return false;
        }
        at(2);
        types = [0xFFFF; 8];
        if GetStringTypeExA(0x409, 1, b"zz".as_ptr(), 2, types.as_mut_ptr()) == 0 || types[..2] != [0x2 | 0x100; 2] {
            return false;
        }
        at(3);
        // CT_CTYPE2: left to right for a letter, a European number for a digit.
        if GetStringTypeA(0x409, 2, b"a5".as_ptr(), 2, types.as_mut_ptr()) == 0 || types[..2] != [1, 3] {
            return false;
        }
        at(4);
        let mut wide = [0u16; 8];
        utf16(&mut wide, "Q-");
        // CT_CTYPE3: alpha; a hyphen has no type 3 flag.
        if GetStringTypeExW(0x409, 4, wide.as_ptr(), 2, types.as_mut_ptr()) == 0 || types[0] != 0x8000 {
            return false;
        }
        at(5);
        // Length -1 counts the NUL.
        types = [0xFFFF; 8];
        GetStringTypeA(0x409, 1, b"ab\0".as_ptr(), -1, types.as_mut_ptr()) != 0 && types[2] == 0x20
    }
}

fn mapping() -> bool {
    let mut buffer = [0u8; 16];
    // SAFETY: documented calls with valid buffers.
    unsafe {
        at(1);
        // LCMAP_UPPERCASE = 0x200: five letters and the NUL; 0xE9 (e acute) goes to 0xC9, 0xB5 stays.
        if LCMapStringA(0x409, 0x200, b"he\xE9\xB5o\0".as_ptr(), -1, buffer.as_mut_ptr(), 16) != 6
            || &buffer[..6] != b"HE\xC9\xB5O\0"
        {
            return false;
        }
        at(2);
        // LCMAP_LOWERCASE = 0x100, a size query first.
        if LCMapStringA(0x409, 0x100, b"ABC".as_ptr(), 3, core::ptr::null_mut(), 0) != 3 {
            return false;
        }
        at(3);
        // The flags are required (0 is ERROR_INVALID_FLAGS, 1004).
        if LCMapStringA(0x409, 0, b"ABC".as_ptr(), 3, buffer.as_mut_ptr(), 16) != 0 || GetLastError() != 1004 {
            return false;
        }
        at(4);
        let mut wide = [0u16; 16];
        utf16(&mut wide, "Mixed Ca\u{e9}");
        if CharUpperBuffW(wide.as_mut_ptr(), 9) != 9 || !units_eq(&wide[..9], "MIXED CA\u{c9}") {
            return false;
        }
        at(5);
        if CharLowerBuffW(wide.as_mut_ptr(), 3) != 3 || wide[0] != u16::from(b'm') || wide[3] != u16::from(b'E') {
            return false;
        }
        at(6);
        // One character in the low word, and a string in place.
        if CharLowerW(0x41 as *mut u16) as usize != 0x61 || CharUpperA(0x7A as *mut u8) as usize != 0x5A {
            return false;
        }
        at(7);
        let mut ansi = *b"abc\0";
        if CharUpperBuffA(ansi.as_mut_ptr(), 3) != 3 || &ansi != b"ABC\0" {
            return false;
        }
        at(8);
        let text = [0x61u16, 0x62, 0];
        let next = CharNextW(text.as_ptr());
        let end = CharNextW(text.as_ptr().add(2));
        at(9);
        next == text.as_ptr().add(1) && end == text.as_ptr().add(2) && IsCharAlphaW(0x61) != 0 && IsCharAlphaW(0x31) == 0
            && IsCharAlphaNumericW(0x31) != 0
    }
}

fn compare() -> bool {
    // SAFETY: documented calls on valid strings.
    unsafe {
        at(1);
        // CSTR_EQUAL = 2, CSTR_LESS_THAN = 1, CSTR_GREATER_THAN = 3; NORM_IGNORECASE = 1.
        if CompareStringA(0x409, 1, b"abc".as_ptr(), 3, b"ABC".as_ptr(), 3) != 2 {
            return false;
        }
        at(2);
        if CompareStringA(0x409, 0, b"abc".as_ptr(), -1, b"abd".as_ptr(), -1) != 1 {
            return false;
        }
        at(3);
        CompareStringA(0x409, 0, b"b".as_ptr(), 1, b"a".as_ptr(), 1) == 3
    }
}

fn code_page() -> bool {
    let mut info = [0u8; 544];
    // SAFETY: documented call with a CPINFOEXW buffer.
    unsafe {
        at(1);
        if GetCPInfoExW(0, 0, info.as_mut_ptr()) == 0 {
            return false;
        }
        at(2);
        let number = u32::from_le_bytes([info[20], info[21], info[22], info[23]]);
        // MaxCharSize 1, CodePage 1252, a name that starts with the number.
        info[0] == 1 && number == 1252 && info[24] == b'1' && info[25] == 0 && info[26] == b'2'
    }
}

static MONTH: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

unsafe extern "system" fn collect_month(text: *const u16) -> i32 {
    // The first letter and the length of the string.
    let mut n = 0u32;
    // SAFETY: the system's NUL-terminated string.
    while unsafe { text.add(n as usize).read() } != 0 {
        n += 1;
    }
    // SAFETY: as above.
    MONTH.store(n << 16 | u32::from(unsafe { text.read() }), core::sync::atomic::Ordering::Relaxed);
    1
}

fn calendar() -> bool {
    let mut buffer = [0u16; 32];
    let mut number = 0u32;
    // SAFETY: documented calls.
    unsafe {
        at(1);
        // CAL_GREGORIAN = 1; CAL_SMONTHNAME3 = 0x15 + 2 is "March".
        if EnumCalendarInfoW(collect_month, 0x409, 1, 0x15 + 2) == 0 {
            return false;
        }
        at(2);
        if MONTH.load(core::sync::atomic::Ordering::Relaxed) != 5 << 16 | u32::from(b'M') {
            return false;
        }
        at(3);
        // ENUM_ALL_CALENDARS; CAL_SERASTRING = 4 is "A.D.".
        if EnumCalendarInfoW(collect_month, 0x409, 0xFFFF_FFFF, 4) == 0
            || MONTH.load(core::sync::atomic::Ordering::Relaxed) != 4 << 16 | u32::from(b'A')
        {
            return false;
        }
        at(4);
        // A calendar that is not Gregorian: ERROR_INVALID_PARAMETER (87).
        if EnumCalendarInfoW(collect_month, 0x409, 3, 4) != 0 || GetLastError() != 87 {
            return false;
        }
        at(5);
        // CAL_SDAYNAME1 = 7 is "Monday" (the first day of the header's week); size 7 with the NUL.
        if GetCalendarInfoW(0x409, 1, 7, buffer.as_mut_ptr(), 32, core::ptr::null_mut()) != 7 || buffer[0] != u16::from(b'M')
        {
            return false;
        }
        at(6);
        // CAL_ICALINTVALUE = 1 | CAL_RETURN_NUMBER = 0x20000000.
        GetCalendarInfoW(0x409, 1, 1 | 0x2000_0000, core::ptr::null_mut(), 0, &mut number) != 0 && number == 1
    }
}

fn pointers() -> bool {
    // SAFETY: documented calls.
    unsafe {
        at(1);
        // MulDiv rounds half away from zero.
        if MulDiv(1, 3, 2) != 2 || MulDiv(-1, 3, 2) != -2 || MulDiv(100, 96, 72) != 133 || MulDiv(1, 1, 0) != -1 {
            return false;
        }
        at(2);
        for p in [0usize, 1, 0x40_0000, 0x7FFE_0000, 0xDEAD_BEEF, usize::MAX] {
            if DecodePointer(EncodePointer(p)) != p {
                return false;
            }
        }
        at(3);
        EncodePointer(0x40_1000) != 0x40_1000 && EncodePointer(0) != 0
    }
}

/// `RTL_CRITICAL_SECTION` in a 32-bit process is 24 bytes (`winnt.h`: `DebugInfo`, `LockCount`,
/// `RecursionCount`, `OwningThread`, `LockSemaphore`, `SpinCount`, each pointer 4 bytes): a Delphi
/// object embeds one, and what follows it is the object's next field.
fn critical_section() -> bool {
    // Six words of section and two canaries after it.
    let mut words = [0xC0DE_C0DEu32; 8];
    let section = words.as_mut_ptr();
    // SAFETY: documented calls on a 24-byte section inside a larger buffer.
    unsafe {
        at(1);
        InitializeCriticalSection(section);
        if words[6] != 0xC0DE_C0DE || words[7] != 0xC0DE_C0DE {
            return false;
        }
        at(2);
        // LockCount -1 at 4, RecursionCount 0 at 8, OwningThread 0 at 12.
        if words[1] != u32::MAX || words[2] != 0 || words[3] != 0 {
            return false;
        }
        at(3);
        EnterCriticalSection(section);
        EnterCriticalSection(section);
        if words[1] != (-3i32) as u32 || words[2] != 2 || words[3] != GetCurrentThreadId() {
            return false;
        }
        at(4);
        LeaveCriticalSection(section);
        LeaveCriticalSection(section);
        if words[1] != u32::MAX || words[2] != 0 || words[3] != 0 {
            return false;
        }
        DeleteCriticalSection(section);
        at(5);
        words[6] == 0xC0DE_C0DE && words[7] == 0xC0DE_C0DE
    }
}

/// A memory manager reserves a region, commits it page by page, and may commit pages it has
/// committed already (an overlapping range): the contents stay (Learn, `VirtualAlloc`: "an attempt
/// to commit a page that is already committed does not cause the function to fail").
fn recommit() -> bool {
    // SAFETY: documented calls on a region this check reserves.
    unsafe {
        // MEM_RESERVE = 0x2000, MEM_COMMIT = 0x1000, PAGE_READWRITE = 4, MEM_RELEASE = 0x8000.
        at(1);
        let region = VirtualAlloc(0, 0x10_0000, 0x2000, 4);
        if region == 0 {
            return false;
        }
        at(2);
        if VirtualAlloc(region, 0x2000, 0x1000, 4) != region {
            return false;
        }
        (region as *mut u32).write(0xA5A5_1234);
        ((region + 0x1000) as *mut u32).write(0x5A5A_4321);
        at(3);
        // The second commit overlaps the first two pages and adds a third.
        if VirtualAlloc(region + 0x1000, 0x2000, 0x1000, 4) != region + 0x1000 {
            return false;
        }
        at(4);
        let kept = (region as *const u32).read() == 0xA5A5_1234 && ((region + 0x1000) as *const u32).read() == 0x5A5A_4321;
        at(5);
        let zero = ((region + 0x2000) as *const u32).read() == 0;
        // The same pages again, whole.
        at(6);
        let same = VirtualAlloc(region, 0x3000, 0x1000, 4) == region
            && (region as *const u32).read() == 0xA5A5_1234
            && ((region + 0x1000) as *const u32).read() == 0x5A5A_4321;
        VirtualFree(region, 0, 0x8000);
        kept && zero && same
    }
}

/// The part of every section's last page that the file does not hold reads as zeros, also when the
/// file goes on after the image (a signature, an installer's payload): the pages are the section's,
/// not the file's.
fn section_tails() -> bool {
    // SAFETY: reads the program's own headers and mapped sections.
    unsafe {
        let base = GetModuleHandleW(core::ptr::null());
        let u32_at = |at: usize| ((base + at) as *const u32).read_unaligned();
        let pe = u32_at(0x3c) as usize;
        let sections = u32::from(((base + pe + 6) as *const u16).read_unaligned()) as usize;
        let optional = u32::from(((base + pe + 20) as *const u16).read_unaligned()) as usize;
        let table = pe + 24 + optional;
        at(1);
        for i in 0..sections {
            let entry = table + 40 * i;
            let (virtual_size, address, raw_size) = (u32_at(entry + 8) as usize, u32_at(entry + 12) as usize, u32_at(entry + 16) as usize);
            let used = raw_size.min(virtual_size);
            let end = (address + virtual_size + 0xfff) & !0xfff;
            at(2 + i as u32);
            // A writable section's bytes past the file's are its own variables (`.data` without raw data).
            let writable = u32_at(entry + 36) & 0x8000_0000 != 0;
            let from = if writable { address + virtual_size } else { address + used };
            for offset in from..end {
                if ((base + offset) as *const u8).read() != 0 {
                    return false;
                }
            }
        }
        true
    }
}

/// `MEMORY_BASIC_INFORMATION` of the program's own image: the allocation base is the module, the
/// header region is committed, read-only image memory, and each region ends inside the image.
fn image_regions() -> bool {
    // SAFETY: documented calls with a 28-byte buffer.
    unsafe {
        let base = GetModuleHandleW(core::ptr::null());
        let mut info = [0u32; 7];
        at(1);
        if VirtualQuery(base, info.as_mut_ptr(), 28) != 28 {
            return false;
        }
        out_hex(b"  base ", info[0]);
        out_hex(b"  allocation base ", info[1]);
        out_hex(b"  size ", info[3]);
        out_hex(b"  state ", info[4]);
        out_hex(b"  protect ", info[5]);
        out_hex(b"  type ", info[6]);
        at(2);
        // MEM_COMMIT = 0x1000, MEM_IMAGE = 0x1000000.
        info[0] as usize == base && info[1] as usize == base && info[4] == 0x1000 && info[6] == 0x100_0000
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> u32 {
    let checks: [(&[u8], fn() -> bool); 11] = [
        (b"locale\n", locale_a),
        (b"string types\n", string_types),
        (b"mapping\n", mapping),
        (b"compare\n", compare),
        (b"code page\n", code_page),
        (b"calendar\n", calendar),
        (b"pointers\n", pointers),
        (b"image regions\n", image_regions),
        (b"section tails\n", section_tails),
        (b"recommit\n", recommit),
        (b"critical section\n", critical_section),
    ];
    if !out(b"ansi32\n") {
        // SAFETY: ends the process.
        unsafe { ExitProcess(1) }
    }
    for (i, (line, check)) in checks.iter().enumerate() {
        if !check() {
            out(b"failed: ");
            out(line);
            let step = STEP.load(core::sync::atomic::Ordering::Relaxed);
            out(&[b's', b't', b'e', b'p', b' ', b'0' + (step / 10 % 10) as u8, b'0' + (step % 10) as u8, 10]);
            // SAFETY: ends the process.
            unsafe { ExitProcess(2 + i as u32) }
        }
        out(line);
    }
    // SAFETY: ends the process.
    unsafe { ExitProcess(0) }
}

/// MSVC-style code references this once floating point is used.
#[unsafe(no_mangle)]
#[used]
static _fltused: i32 = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: ends the process.
    unsafe { ExitProcess(0xDEAD) }
}
