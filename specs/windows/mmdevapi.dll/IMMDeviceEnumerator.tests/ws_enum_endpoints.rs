//! IMMDeviceEnumerator::EnumAudioEndpoints: the active render endpoints of the machine (one sound card on the oracle) as
//! an IMMDeviceCollection; the data-flow and state-mask arguments are checked (Learn: "IMMDeviceEnumerator::
//! EnumAudioEndpoints", mmdeviceapi.h EDataFlow and DEVICE_STATE_*).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all, static_mut_refs)]

// The declarations in this file are from Microsoft Learn (prototypes, interface layouts) and the public SDK headers
// named beside them (constants, error codes: mmdeviceapi.h, audioclient.h, audiopolicy.h, audiosessiontypes.h,
// endpointvolume.h, mmreg.h, ksmedia.h, winerror.h, objbase.h); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use poly_wtest::*;

type HRESULT = u32;
type BOOL = i32;

// --- winerror.h ---
const S_OK: HRESULT = 0;
const S_FALSE: HRESULT = 1;
const E_NOINTERFACE: HRESULT = 0x8000_4002;
const E_POINTER: HRESULT = 0x8000_4003;
const E_FAIL: HRESULT = 0x8000_4005;
const E_UNEXPECTED: HRESULT = 0x8000_FFFF;
const RPC_X_NULL_REF_POINTER: HRESULT = 0x8007_06F4;
const E_INVALIDARG: HRESULT = 0x8007_0057;
const E_NOTFOUND: HRESULT = 0x8007_0490;
const E_OUTOFMEMORY: HRESULT = 0x8007_000E;
const CLASS_E_NOAGGREGATION: HRESULT = 0x8004_0110;
const REGDB_E_CLASSNOTREG: HRESULT = 0x8004_0154;

// --- audioclient.h ---
const AUDCLNT_E_NOT_INITIALIZED: HRESULT = 0x8889_0001;
const AUDCLNT_E_ALREADY_INITIALIZED: HRESULT = 0x8889_0002;
const AUDCLNT_E_WRONG_ENDPOINT_TYPE: HRESULT = 0x8889_0003;
const AUDCLNT_E_DEVICE_INVALIDATED: HRESULT = 0x8889_0004;
const AUDCLNT_E_NOT_STOPPED: HRESULT = 0x8889_0005;
const AUDCLNT_E_BUFFER_TOO_LARGE: HRESULT = 0x8889_0006;
const AUDCLNT_E_OUT_OF_ORDER: HRESULT = 0x8889_0007;
const AUDCLNT_E_UNSUPPORTED_FORMAT: HRESULT = 0x8889_0008;
const AUDCLNT_E_INVALID_SIZE: HRESULT = 0x8889_0009;
const AUDCLNT_E_DEVICE_IN_USE: HRESULT = 0x8889_000A;
const AUDCLNT_E_BUFFER_OPERATION_PENDING: HRESULT = 0x8889_000B;
const AUDCLNT_E_THREAD_NOT_REGISTERED: HRESULT = 0x8889_000C;
const AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED: HRESULT = 0x8889_000E;
const AUDCLNT_E_ENDPOINT_CREATE_FAILED: HRESULT = 0x8889_000F;
const AUDCLNT_E_SERVICE_NOT_RUNNING: HRESULT = 0x8889_0010;
const AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED: HRESULT = 0x8889_0011;
const AUDCLNT_E_EXCLUSIVE_MODE_ONLY: HRESULT = 0x8889_0012;
const AUDCLNT_E_BUFDURATION_PERIOD_NOT_EQUAL: HRESULT = 0x8889_0013;
const AUDCLNT_E_EVENTHANDLE_NOT_SET: HRESULT = 0x8889_0014;
const AUDCLNT_E_INCORRECT_BUFFER_SIZE: HRESULT = 0x8889_0015;
const AUDCLNT_E_BUFFER_SIZE_ERROR: HRESULT = 0x8889_0016;
const AUDCLNT_E_CPUUSAGE_EXCEEDED: HRESULT = 0x8889_0017;
const AUDCLNT_E_BUFFER_ERROR: HRESULT = 0x8889_0018;
const AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED: HRESULT = 0x8889_0019;
const AUDCLNT_E_INVALID_DEVICE_PERIOD: HRESULT = 0x8889_0020;
const AUDCLNT_E_INVALID_STREAM_FLAG: HRESULT = 0x8889_0021;
const AUDCLNT_S_BUFFER_EMPTY: HRESULT = 0x0889_0001;
const AUDCLNT_S_THREAD_ALREADY_REGISTERED: HRESULT = 0x0889_0002;
const AUDCLNT_S_POSITION_STALLED: HRESULT = 0x0889_0003;

const AUDCLNT_SHAREMODE_SHARED: u32 = 0;
const AUDCLNT_SHAREMODE_EXCLUSIVE: u32 = 1;
const AUDCLNT_STREAMFLAGS_CROSSPROCESS: u32 = 0x0001_0000;
const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x0002_0000;
const AUDCLNT_STREAMFLAGS_EVENTCALLBACK: u32 = 0x0004_0000;
const AUDCLNT_STREAMFLAGS_NOPERSIST: u32 = 0x0008_0000;
const AUDCLNT_STREAMFLAGS_RATEADJUST: u32 = 0x0010_0000;
const AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY: u32 = 0x0800_0000;
const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 0x8000_0000;
const AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY: u32 = 1;
const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 2;
const AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR: u32 = 4;

