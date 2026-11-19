//! A 32-bit program that uses the other system DLLs (ADR 0186, stage 3): Poly's i686 `advapi32`,
//! `ole32`, `oleaut32`, `shell32`, `shlwapi`, `version`, `winmm`, `ws2_32`, `comctl32`, `bcrypt`
//! and `secur32`, built from the same sources as the x64 ones and installed in `SysWOW64`. One
//! line on standard output per check; the program ends with 0 when all passed, else with `2 + n`
//! for the first check `n` that failed, so the boot test names it.
//!
//! - `registry`: `RegCreateKeyExW`, `RegSetValueExW` (a string and a DWORD), `RegQueryValueExW`,
//!   `RegDeleteValueW`, `RegDeleteKeyW` in `HKCU` (the 32-bit `HKEY` handles are sign-extended);
//! - `token`: `OpenProcessToken` + `GetTokenInformation(TokenUser)` (a `TOKEN_USER` with 4-byte
//!   pointers), `ConvertSidToStringSidW`, `GetUserNameW`;
//! - `com`: `CoInitializeEx`, `CoCreateGuid`, `StringFromGUID2`, `IIDFromString`,
//!   `CoTaskMemAlloc`/`CoTaskMemFree`;
//! - `oleaut`: `SysAllocString`, `SysStringLen`, a `VARIANT` (16 bytes), a `SAFEARRAY` vector
//!   (the 24-byte 32-bit header with a 4-byte data pointer);
//! - `shell`: `CommandLineToArgvW`, `SHGetFolderPathW`; `shlwapi`: `PathFindFileNameW`,
//!   `PathFindExtensionW`;
//! - `version`: the version resource of `C:\Windows\SysWOW64\kernel32.dll` through
//!   `GetFileVersionInfoW` and `VerQueryValueW` (`VS_FIXEDFILEINFO`);
//! - `winmm`: `timeGetTime`; `comctl32`: `InitCommonControlsEx`;
//! - `winsock`: `WSAStartup` (the 400-byte 32-bit `WSADATA`), a TCP connection over the loopback
//!   interface (`bind`, `getsockname`, `listen`, `connect`, `accept`, `send`, `recv`);
//! - `bcrypt`: `BCryptGenRandom` and SHA-256 of `abc` through `BCryptHash`;
//! - `secur32`: `GetUserNameExW`;
//! - `updater`: what Google's 32-bit `updater.exe` and `ChromeSetup.exe` import that Poly's 32-bit
//!   set lacked: `QueryFullProcessImageNameW` (the Win32 form equals `GetModuleFileNameW(NULL)`,
//!   the device form starts with `\Device\`, a buffer without room for the NUL is
//!   `ERROR_INSUFFICIENT_BUFFER`) and `msimg32.dll`'s `AlphaBlend` (bound, and a call with no
//!   device contexts returns FALSE with the stack as it was: eleven 4-byte arguments), and gdi32's
//!   `FrameRgn`, `RoundRect` and `OffsetWindowOrgEx` on a memory device context.
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
    fn GetCurrentProcess() -> Handle;
    fn LocalFree(memory: *mut u8) -> *mut u8;
    fn ExitProcess(code: u32) -> !;
    fn GetModuleFileNameW(module: Handle, buffer: *mut u16, size: u32) -> u32;
    fn QueryFullProcessImageNameW(process: Handle, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    fn GetLastError() -> u32;
}

#[link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc: Handle) -> Handle;
    fn DeleteDC(dc: Handle) -> i32;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> Handle;
    fn DeleteObject(object: Handle) -> i32;
    fn GetStockObject(index: i32) -> Handle;
    fn FrameRgn(dc: Handle, rgn: Handle, brush: Handle, w: i32, h: i32) -> i32;
    fn RoundRect(dc: Handle, l: i32, t: i32, r: i32, b: i32, w: i32, h: i32) -> i32;
    fn OffsetWindowOrgEx(dc: Handle, x: i32, y: i32, old: *mut i32) -> i32;
}

#[link(name = "msimg32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn AlphaBlend(
        dest: Handle,
        x_dest: i32,
        y_dest: i32,
        w_dest: i32,
        h_dest: i32,
        src: Handle,
        x_src: i32,
        y_src: i32,
        w_src: i32,
        h_src: i32,
        blend: u32,
    ) -> i32;
}

#[link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn RegCreateKeyExW(
        key: isize,
        sub_key: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        access: u32,
        security: *const u8,
        result: *mut isize,
        disposition: *mut u32,
    ) -> i32;
    fn RegSetValueExW(key: isize, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
    fn RegQueryValueExW(
        key: isize,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegDeleteValueW(key: isize, name: *const u16) -> i32;
    fn RegDeleteKeyW(key: isize, sub_key: *const u16) -> i32;
    fn RegCloseKey(key: isize) -> i32;
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(token: Handle, class: u32, info: *mut u8, length: u32, needed: *mut u32) -> i32;
    fn ConvertSidToStringSidW(sid: *const u8, string: *mut *mut u16) -> i32;
    fn GetUserNameW(buffer: *mut u16, size: *mut u32) -> i32;
}

#[link(name = "ole32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *const u8, model: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateGuid(guid: *mut u8) -> i32;
    fn StringFromGUID2(guid: *const u8, buffer: *mut u16, max: i32) -> i32;
    fn IIDFromString(string: *const u16, iid: *mut u8) -> i32;
    fn CoTaskMemAlloc(size: usize) -> *mut u8;
    fn CoTaskMemFree(memory: *mut u8);
}

#[link(name = "oleaut32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn SysAllocString(text: *const u16) -> *mut u16;
    fn SysStringLen(text: *const u16) -> u32;
    fn SysFreeString(text: *mut u16);
    fn VariantInit(variant: *mut u8);
    fn VariantClear(variant: *mut u8) -> i32;
    fn SafeArrayCreateVector(kind: u16, lower: i32, count: u32) -> *mut u8;
    fn SafeArrayPutElement(array: *mut u8, index: *const i32, value: *const u8) -> i32;
    fn SafeArrayGetElement(array: *mut u8, index: *const i32, value: *mut u8) -> i32;
    fn SafeArrayAccessData(array: *mut u8, data: *mut *mut u8) -> i32;
    fn SafeArrayUnaccessData(array: *mut u8) -> i32;
    fn SafeArrayDestroy(array: *mut u8) -> i32;
}

#[link(name = "shell32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn CommandLineToArgvW(command_line: *const u16, count: *mut i32) -> *mut *mut u16;
    fn SHGetFolderPathW(window: isize, folder: i32, token: Handle, flags: u32, path: *mut u16) -> i32;
}

#[link(name = "shlwapi", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn PathFindFileNameW(path: *const u16) -> *const u16;
    fn PathFindExtensionW(path: *const u16) -> *const u16;
}

#[link(name = "version", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn GetFileVersionInfoSizeW(name: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(name: *const u16, handle: u32, length: u32, data: *mut u8) -> i32;
    fn VerQueryValueW(block: *const u8, sub_block: *const u16, buffer: *mut *mut u8, length: *mut u32) -> i32;
}

#[link(name = "winmm", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn timeGetTime() -> u32;
}

#[link(name = "comctl32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn InitCommonControlsEx(init: *const u32) -> i32;
}

#[link(name = "ws2_32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn WSACleanup() -> i32;
    fn socket(family: i32, kind: i32, protocol: i32) -> usize;
    fn bind(socket: usize, name: *const u8, length: i32) -> i32;
    fn getsockname(socket: usize, name: *mut u8, length: *mut i32) -> i32;
    fn listen(socket: usize, backlog: i32) -> i32;
    fn connect(socket: usize, name: *const u8, length: i32) -> i32;
    fn accept(socket: usize, name: *mut u8, length: *mut i32) -> usize;
    fn send(socket: usize, buffer: *const u8, length: i32, flags: i32) -> i32;
    fn recv(socket: usize, buffer: *mut u8, length: i32, flags: i32) -> i32;
    fn closesocket(socket: usize) -> i32;
}

#[link(name = "bcrypt", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: Handle, buffer: *mut u8, length: u32, flags: u32) -> i32;
    fn BCryptOpenAlgorithmProvider(handle: *mut Handle, id: *const u16, implementation: *const u16, flags: u32) -> i32;
    fn BCryptHash(
        algorithm: Handle,
        secret: *const u8,
        secret_length: u32,
        input: *const u8,
        input_length: u32,
        output: *mut u8,
        output_length: u32,
    ) -> i32;
    fn BCryptCloseAlgorithmProvider(handle: Handle, flags: u32) -> i32;
}

#[link(name = "secur32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn GetUserNameExW(format: i32, buffer: *mut u16, size: *mut u32) -> u8;
}

/// The sub-step a check was at when it failed (printed after the check's name).
static STEP: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn at(step: u32) {
    STEP.store(step, core::sync::atomic::Ordering::Relaxed);
}