// --- mmdeviceapi.h ---
const eRender: u32 = 0;
const eCapture: u32 = 1;
const eAll: u32 = 2;
const eConsole: u32 = 0;
const eMultimedia: u32 = 1;
const eCommunications: u32 = 2;
const DEVICE_STATE_ACTIVE: u32 = 1;
const DEVICE_STATE_DISABLED: u32 = 2;
const DEVICE_STATE_NOTPRESENT: u32 = 4;
const DEVICE_STATE_UNPLUGGED: u32 = 8;
const DEVICE_STATEMASK_ALL: u32 = 0xF;
const CLSCTX_INPROC_SERVER: u32 = 1;
const CLSCTX_ALL: u32 = 0x17;
const COINIT_MULTITHREADED: u32 = 0;
const COINIT_APARTMENTTHREADED: u32 = 2;

// --- audiopolicy.h: AudioSessionState ---
const AudioSessionStateInactive: u32 = 0;
const AudioSessionStateActive: u32 = 1;
const AudioSessionStateExpired: u32 = 2;

// --- winbase.h ---
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const INFINITE: u32 = 0xFFFF_FFFF;

// --- mmreg.h ---
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

#[repr(C)]
#[derive(Clone, Copy, PartialEq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> GUID {
    GUID { d1, d2, d3, d4 }
}

const CLSID_MMDeviceEnumerator: GUID =
    guid(0xBCDE0395, 0xE52F, 0x467C, [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E]);
const IID_IUnknown: GUID = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IMMDeviceEnumerator: GUID =
    guid(0xA95664D2, 0x9614, 0x4F35, [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6]);
const IID_IMMDeviceCollection: GUID =
    guid(0x0BD7A1BE, 0x7A1A, 0x44DB, [0x83, 0x97, 0xCC, 0x53, 0x92, 0x38, 0x7B, 0x5E]);
const IID_IMMDevice: GUID = guid(0xD666063F, 0x1587, 0x4E43, [0x81, 0xF1, 0xB9, 0x48, 0xE8, 0x07, 0x36, 0x3F]);
const IID_IMMNotificationClient: GUID =
    guid(0x7991EEC9, 0x7E89, 0x4D85, [0x83, 0x90, 0x6C, 0x70, 0x3C, 0xEC, 0x60, 0xC0]);
const IID_IAudioClient: GUID = guid(0x1CB9AD4C, 0xDBFA, 0x4C32, [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2]);
const IID_IAudioClient2: GUID = guid(0x726778CD, 0xF60A, 0x4EDA, [0x82, 0xDE, 0xE4, 0x76, 0x10, 0xCD, 0x78, 0xAA]);
const IID_IAudioClient3: GUID = guid(0x7ED4EE07, 0x8E67, 0x4CD4, [0x8C, 0x1A, 0x2B, 0x7A, 0x59, 0x87, 0xAD, 0x42]);
const IID_IAudioRenderClient: GUID = guid(0xF294ACFC, 0x3146, 0x4483, [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2]);
const IID_IAudioCaptureClient: GUID =
    guid(0xC8ADBD64, 0xE71E, 0x48A0, [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17]);
const IID_IAudioClock: GUID = guid(0xCD63314F, 0x3FBA, 0x4A1B, [0x81, 0x2C, 0xEF, 0x96, 0x35, 0x87, 0x28, 0xE7]);
const IID_IAudioClock2: GUID = guid(0x6F49FF73, 0x6727, 0x49AC, [0xA0, 0x08, 0xD9, 0x8C, 0xF5, 0xE7, 0x00, 0x48]);
const IID_IAudioSessionControl: GUID =
    guid(0xF4B1A599, 0x7266, 0x4319, [0xA8, 0xCA, 0xE7, 0x0A, 0xCB, 0x11, 0xE8, 0xCD]);
const IID_IAudioSessionEvents: GUID =
    guid(0x24918ACC, 0x64B3, 0x37C1, [0x8C, 0xA9, 0x74, 0xA6, 0x6E, 0x99, 0x57, 0xA8]);
const IID_ISimpleAudioVolume: GUID = guid(0x87CE5498, 0x68D6, 0x44E5, [0x92, 0x15, 0x6D, 0xA4, 0x7E, 0xF8, 0x83, 0xD8]);
/// A made-up interface id nothing implements.
const IID_Nothing: GUID = guid(0x1234_5678, 0x9ABC, 0xDEF0, [1, 2, 3, 4, 5, 6, 7, 8]);
const SUBTYPE_PCM: GUID = guid(1, 0, 0x10, [0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
const SUBTYPE_FLOAT: GUID = guid(3, 0, 0x10, [0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);

#[repr(C, packed(1))]
#[derive(Clone, Copy)]
struct WAVEFORMATEX {
    wFormatTag: u16,
    nChannels: u16,
    nSamplesPerSec: u32,
    nAvgBytesPerSec: u32,
    nBlockAlign: u16,
    wBitsPerSample: u16,
    cbSize: u16,
}

#[repr(C, packed(1))]
#[derive(Clone, Copy)]
struct WAVEFORMATEXTENSIBLE {
    Format: WAVEFORMATEX,
    wValidBitsPerSample: u16,
    dwChannelMask: u32,
    SubFormat: GUID,
}

fn pcm(rate: u32, ch: u16, bits: u16) -> WAVEFORMATEX {
    let align = ch * (bits / 8);
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM,
        nChannels: ch,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * align as u32,
        nBlockAlign: align,
        wBitsPerSample: bits,
        cbSize: 0,
    }
}

fn ext(rate: u32, ch: u16, bits: u16, float: bool, mask: u32) -> WAVEFORMATEXTENSIBLE {
    let align = ch * (bits / 8);
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE,
            nChannels: ch,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * align as u32,
            nBlockAlign: align,
            wBitsPerSample: bits,
            cbSize: 22,
        },
        wValidBitsPerSample: bits,
        dwChannelMask: mask,
        SubFormat: if float { SUBTYPE_FLOAT } else { SUBTYPE_PCM },
    }
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: usize, flags: u32) -> HRESULT;
    fn CoUninitialize();
    fn CoCreateInstance(clsid: *const GUID, outer: usize, ctx: u32, iid: *const GUID, out: *mut usize) -> HRESULT;
    fn CoTaskMemFree(p: usize);
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateEventW(attr: *const c_void, manual: BOOL, initial: BOOL, name: *const u16) -> usize;
    fn SetEvent(h: usize) -> BOOL;
    fn ResetEvent(h: usize) -> BOOL;
    fn CloseHandle(h: usize) -> BOOL;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn Sleep(ms: u32);
    fn GetCurrentThreadId() -> u32;
    fn GetTickCount64() -> u64;
    fn QueryPerformanceCounter(v: *mut i64) -> BOOL;
    fn QueryPerformanceFrequency(v: *mut i64) -> BOOL;
    fn CreateThread(
        attr: *const c_void,
        stack: usize,
        f: extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> usize;
    fn GetTickCount() -> u32;
}

// ---------------------------------------------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------------------------------------------

struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}
impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}
impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

macro_rules! say {
    ($t:expr, $($arg:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($arg)*);
        $t.line(b.as_str());
    }};
}

fn hr_name(r: HRESULT) -> &'static str {
    match r {
        S_OK => "S_OK",
        S_FALSE => "S_FALSE",
        E_NOINTERFACE => "E_NOINTERFACE",
        E_POINTER => "E_POINTER",
        E_FAIL => "E_FAIL",
        E_UNEXPECTED => "E_UNEXPECTED",
        RPC_X_NULL_REF_POINTER => "RPC_X_NULL_REF_POINTER",
        E_INVALIDARG => "E_INVALIDARG",
        E_NOTFOUND => "E_NOTFOUND",
        E_OUTOFMEMORY => "E_OUTOFMEMORY",
        CLASS_E_NOAGGREGATION => "CLASS_E_NOAGGREGATION",
        REGDB_E_CLASSNOTREG => "REGDB_E_CLASSNOTREG",
        AUDCLNT_E_NOT_INITIALIZED => "AUDCLNT_E_NOT_INITIALIZED",
        AUDCLNT_E_ALREADY_INITIALIZED => "AUDCLNT_E_ALREADY_INITIALIZED",
        AUDCLNT_E_WRONG_ENDPOINT_TYPE => "AUDCLNT_E_WRONG_ENDPOINT_TYPE",
        AUDCLNT_E_DEVICE_INVALIDATED => "AUDCLNT_E_DEVICE_INVALIDATED",
        AUDCLNT_E_NOT_STOPPED => "AUDCLNT_E_NOT_STOPPED",
        AUDCLNT_E_BUFFER_TOO_LARGE => "AUDCLNT_E_BUFFER_TOO_LARGE",
        AUDCLNT_E_OUT_OF_ORDER => "AUDCLNT_E_OUT_OF_ORDER",
        AUDCLNT_E_UNSUPPORTED_FORMAT => "AUDCLNT_E_UNSUPPORTED_FORMAT",
        AUDCLNT_E_INVALID_SIZE => "AUDCLNT_E_INVALID_SIZE",
        AUDCLNT_E_DEVICE_IN_USE => "AUDCLNT_E_DEVICE_IN_USE",
        AUDCLNT_E_BUFFER_OPERATION_PENDING => "AUDCLNT_E_BUFFER_OPERATION_PENDING",
        AUDCLNT_E_THREAD_NOT_REGISTERED => "AUDCLNT_E_THREAD_NOT_REGISTERED",
        AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => "AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED",
        AUDCLNT_E_ENDPOINT_CREATE_FAILED => "AUDCLNT_E_ENDPOINT_CREATE_FAILED",
        AUDCLNT_E_SERVICE_NOT_RUNNING => "AUDCLNT_E_SERVICE_NOT_RUNNING",
        AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED => "AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED",
        AUDCLNT_E_EXCLUSIVE_MODE_ONLY => "AUDCLNT_E_EXCLUSIVE_MODE_ONLY",
        AUDCLNT_E_BUFDURATION_PERIOD_NOT_EQUAL => "AUDCLNT_E_BUFDURATION_PERIOD_NOT_EQUAL",
        AUDCLNT_E_EVENTHANDLE_NOT_SET => "AUDCLNT_E_EVENTHANDLE_NOT_SET",
        AUDCLNT_E_INCORRECT_BUFFER_SIZE => "AUDCLNT_E_INCORRECT_BUFFER_SIZE",
        AUDCLNT_E_BUFFER_SIZE_ERROR => "AUDCLNT_E_BUFFER_SIZE_ERROR",
        AUDCLNT_E_CPUUSAGE_EXCEEDED => "AUDCLNT_E_CPUUSAGE_EXCEEDED",
        AUDCLNT_E_BUFFER_ERROR => "AUDCLNT_E_BUFFER_ERROR",
        AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED => "AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED",
        AUDCLNT_E_INVALID_DEVICE_PERIOD => "AUDCLNT_E_INVALID_DEVICE_PERIOD",
        AUDCLNT_E_INVALID_STREAM_FLAG => "AUDCLNT_E_INVALID_STREAM_FLAG",
        AUDCLNT_S_BUFFER_EMPTY => "AUDCLNT_S_BUFFER_EMPTY",
        AUDCLNT_S_THREAD_ALREADY_REGISTERED => "AUDCLNT_S_THREAD_ALREADY_REGISTERED",
        AUDCLNT_S_POSITION_STALLED => "AUDCLNT_S_POSITION_STALLED",
        _ => "other",
    }
}