const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;

fn out(text: &[u8]) -> bool {
    let mut written = 0u32;
    // SAFETY: plain kernel32 calls with valid buffers.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        WriteFile(h, text.as_ptr(), text.len() as u32, &mut written, core::ptr::null_mut()) != 0
            && written == text.len() as u32
    }
}

/// `text` as UTF-16 with a terminating NUL in `buffer`; the number of units written (no NUL).
fn wide(buffer: &mut [u16], text: &str) -> usize {
    let mut n = 0;
    for u in text.encode_utf16() {
        buffer[n] = u;
        n += 1;
    }
    buffer[n] = 0;
    n
}

/// The UTF-16 string at `p` (NUL-terminated) equals `text`.
fn wide_eq(p: *const u16, text: &str) -> bool {
    if p.is_null() {
        return false;
    }
    let mut i = 0;
    for u in text.encode_utf16() {
        // SAFETY: the caller's string is at least as long as `text` or ends earlier (NUL).
        if unsafe { p.add(i).read() } != u {
            return false;
        }
        i += 1;
    }
    // SAFETY: as above, the terminator is the next unit.
    unsafe { p.add(i).read() == 0 }
}

fn registry() -> bool {
    let mut path = [0u16; 40];
    wide(&mut path, "Software\\PolyWow64Sys32");
    let mut name = [0u16; 16];
    wide(&mut name, "Name");
    let mut count = [0u16; 16];
    wide(&mut count, "Count");
    let mut text = [0u16; 16];
    let text_len = wide(&mut text, "poly 32");
    let mut key: isize = 0;
    let mut disposition = 0u32;
    // SAFETY: documented advapi32 calls with valid buffers.
    unsafe {
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            core::ptr::null(),
            0,
            0xF003F,
            core::ptr::null(),
            &mut key,
            &mut disposition,
        ) != 0
            || key == 0
        {
            return false;
        }
        let bytes = ((text_len + 1) * 2) as u32;
        let mut ok = RegSetValueExW(key, name.as_ptr(), 0, 1, text.as_ptr().cast(), bytes) == 0
            && RegSetValueExW(key, count.as_ptr(), 0, 4, (&7u32 as *const u32).cast(), 4) == 0;
        let mut kind = 0u32;
        let mut back = [0u16; 16];
        let mut size = 32u32;
        ok = ok
            && RegQueryValueExW(key, name.as_ptr(), core::ptr::null_mut(), &mut kind, back.as_mut_ptr().cast(), &mut size)
                == 0
            && kind == 1
            && size == bytes
            && wide_eq(back.as_ptr(), "poly 32");
        let mut number = 0u32;
        size = 4;
        ok = ok
            && RegQueryValueExW(key, count.as_ptr(), core::ptr::null_mut(), &mut kind, (&mut number as *mut u32).cast(), &mut size)
                == 0
            && kind == 4
            && number == 7;
        ok = ok && RegDeleteValueW(key, name.as_ptr()) == 0;
        RegCloseKey(key);
        ok && RegDeleteKeyW(HKEY_CURRENT_USER, path.as_ptr()) == 0
    }
}

fn token() -> bool {
    let mut token: Handle = 0;
    let mut info = [0u8; 256];
    let mut needed = 0u32;
    // SAFETY: documented advapi32 calls with valid buffers.
    unsafe {
        // TOKEN_QUERY = 8.
        at(1);
        if OpenProcessToken(GetCurrentProcess(), 8, &mut token) == 0 {
            return false;
        }
        at(2);
        // TokenUser = 1: a TOKEN_USER is {SID pointer, attributes}, 8 bytes in 32 bits.
        if GetTokenInformation(token, 1, info.as_mut_ptr(), info.len() as u32, &mut needed) == 0 || needed < 8 {
            return false;
        }
        at(3);
        let sid = u32::from_le_bytes([info[0], info[1], info[2], info[3]]) as usize;
        let base = info.as_ptr() as usize;
        // The SID follows the structure inside the caller's buffer.
        if sid < base + 8 || sid >= base + info.len() {
            return false;
        }
        at(4);
        let mut string: *mut u16 = core::ptr::null_mut();
        if ConvertSidToStringSidW(sid as *const u8, &mut string) == 0 || string.is_null() {
            return false;
        }
        at(5);
        let ok = string.read() == b'S' as u16 && string.add(1).read() == b'-' as u16;
        LocalFree(string.cast());
        at(6);
        let mut user = [0u16; 64];
        let mut n = 64u32;
        ok && GetUserNameW(user.as_mut_ptr(), &mut n) != 0 && n > 1 && user[0] != 0
    }
}