/// `r` must be `want`; writes `name = <HRESULT name> (0x...)`.
fn hr(t: &mut Test, name: &str, r: HRESULT, want: HRESULT) -> bool {
    say!(t, "{} = {} ({:#010x})", name, hr_name(r), r);
    t.expect(name, r as i64, want as i64)
}

fn settle(ms: u32) {
    // SAFETY: plain call.
    unsafe { Sleep(ms) };
}

fn wait_for(ms: u32, mut cond: impl FnMut() -> bool) -> bool {
    // SAFETY: plain call.
    let start = unsafe { GetTickCount64() };
    loop {
        if cond() {
            return true;
        }
        // SAFETY: plain call.
        if unsafe { GetTickCount64() } - start >= ms as u64 {
            return false;
        }
        unsafe { Sleep(1) };
    }
}

fn new_event(manual: bool) -> usize {
    // SAFETY: unnamed event.
    unsafe { CreateEventW(core::ptr::null(), manual as i32, 0, core::ptr::null()) }
}

fn evt_wait(e: usize, ms: u32) -> bool {
    // SAFETY: `e` is an event handle.
    unsafe { WaitForSingleObject(e, ms) == WAIT_OBJECT_0 }
}

// ---------------------------------------------------------------------------------------------------------------
// COM calls through vtables
// ---------------------------------------------------------------------------------------------------------------

macro_rules! vcall {
    ($p:expr, $slot:expr, ($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let obj: usize = $p;
        // SAFETY: `obj` is a COM interface pointer the test got from the system (or a null one on purpose, which these
        // tests never call through); the slot and signature are from the interface's header.
        unsafe {
            let vt = *(obj as *const *const usize);
            let f: extern "system" fn(usize $(, $t)*) -> $r = core::mem::transmute::<usize, _>(*vt.add($slot));
            f(obj $(, $a)*)
        }
    }};
}

fn release(p: usize) -> u32 {
    if p == 0 {
        return 0;
    }
    vcall!(p, 2, () -> u32)
}
fn addref(p: usize) -> u32 {
    vcall!(p, 1, () -> u32)
}
fn qi(p: usize, iid: &GUID) -> (HRESULT, usize) {
    let mut out = 0usize;
    let r = vcall!(p, 0, (*const GUID, *mut usize) -> HRESULT, iid, &mut out);
    (r, out)
}

/// COM initialised on this thread (multithreaded apartment unless the test says otherwise).
fn com_init() -> HRESULT {
    // SAFETY: plain call.
    unsafe { CoInitializeEx(0, COINIT_MULTITHREADED) }
}

fn enumerator() -> (HRESULT, usize) {
    let mut p = 0usize;
    // SAFETY: valid pointers.
    let r = unsafe { CoCreateInstance(&CLSID_MMDeviceEnumerator, 0, CLSCTX_ALL, &IID_IMMDeviceEnumerator, &mut p) };
    (r, p)
}

// IMMDeviceEnumerator
fn en_enum(e: usize, flow: u32, mask: u32) -> (HRESULT, usize) {
    let mut c = 0usize;
    let r = vcall!(e, 3, (u32, u32, *mut usize) -> HRESULT, flow, mask, &mut c);
    (r, c)
}
fn en_default(e: usize, flow: u32, role: u32) -> (HRESULT, usize) {
    let mut d = 0usize;
    let r = vcall!(e, 4, (u32, u32, *mut usize) -> HRESULT, flow, role, &mut d);
    (r, d)
}
fn en_get_device(e: usize, id: *const u16) -> (HRESULT, usize) {
    let mut d = 0usize;
    let r = vcall!(e, 5, (*const u16, *mut usize) -> HRESULT, id, &mut d);
    (r, d)
}
// IMMDeviceCollection
fn co_count(c: usize) -> (HRESULT, u32) {
    let mut n = 0xdead_beefu32;
    let r = vcall!(c, 3, (*mut u32) -> HRESULT, &mut n);
    (r, n)
}
fn co_item(c: usize, i: u32) -> (HRESULT, usize) {
    let mut d = 0usize;
    let r = vcall!(c, 4, (u32, *mut usize) -> HRESULT, i, &mut d);
    (r, d)
}
// IMMDevice
fn dev_activate(d: usize, iid: &GUID, ctx: u32) -> (HRESULT, usize) {
    let mut o = 0usize;
    let r = vcall!(d, 3, (*const GUID, u32, usize, *mut usize) -> HRESULT, iid, ctx, 0usize, &mut o);
    (r, o)
}
fn dev_id(d: usize) -> (HRESULT, usize) {
    let mut s = 0usize;
    let r = vcall!(d, 5, (*mut usize) -> HRESULT, &mut s);
    (r, s)
}
fn dev_state(d: usize) -> (HRESULT, u32) {
    let mut s = 0xdead_beefu32;
    let r = vcall!(d, 6, (*mut u32) -> HRESULT, &mut s);
    (r, s)
}

// IAudioClient
fn ac_initialize(c: usize, mode: u32, flags: u32, dur: i64, per: i64, fmt: *const WAVEFORMATEX) -> HRESULT {
    vcall!(c, 3, (u32, u32, i64, i64, *const WAVEFORMATEX, usize) -> HRESULT, mode, flags, dur, per, fmt, 0usize)
}
fn ac_buffer_size(c: usize) -> (HRESULT, u32) {
    let mut n = 0xdead_beefu32;
    let r = vcall!(c, 4, (*mut u32) -> HRESULT, &mut n);
    (r, n)
}
fn ac_latency(c: usize) -> (HRESULT, i64) {
    let mut n = -77i64;
    let r = vcall!(c, 5, (*mut i64) -> HRESULT, &mut n);
    (r, n)
}
fn ac_padding(c: usize) -> (HRESULT, u32) {
    let mut n = 0xdead_beefu32;
    let r = vcall!(c, 6, (*mut u32) -> HRESULT, &mut n);
    (r, n)
}
fn ac_is_supported(c: usize, mode: u32, fmt: *const WAVEFORMATEX) -> (HRESULT, usize) {
    let mut closest = 0usize;
    let r = vcall!(c, 7, (u32, *const WAVEFORMATEX, *mut usize) -> HRESULT, mode, fmt, &mut closest);
    (r, closest)
}
fn ac_mix_format(c: usize) -> (HRESULT, usize) {
    let mut f = 0usize;
    let r = vcall!(c, 8, (*mut usize) -> HRESULT, &mut f);
    (r, f)
}
fn ac_period(c: usize) -> (HRESULT, i64, i64) {
    let mut d = -77i64;
    let mut m = -77i64;
    let r = vcall!(c, 9, (*mut i64, *mut i64) -> HRESULT, &mut d, &mut m);
    (r, d, m)
}
fn ac_start(c: usize) -> HRESULT {
    vcall!(c, 10, () -> HRESULT)
}
fn ac_stop(c: usize) -> HRESULT {
    vcall!(c, 11, () -> HRESULT)
}
fn ac_reset(c: usize) -> HRESULT {
    vcall!(c, 12, () -> HRESULT)
}
fn ac_set_event(c: usize, e: usize) -> HRESULT {
    vcall!(c, 13, (usize) -> HRESULT, e)
}
fn ac_get_service(c: usize, iid: &GUID) -> (HRESULT, usize) {
    let mut o = 0usize;
    let r = vcall!(c, 14, (*const GUID, *mut usize) -> HRESULT, iid, &mut o);
    (r, o)
}
// IAudioClient3
fn ac3_shared_period(c: usize, fmt: *const WAVEFORMATEX) -> (HRESULT, [u32; 4]) {
    let mut v = [0xdead_beefu32; 4];
    let r = vcall!(c, 18, (*const WAVEFORMATEX, *mut u32, *mut u32, *mut u32, *mut u32) -> HRESULT, fmt, &mut v[0], &mut v[1], &mut v[2], &mut v[3]);
    (r, v)
}
fn ac3_current_period(c: usize) -> (HRESULT, usize, u32) {
    let mut f = 0usize;
    let mut p = 0xdead_beefu32;
    let r = vcall!(c, 19, (*mut usize, *mut u32) -> HRESULT, &mut f, &mut p);
    (r, f, p)
}
// IAudioRenderClient
fn rc_get_buffer(r: usize, frames: u32) -> (HRESULT, usize) {
    let mut d = 0usize;
    let h = vcall!(r, 3, (u32, *mut usize) -> HRESULT, frames, &mut d);
    (h, d)
}
fn rc_release_buffer(r: usize, frames: u32, flags: u32) -> HRESULT {
    vcall!(r, 4, (u32, u32) -> HRESULT, frames, flags)
}
// IAudioClock
fn clk_frequency(c: usize) -> (HRESULT, u64) {
    let mut f = 0u64;
    let r = vcall!(c, 3, (*mut u64) -> HRESULT, &mut f);
    (r, f)
}
fn clk_position(c: usize) -> (HRESULT, u64, u64) {
    let mut p = u64::MAX;
    let mut q = u64::MAX;
    let r = vcall!(c, 4, (*mut u64, *mut u64) -> HRESULT, &mut p, &mut q);
    (r, p, q)
}
fn clk_characteristics(c: usize) -> (HRESULT, u32) {
    let mut v = 0xdead_beefu32;
    let r = vcall!(c, 5, (*mut u32) -> HRESULT, &mut v);
    (r, v)
}
// IAudioSessionControl
fn sc_state(c: usize) -> (HRESULT, u32) {
    let mut s = 0xdead_beefu32;
    let r = vcall!(c, 3, (*mut u32) -> HRESULT, &mut s);
    (r, s)
}
fn sc_register(c: usize, n: usize) -> HRESULT {
    vcall!(c, 10, (usize) -> HRESULT, n)
}
fn sc_unregister(c: usize, n: usize) -> HRESULT {
    vcall!(c, 11, (usize) -> HRESULT, n)
}