fn com() -> bool {
    let mut guid = [0u8; 16];
    let mut again = [0u8; 16];
    let mut text = [0u16; 40];
    // SAFETY: documented ole32 calls with valid buffers.
    unsafe {
        // COINIT_MULTITHREADED = 0; S_OK or S_FALSE.
        let init = CoInitializeEx(core::ptr::null(), 0);
        if init < 0 {
            return false;
        }
        let ok = CoCreateGuid(guid.as_mut_ptr()) == 0
            && guid != [0u8; 16]
            && StringFromGUID2(guid.as_ptr(), text.as_mut_ptr(), 40) == 39
            && text[0] == b'{' as u16
            && IIDFromString(text.as_ptr(), again.as_mut_ptr()) == 0
            && again == guid;
        let memory = CoTaskMemAlloc(24);
        let ok = ok && !memory.is_null();
        if !memory.is_null() {
            memory.write_bytes(0x5A, 24);
            CoTaskMemFree(memory);
        }
        CoUninitialize();
        ok
    }
}

fn oleaut() -> bool {
    let mut source = [0u16; 8];
    wide(&mut source, "hello");
    // SAFETY: documented oleaut32 calls with valid buffers.
    unsafe {
        at(10);
        let bstr = SysAllocString(source.as_ptr());
        if bstr.is_null() || SysStringLen(bstr) != 5 || !wide_eq(bstr, "hello") {
            return false;
        }
        // The length prefix sits 4 bytes before the characters (`BSTR`).
        let prefix = bstr.cast::<u8>().sub(4).cast::<u32>().read_unaligned();
        SysFreeString(bstr);
        at(11);
        if prefix != 10 {
            return false;
        }
        at(12);
        // A VARIANT is 16 bytes: VT_EMPTY after VariantInit; VariantClear keeps it so.
        let mut variant = [0xFFu8; 16];
        VariantInit(variant.as_mut_ptr());
        if variant[0] != 0 || variant[1] != 0 || VariantClear(variant.as_mut_ptr()) != 0 {
            return false;
        }
        // A SAFEARRAY vector of 4 VT_I4 (3): cDims 1, cbElements 4, pvData at offset 12, the
        // bounds at offset 16 (32-bit layout, 24 bytes).
        at(13);
        let array = SafeArrayCreateVector(3, 0, 4);
        if array.is_null() {
            return false;
        }
        at(14);
        let dims = array.cast::<u16>().read();
        let element = array.add(4).cast::<u32>().read();
        let bound_count = array.add(16).cast::<u32>().read();
        let value = 0x1234_5678u32;
        let index = 2i32;
        let mut got = 0u32;
        let mut data: *mut u8 = core::ptr::null_mut();
        at(15 + (dims != 1) as u32 + 2 * (element != 4) as u32 + 4 * (bound_count != 4) as u32);
        let mut ok = dims == 1
            && element == 4
            && bound_count == 4
            && SafeArrayPutElement(array, &index, (&value as *const u32).cast()) == 0
            && SafeArrayGetElement(array, &index, (&mut got as *mut u32).cast()) == 0
            && got == value
            && SafeArrayAccessData(array, &mut data) == 0
            && !data.is_null();
        if ok {
            ok = data.cast::<u32>().add(2).read() == value;
            SafeArrayUnaccessData(array);
        }
        SafeArrayDestroy(array) == 0 && ok
    }
}

fn shell() -> bool {
    let mut line = [0u16; 24];
    wide(&mut line, "a \"b c\" d");
    let mut count = 0i32;
    let mut folder = [0u16; 260];
    // SAFETY: documented shell32 calls with valid buffers.
    unsafe {
        let argv = CommandLineToArgvW(line.as_ptr(), &mut count);
        if argv.is_null() || count != 3 {
            return false;
        }
        // argv is an array of 4-byte pointers followed by the strings.
        let first = argv.read();
        let second = argv.add(1).read();
        let third = argv.add(2).read();
        let ok = wide_eq(first, "a") && wide_eq(second, "b c") && wide_eq(third, "d");
        LocalFree(argv.cast());
        // CSIDL_WINDOWS = 0x24.
        ok && SHGetFolderPathW(0, 0x24, 0, 0, folder.as_mut_ptr()) == 0 && wide_eq(folder.as_ptr(), "C:\\Windows")
    }
}