// ---------------------------------------------------------------------------------------------------------------
// Reading a WAVEFORMATEX the system returned
// ---------------------------------------------------------------------------------------------------------------

fn rd16(p: usize) -> u16 {
    // SAFETY: the system returned this block.
    unsafe { core::ptr::read_unaligned(p as *const u16) }
}
fn rd32(p: usize) -> u32 {
    // SAFETY: as above.
    unsafe { core::ptr::read_unaligned(p as *const u32) }
}

struct Fmt {
    tag: u16,
    ch: u16,
    rate: u32,
    avg: u32,
    align: u16,
    bits: u16,
    cb: u16,
    valid: u16,
    mask: u32,
    float: bool,
}

fn read_fmt(p: usize) -> Fmt {
    let tag = rd16(p);
    let cb = rd16(p + 16);
    let mut f = Fmt {
        tag,
        ch: rd16(p + 2),
        rate: rd32(p + 4),
        avg: rd32(p + 8),
        align: rd16(p + 12),
        bits: rd16(p + 14),
        cb,
        valid: 0,
        mask: 0,
        float: tag == WAVE_FORMAT_IEEE_FLOAT,
    };
    if tag == WAVE_FORMAT_EXTENSIBLE && cb >= 22 {
        f.valid = rd16(p + 18);
        f.mask = rd32(p + 20);
        // SAFETY: the sub-format GUID follows.
        let g = unsafe { core::ptr::read_unaligned((p + 24) as *const GUID) };
        f.float = g == SUBTYPE_FLOAT;
    }
    f
}

fn say_fmt(t: &mut Test, label: &str, f: &Fmt) {
    say!(
        t,
        "{}: tag {:#x} channels {} rate {} avg {} align {} bits {} cb {} valid {} mask {:#x} float {}",
        label,
        f.tag,
        f.ch,
        f.rate,
        f.avg,
        f.align,
        f.bits,
        f.cb,
        f.valid,
        f.mask,
        f.float
    );
}

/// Everything a render test needs: COM up, the default render device's client (not initialised yet).
struct Dev {
    enumerator: usize,
    device: usize,
    client: usize,
}

fn open_device(t: &mut Test) -> Option<Dev> {
    let c = com_init();
    if c != S_OK && c != S_FALSE {
        say!(t, "CoInitializeEx failed {:#x}", c);
        t.check("COM initialises", false, c as i64);
        return None;
    }
    let (r, e) = enumerator();
    if r != S_OK {
        hr(t, "CoCreateInstance(MMDeviceEnumerator)", r, S_OK);
        return None;
    }
    let (r, d) = en_default(e, eRender, eConsole);
    if r != S_OK {
        hr(t, "GetDefaultAudioEndpoint", r, S_OK);
        return None;
    }
    let (r, c) = dev_activate(d, &IID_IAudioClient, CLSCTX_INPROC_SERVER);
    if r != S_OK {
        hr(t, "Activate(IAudioClient)", r, S_OK);
        return None;
    }
    Some(Dev { enumerator: e, device: d, client: c })
}

/// A new client on the same device (Activate again).
fn new_client(d: &Dev) -> usize {
    let (r, c) = dev_activate(d.device, &IID_IAudioClient, CLSCTX_INPROC_SERVER);
    if r != S_OK {
        return 0;
    }
    c
}

/// The mix format of a client, read and freed.
fn mix(c: usize) -> (HRESULT, Fmt, usize) {
    let (r, p) = ac_mix_format(c);
    if r != S_OK || p == 0 {
        return (r, read_fmt_zero(), 0);
    }
    (r, read_fmt(p), p)
}

fn read_fmt_zero() -> Fmt {
    Fmt { tag: 0, ch: 0, rate: 0, avg: 0, align: 0, bits: 0, cb: 0, valid: 0, mask: 0, float: false }
}

/// Initialise `c` the way a browser does: shared mode, event callback, engine period, the mix format; set the event.
fn init_chrome_way(c: usize, ev: usize) -> HRESULT {
    let (r, f) = ac_mix_format(c);
    if r != S_OK {
        return r;
    }
    let h =
        ac_initialize(c, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 0, 0, f as *const WAVEFORMATEX);
    // SAFETY: the format block was allocated by the system for us.
    unsafe { CoTaskMemFree(f) };
    if h != S_OK {
        return h;
    }
    ac_set_event(c, ev)
}

fn qpc() -> i64 {
    let mut v = 0i64;
    // SAFETY: valid pointer.
    unsafe { QueryPerformanceCounter(&mut v) };
    v
}
fn qpf() -> i64 {
    let mut v = 0i64;
    // SAFETY: valid pointer.
    unsafe { QueryPerformanceFrequency(&mut v) };
    v
}

// ---------------------------------------------------------------------------------------------------------------
// Callback objects the test implements: IMMNotificationClient and IAudioSessionEvents
// ---------------------------------------------------------------------------------------------------------------

#[repr(C)]
struct Obj {
    vtbl: *const usize,
    iid: GUID,
}

static CB_COUNT: AtomicU32 = AtomicU32::new(0);
static CB_LOG: [AtomicU32; 32] = [const { AtomicU32::new(0) }; 32];
static CB_QI: AtomicU32 = AtomicU32::new(0);