fn shlwapi() -> bool {
    let mut path = [0u16; 32];
    wide(&mut path, "C:\\dir\\file.txt");
    // SAFETY: documented shlwapi calls on a NUL-terminated string.
    unsafe {
        let name = PathFindFileNameW(path.as_ptr());
        let extension = PathFindExtensionW(path.as_ptr());
        wide_eq(name, "file.txt") && wide_eq(extension, ".txt") && name as usize == path.as_ptr() as usize + 14
    }
}

fn version() -> bool {
    let mut name = [0u16; 48];
    wide(&mut name, "C:\\Windows\\SysWOW64\\kernel32.dll");
    let mut sub = [0u16; 4];
    wide(&mut sub, "\\");
    let mut handle = 0u32;
    // The version block of a system module is a few hundred bytes.
    let mut block = [0u8; 2048];
    let mut value: *mut u8 = core::ptr::null_mut();
    let mut length = 0u32;
    // SAFETY: documented version.dll calls with valid buffers.
    unsafe {
        let size = GetFileVersionInfoSizeW(name.as_ptr(), &mut handle);
        if size == 0 || size as usize > block.len() {
            return false;
        }
        if GetFileVersionInfoW(name.as_ptr(), 0, size, block.as_mut_ptr()) == 0 {
            return false;
        }
        // VS_FIXEDFILEINFO (52 bytes) starts with dwSignature 0xFEEF04BD.
        VerQueryValueW(block.as_ptr(), sub.as_ptr(), &mut value, &mut length) != 0
            && length == 52
            && !value.is_null()
            && value.cast::<u32>().read_unaligned() == 0xFEEF_04BD
    }
}

fn winmm_comctl() -> bool {
    // INITCOMMONCONTROLSEX: {dwSize 8, dwICC}.
    let init = [8u32, 0xFF];
    // SAFETY: plain calls.
    unsafe {
        let a = timeGetTime();
        let b = timeGetTime();
        b >= a && InitCommonControlsEx(init.as_ptr()) != 0
    }
}

/// `sockaddr_in` for 127.0.0.1 and `port` (host order).
fn loopback(port: u16) -> [u8; 16] {
    let mut a = [0u8; 16];
    a[0] = 2; // AF_INET
    a[2..4].copy_from_slice(&port.to_be_bytes());
    a[4..8].copy_from_slice(&[127, 0, 0, 1]);
    a
}

fn winsock() -> bool {
    let mut data = [0u8; 512];
    // SAFETY: documented ws2_32 calls with valid buffers.
    unsafe {
        // The 32-bit WSADATA starts with wVersion 0x0202 when asked for 2.2.
        if WSAStartup(0x0202, data.as_mut_ptr()) != 0 || data[0] != 2 || data[1] != 2 {
            return false;
        }
        let listener = socket(2, 1, 6);
        if listener == usize::MAX {
            return false;
        }
        let any = loopback(0);
        let mut name = [0u8; 16];
        let mut length = 16i32;
        let mut ok = bind(listener, any.as_ptr(), 16) == 0
            && getsockname(listener, name.as_mut_ptr(), &mut length) == 0
            && length == 16
            && name[0] == 2
            && listen(listener, 1) == 0;
        let port = u16::from_be_bytes([name[2], name[3]]);
        ok = ok && port != 0;
        let client = socket(2, 1, 6);
        ok = ok && client != usize::MAX && connect(client, name.as_ptr(), 16) == 0;
        let mut peer = [0u8; 16];
        let mut peer_length = 16i32;
        let server = if ok { accept(listener, peer.as_mut_ptr(), &mut peer_length) } else { usize::MAX };
        ok = ok && server != usize::MAX && peer[0] == 2 && peer[4..8] == [127, 0, 0, 1];
        let mut buffer = [0u8; 8];
        ok = ok
            && send(client, b"ping!".as_ptr(), 5, 0) == 5
            && recv(server, buffer.as_mut_ptr(), 8, 0) == 5
            && &buffer[..5] == b"ping!"
            && send(server, b"pong".as_ptr(), 4, 0) == 4
            && recv(client, buffer.as_mut_ptr(), 8, 0) == 4
            && &buffer[..4] == b"pong";
        for s in [server, client, listener] {
            if s != usize::MAX {
                closesocket(s);
            }
        }
        WSACleanup();
        ok
    }
}

fn bcrypt() -> bool {
    let mut random = [0u8; 16];
    let mut digest = [0u8; 32];
    let mut sha = [0u16; 8];
    wide(&mut sha, "SHA256");
    let mut provider: Handle = 0;
    // SAFETY: documented bcrypt calls with valid buffers.
    unsafe {
        // BCRYPT_USE_SYSTEM_PREFERRED_RNG = 2.
        if BCryptGenRandom(0, random.as_mut_ptr(), 16, 2) != 0 || random == [0u8; 16] {
            return false;
        }
        if BCryptOpenAlgorithmProvider(&mut provider, sha.as_ptr(), core::ptr::null(), 0) != 0 {
            return false;
        }
        let hashed = BCryptHash(provider, core::ptr::null(), 0, b"abc".as_ptr(), 3, digest.as_mut_ptr(), 32) == 0;
        BCryptCloseAlgorithmProvider(provider, 0);
        // SHA-256("abc") = ba7816bf 8f01cfea 414140de 5dae2223 ... (FIPS 180-4).
        hashed && digest[..8] == [0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea] && digest[31] == 0xad
    }
}

fn secur() -> bool {
    let mut name = [0u16; 128];
    let mut n = 128u32;
    // SAFETY: documented secur32 call; NameSamCompatible = 2.
    unsafe { GetUserNameExW(2, name.as_mut_ptr(), &mut n) != 0 && n > 0 && name[0] != 0 }
}

fn updater() -> bool {
    let mut own = [0u16; 300];
    let mut name = [0u16; 300];
    // SAFETY: documented kernel32 and msimg32 calls with valid buffers.
    unsafe {
        at(1);
        let n = GetModuleFileNameW(0, own.as_mut_ptr(), 300) as usize;
        let mut size = 300u32;
        if n == 0 || QueryFullProcessImageNameW(GetCurrentProcess(), 0, name.as_mut_ptr(), &mut size) == 0 {
            return false;
        }
        at(2);
        if size as usize != n || name[..n] != own[..n] || name[n] != 0 {
            return false;
        }
        at(3);
        let mut size = 300u32;
        let device: [u16; 8] = [92, 68, 101, 118, 105, 99, 101, 92]; // \Device\
        if QueryFullProcessImageNameW(GetCurrentProcess(), 1, name.as_mut_ptr(), &mut size) == 0
            || size < 8
            || name[..8] != device
        {
            return false;
        }
        at(4);
        let mut size = n as u32;
        if QueryFullProcessImageNameW(GetCurrentProcess(), 0, name.as_mut_ptr(), &mut size) != 0
            || GetLastError() != 122
            || size != n as u32
        {
            return false;
        }
        at(5);
        // AC_SRC_OVER, no flags, a constant alpha of 255, AC_SRC_ALPHA; no device contexts.
        let probe = core::hint::black_box(0x5EED_u32);
        let r = AlphaBlend(0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0x01FF_0000);
        if r != 0 || core::hint::black_box(probe) != 0x5EED {
            return false;
        }
        at(6);
        // gdi32's FrameRgn, RoundRect and OffsetWindowOrgEx on a memory DC (BLACK_BRUSH = 4).
        let dc = CreateCompatibleDC(0);
        let rgn = CreateRectRgn(0, 0, 10, 10);
        let mut old = [7i32; 2];
        let ok = dc != 0
            && FrameRgn(dc, rgn, GetStockObject(4), 1, 1) == 1
            && FrameRgn(dc, rgn, GetStockObject(4), 0, 1) == 0
            && RoundRect(dc, 0, 0, 10, 10, 3, 3) == 1
            && OffsetWindowOrgEx(dc, 2, 3, old.as_mut_ptr()) == 1
            && OffsetWindowOrgEx(dc, 1, 1, old.as_mut_ptr()) == 1
            && old == [2, 3]
            && core::hint::black_box(probe) == 0x5EED;
        DeleteObject(rgn);
        DeleteDC(dc);
        ok
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> u32 {
    let checks: [(&[u8], fn() -> bool); 12] = [
        (b"registry\n", registry),
        (b"token\n", token),
        (b"com\n", com),
        (b"oleaut\n", oleaut),
        (b"shell\n", shell),
        (b"shlwapi\n", shlwapi),
        (b"version\n", version),
        (b"winmm and comctl32\n", winmm_comctl),
        (b"winsock\n", winsock),
        (b"bcrypt\n", bcrypt),
        (b"secur32\n", secur),
        (b"updater imports\n", updater),
    ];
    if !out(b"sys32\n") {
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