fn cb_reset() {
    CB_COUNT.store(0, Ordering::SeqCst);
    CB_QI.store(0, Ordering::SeqCst);
}
fn cb_push(kind: u32) {
    let n = CB_COUNT.fetch_add(1, Ordering::SeqCst) as usize;
    if n < 32 {
        CB_LOG[n].store(kind, Ordering::SeqCst);
    }
}

extern "system" fn obj_qi(this: usize, iid: *const GUID, out: *mut usize) -> HRESULT {
    // SAFETY: COM passes valid pointers; `this` is one of our static objects.
    unsafe {
        let o = this as *const Obj;
        let v = *iid;
        CB_QI.fetch_add(1, Ordering::SeqCst);
        if v == IID_IUnknown || v == (*o).iid {
            *out = this;
            return S_OK;
        }
        *out = 0;
        E_NOINTERFACE
    }
}
extern "system" fn obj_addref(_t: usize) -> u32 {
    2
}
extern "system" fn obj_release(_t: usize) -> u32 {
    1
}

// IAudioSessionEvents: kinds 1.. in vtable order
extern "system" fn ev_display_name(_t: usize, _n: usize, _c: usize) -> HRESULT {
    cb_push(1);
    S_OK
}
extern "system" fn ev_icon_path(_t: usize, _n: usize, _c: usize) -> HRESULT {
    cb_push(2);
    S_OK
}
extern "system" fn ev_simple_volume(_t: usize, _v: f32, _m: i32, _c: usize) -> HRESULT {
    cb_push(3);
    S_OK
}
extern "system" fn ev_channel_volume(_t: usize, _n: u32, _v: usize, _ch: u32, _c: usize) -> HRESULT {
    cb_push(4);
    S_OK
}
extern "system" fn ev_grouping(_t: usize, _n: usize, _c: usize) -> HRESULT {
    cb_push(5);
    S_OK
}
extern "system" fn ev_state(_t: usize, state: u32) -> HRESULT {
    cb_push(0x100 + state);
    S_OK
}
extern "system" fn ev_disconnected(_t: usize, reason: u32) -> HRESULT {
    cb_push(0x200 + reason);
    S_OK
}

static mut SESSION_EVENTS_VT: [usize; 10] = [0; 10];
static mut SESSION_EVENTS: Obj = Obj { vtbl: core::ptr::null(), iid: IID_IAudioSessionEvents };

fn session_events() -> usize {
    // SAFETY: single-threaded set-up of statics used only by these tests.
    unsafe {
        SESSION_EVENTS_VT = [
            obj_qi as usize,
            obj_addref as usize,
            obj_release as usize,
            ev_display_name as usize,
            ev_icon_path as usize,
            ev_simple_volume as usize,
            ev_channel_volume as usize,
            ev_grouping as usize,
            ev_state as usize,
            ev_disconnected as usize,
        ];
        SESSION_EVENTS.vtbl = core::ptr::addr_of!(SESSION_EVENTS_VT) as *const usize;
        core::ptr::addr_of_mut!(SESSION_EVENTS) as usize
    }
}

// IMMNotificationClient: OnDeviceStateChanged(id, state), OnDeviceAdded(id), OnDeviceRemoved(id), OnDefaultDeviceChanged(flow, role, id), OnPropertyValueChanged(id, key)
extern "system" fn nc_state(_t: usize, _id: usize, _s: u32) -> HRESULT {
    cb_push(0x300);
    S_OK
}
extern "system" fn nc_added(_t: usize, _id: usize) -> HRESULT {
    cb_push(0x301);
    S_OK
}
extern "system" fn nc_removed(_t: usize, _id: usize) -> HRESULT {
    cb_push(0x302);
    S_OK
}
extern "system" fn nc_default(_t: usize, _f: u32, _r: u32, _id: usize) -> HRESULT {
    cb_push(0x303);
    S_OK
}
extern "system" fn nc_property(_t: usize, _id: usize, _k: usize) -> HRESULT {
    cb_push(0x304);
    S_OK
}

static mut NOTIFY_VT: [usize; 8] = [0; 8];
static mut NOTIFY: Obj = Obj { vtbl: core::ptr::null(), iid: IID_IMMNotificationClient };

fn notify_client() -> usize {
    // SAFETY: as above.
    unsafe {
        NOTIFY_VT = [
            obj_qi as usize,
            obj_addref as usize,
            obj_release as usize,
            nc_state as usize,
            nc_added as usize,
            nc_removed as usize,
            nc_default as usize,
            nc_property as usize,
        ];
        NOTIFY.vtbl = core::ptr::addr_of!(NOTIFY_VT) as *const usize;
        core::ptr::addr_of_mut!(NOTIFY) as usize
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Event counting thread: counts how often an event handle is set while `RUN` is true (auto-reset event).
// ---------------------------------------------------------------------------------------------------------------

static COUNT_RUN: AtomicU32 = AtomicU32::new(0);
static COUNT_N: AtomicU32 = AtomicU32::new(0);
static COUNT_EVENT: AtomicUsize = AtomicUsize::new(0);

extern "system" fn counter(_p: *mut c_void) -> u32 {
    let e = COUNT_EVENT.load(Ordering::SeqCst);
    while COUNT_RUN.load(Ordering::SeqCst) != 0 {
        // SAFETY: valid event handle.
        if unsafe { WaitForSingleObject(e, 20) } == WAIT_OBJECT_0 {
            COUNT_N.fetch_add(1, Ordering::SeqCst);
        }
    }
    0
}

/// Write `frames` frames of silence through the render client (the SILENT flag): the data is not touched.
fn fill_silent(rc: usize, frames: u32) -> HRESULT {
    let (r, _) = rc_get_buffer(rc, frames);
    if r != S_OK {
        return r;
    }
    rc_release_buffer(rc, frames, AUDCLNT_BUFFERFLAGS_SILENT)
}

/// A client initialised the way a browser does (shared, event callback, mix format) with its event, services and buffer size.
struct Stream {
    client: usize,
    event: usize,
    render: usize,
    clock: usize,
    session: usize,
    frames: u32,
    align: u32,
    rate: u32,
}

fn open_stream(t: &mut Test, d: &Dev) -> Option<Stream> {
    let c = new_client(d);
    let ev = new_event(false);
    let r = init_chrome_way(c, ev);
    if r != S_OK {
        hr(t, "Initialize", r, S_OK);
        return None;
    }
    let (_, frames) = ac_buffer_size(c);
    let (_, rc) = ac_get_service(c, &IID_IAudioRenderClient);
    let (_, clk) = ac_get_service(c, &IID_IAudioClock);
    let (_, sc) = ac_get_service(c, &IID_IAudioSessionControl);
    let (_, f, p) = mix(c);
    if p != 0 {
        // SAFETY: allocated by the system for us.
        unsafe { CoTaskMemFree(p) };
    }
    Some(Stream {
        client: c,
        event: ev,
        render: rc,
        clock: clk,
        session: sc,
        frames,
        align: f.align as u32,
        rate: f.rate,
    })
}

fn count_events(ev: usize, ms: u32) -> u32 {
    // SAFETY: plain calls.
    let t0 = unsafe { GetTickCount64() };
    let mut n = 0;
    while unsafe { GetTickCount64() } - t0 < ms as u64 {
        if evt_wait(ev, 20) {
            n += 1;
        }
    }
    n
}

fn wlen(p: usize) -> usize {
    let mut n = 0;
    while rd16(p + 2 * n) != 0 && n < 400 {
        n += 1;
    }
    n
}

fn wstr_is(p: usize, s: &str) -> bool {
    let b = s.as_bytes();
    wlen(p) == b.len() && b.iter().enumerate().all(|(i, &c)| rd16(p + 2 * i) == c as u16)
}

fn wstr_starts(p: usize, s: &str) -> bool {
    let b = s.as_bytes();
    wlen(p) >= b.len() && b.iter().enumerate().all(|(i, &c)| rd16(p + 2 * i) == c as u16)
}

fn wstr_equal(a: usize, b: usize) -> bool {
    let n = wlen(a);
    n == wlen(b) && (0..n).all(|i| rd16(a + 2 * i) == rd16(b + 2 * i))
}

/// A device id has the form `{0.0.0.00000000}.{<guid>}`: 55 characters.
fn id_form_ok(p: usize) -> bool {
    wstr_starts(p, "{0.0.0.00000000}.{") && wlen(p) == 55 && rd16(p + 2 * 54) == b'}' as u16
}

/// A made-up id string (UTF-16, NUL-terminated) in a fixed buffer.
fn wide_id(s: &str, out: &mut [u16; 80]) -> *const u16 {
    for (i, c) in s.bytes().enumerate() {
        out[i] = c as u16;
    }
    out.as_ptr()
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    com_init();
    let (r, e) = enumerator();
    hr(t, "create", r, S_OK);
    let (r, c) = en_enum(e, eRender, DEVICE_STATE_ACTIVE);
    hr(t, "render, active", r, S_OK);
    let (r, n) = co_count(c);
    hr(t, "GetCount", r, S_OK);
    say!(t, "active render endpoints = {}", n);
    t.expect("one active render endpoint", n as i64, 1);
    release(c);
    // eAll with the active mask contains at least the render endpoint
    let (r, c) = en_enum(e, eAll, DEVICE_STATE_ACTIVE);
    hr(t, "all flows, active", r, S_OK);
    let (_, n_all) = co_count(c);
    t.check("includes the render endpoint", n_all >= 1, n_all as i64);
    release(c);
    // a mask that is a subset of the states is valid; 0 and bits outside the mask are not
    let (r, c) = en_enum(e, eRender, DEVICE_STATE_ACTIVE | DEVICE_STATE_DISABLED);
    hr(t, "render, active|disabled", r, S_OK);
    let (_, n2) = co_count(c);
    t.check("at least the active one", n2 >= 1, n2 as i64);
    release(c);
    let (r, c) = en_enum(e, eRender, 0);
    hr(t, "state mask 0", r, E_INVALIDARG);
    t.expect("no collection", c as i64, 0);
    let (r, c) = en_enum(e, eRender, 0x10);
    hr(t, "state mask 0x10", r, E_INVALIDARG);
    t.expect("no collection", c as i64, 0);
    let (r, c) = en_enum(e, 3, DEVICE_STATE_ACTIVE);
    hr(t, "data flow 3", r, E_INVALIDARG);
    t.expect("no collection", c as i64, 0);
    let r = vcall!(e, 3, (u32, u32, usize) -> HRESULT, eRender, DEVICE_STATE_ACTIVE, 0usize);
    hr(t, "NULL output pointer", r, E_POINTER);
    release(e);
}
