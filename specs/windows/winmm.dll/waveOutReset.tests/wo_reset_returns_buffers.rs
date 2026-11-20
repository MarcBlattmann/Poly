//! waveOutReset: stops playing and gives every queued buffer back: before the call returns each one has WHDR_DONE
//! set and WHDR_INQUEUE cleared and has had its WOM_DONE callback, in the order written; the position goes back to 0.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the public SDK headers
// named beside them (constants, layouts: mmsystem.h, mmeapi.h, mmreg.h, ksmedia.h, winbase.h,
// winerror.h); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use poly_wtest::*;

type BOOL = i32;
type HWAVEOUT = usize;

// --- mmsystem.h: MMRESULT ---
const MMSYSERR_NOERROR: u32 = 0;
const MMSYSERR_ERROR: u32 = 1;
const MMSYSERR_BADDEVICEID: u32 = 2;
const MMSYSERR_NOTENABLED: u32 = 3;
const MMSYSERR_ALLOCATED: u32 = 4;
const MMSYSERR_INVALHANDLE: u32 = 5;
const MMSYSERR_NODRIVER: u32 = 6;
const MMSYSERR_NOMEM: u32 = 7;
const MMSYSERR_NOTSUPPORTED: u32 = 8;
const MMSYSERR_BADERRNUM: u32 = 9;
const MMSYSERR_INVALFLAG: u32 = 10;
const MMSYSERR_INVALPARAM: u32 = 11;
const MMSYSERR_HANDLEBUSY: u32 = 12;
const MMSYSERR_INVALIDALIAS: u32 = 13;
const MMSYSERR_BADDB: u32 = 14;
const MMSYSERR_KEYNOTFOUND: u32 = 15;
const MMSYSERR_READERROR: u32 = 16;
const MMSYSERR_WRITEERROR: u32 = 17;
const MMSYSERR_DELETEERROR: u32 = 18;
const MMSYSERR_VALNOTFOUND: u32 = 19;
const MMSYSERR_NODRIVERCB: u32 = 20;
const MMSYSERR_MOREDATA: u32 = 21;
const WAVERR_BADFORMAT: u32 = 32;
const WAVERR_STILLPLAYING: u32 = 33;
const WAVERR_UNPREPARED: u32 = 34;
const WAVERR_SYNC: u32 = 35;
const WAVERR_LASTERROR: u32 = 35;

// --- mmsystem.h: open flags and callback types ---
const CALLBACK_TYPEMASK: u32 = 0x0007_0000;
const CALLBACK_NULL: u32 = 0x0000_0000;
const CALLBACK_WINDOW: u32 = 0x0001_0000;
const CALLBACK_TASK: u32 = 0x0002_0000;
const CALLBACK_THREAD: u32 = CALLBACK_TASK;
const CALLBACK_FUNCTION: u32 = 0x0003_0000;
const CALLBACK_EVENT: u32 = 0x0005_0000;
const WAVE_FORMAT_QUERY: u32 = 0x0001;
const WAVE_ALLOWSYNC: u32 = 0x0002;
const WAVE_MAPPED: u32 = 0x0004;
const WAVE_FORMAT_DIRECT: u32 = 0x0008;
const WAVE_FORMAT_DIRECT_QUERY: u32 = WAVE_FORMAT_DIRECT | WAVE_FORMAT_QUERY;
const WAVE_MAPPED_DEFAULT_COMMUNICATION_DEVICE: u32 = 0x0010;
const WAVE_MAPPER: u32 = 0xFFFF_FFFF;

// --- mmsystem.h: messages ---
const MM_WOM_OPEN: u32 = 0x3BB;
const MM_WOM_CLOSE: u32 = 0x3BC;
const MM_WOM_DONE: u32 = 0x3BD;
const WOM_OPEN: u32 = MM_WOM_OPEN;
const WOM_CLOSE: u32 = MM_WOM_CLOSE;
const WOM_DONE: u32 = MM_WOM_DONE;

// --- mmsystem.h: WAVEHDR dwFlags ---
const WHDR_DONE: u32 = 1;
const WHDR_PREPARED: u32 = 2;
const WHDR_BEGINLOOP: u32 = 4;
const WHDR_ENDLOOP: u32 = 8;
const WHDR_INQUEUE: u32 = 16;

// --- mmsystem.h: capabilities ---
const WAVECAPS_PITCH: u32 = 1;
const WAVECAPS_PLAYBACKRATE: u32 = 2;
const WAVECAPS_VOLUME: u32 = 4;
const WAVECAPS_LRVOLUME: u32 = 8;
const WAVECAPS_SYNC: u32 = 16;
const WAVECAPS_SAMPLEACCURATE: u32 = 32;

// --- mmsystem.h: MMTIME wType ---
const TIME_MS: u32 = 1;
const TIME_SAMPLES: u32 = 2;
const TIME_BYTES: u32 = 4;
const TIME_SMPTE: u32 = 8;
const TIME_MIDI: u32 = 16;
const TIME_TICKS: u32 = 32;

// --- mmreg.h ---
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_ALAW: u16 = 6;
const WAVE_FORMAT_MULAW: u16 = 7;
const WAVE_FORMAT_ADPCM: u16 = 2;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const SPEAKER_FRONT_LEFT: u32 = 1;
const SPEAKER_FRONT_RIGHT: u32 = 2;
const SPEAKER_FRONT_CENTER: u32 = 4;

// --- winbase.h / winerror.h ---
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const INFINITE: u32 = 0xFFFF_FFFF;

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
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

/// KSDATAFORMAT_SUBTYPE_PCM (ksmedia.h)
const SUBTYPE_PCM: GUID = GUID { d1: 1, d2: 0, d3: 0x10, d4: [0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71] };
/// KSDATAFORMAT_SUBTYPE_IEEE_FLOAT (ksmedia.h)
const SUBTYPE_FLOAT: GUID = GUID { d1: 3, d2: 0, d3: 0x10, d4: [0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71] };

#[repr(C, packed(1))]
#[derive(Clone, Copy)]
struct WAVEFORMATEXTENSIBLE {
    Format: WAVEFORMATEX,
    wValidBitsPerSample: u16,
    dwChannelMask: u32,
    SubFormat: GUID,
}

#[repr(C)]
struct WAVEHDR {
    lpData: *mut u8,
    dwBufferLength: u32,
    dwBytesRecorded: u32,
    dwUser: usize,
    dwFlags: u32,
    dwLoops: u32,
    lpNext: *mut WAVEHDR,
    reserved: usize,
}

#[repr(C)]
struct WAVEOUTCAPSW {
    wMid: u16,
    wPid: u16,
    vDriverVersion: u32,
    szPname: [u16; 32],
    dwFormats: u32,
    wChannels: u16,
    wReserved1: u16,
    dwSupport: u32,
}

#[repr(C)]
struct WAVEOUTCAPSA {
    wMid: u16,
    wPid: u16,
    vDriverVersion: u32,
    szPname: [u8; 32],
    dwFormats: u32,
    wChannels: u16,
    wReserved1: u16,
    dwSupport: u32,
}

/// MMTIME (mmsystem.h, packed to 1 byte): 12 bytes, the type then an 8-byte union.
#[repr(C, packed(1))]
#[derive(Clone, Copy)]
struct MMTIME {
    wType: u32,
    u: [u8; 8],
}

impl MMTIME {
    fn new(t: u32) -> MMTIME {
        MMTIME { wType: t, u: [0xAA; 8] }
    }
    fn typ(&self) -> u32 {
        self.wType
    }
    fn dw(&self) -> u32 {
        u32::from_le_bytes([self.u[0], self.u[1], self.u[2], self.u[3]])
    }
}

#[link(name = "winmm", kind = "raw-dylib")]
unsafe extern "system" {
    fn waveOutGetNumDevs() -> u32;
    fn waveOutGetDevCapsW(id: usize, caps: *mut WAVEOUTCAPSW, cb: u32) -> u32;
    fn waveOutGetDevCapsA(id: usize, caps: *mut WAVEOUTCAPSA, cb: u32) -> u32;
    fn waveOutOpen(h: *mut HWAVEOUT, id: u32, fmt: *const WAVEFORMATEX, cb: usize, inst: usize, flags: u32) -> u32;
    fn waveOutClose(h: HWAVEOUT) -> u32;
    fn waveOutPrepareHeader(h: HWAVEOUT, hdr: *mut WAVEHDR, cb: u32) -> u32;
    fn waveOutUnprepareHeader(h: HWAVEOUT, hdr: *mut WAVEHDR, cb: u32) -> u32;
    fn waveOutWrite(h: HWAVEOUT, hdr: *mut WAVEHDR, cb: u32) -> u32;
    fn waveOutPause(h: HWAVEOUT) -> u32;
    fn waveOutRestart(h: HWAVEOUT) -> u32;
    fn waveOutReset(h: HWAVEOUT) -> u32;
    fn waveOutBreakLoop(h: HWAVEOUT) -> u32;
    fn waveOutGetPosition(h: HWAVEOUT, t: *mut MMTIME, cb: u32) -> u32;
    fn waveOutGetVolume(h: usize, v: *mut u32) -> u32;
    fn waveOutSetVolume(h: usize, v: u32) -> u32;
    fn waveOutGetPitch(h: HWAVEOUT, v: *mut u32) -> u32;
    fn waveOutSetPitch(h: HWAVEOUT, v: u32) -> u32;
    fn waveOutGetPlaybackRate(h: HWAVEOUT, v: *mut u32) -> u32;
    fn waveOutSetPlaybackRate(h: HWAVEOUT, v: u32) -> u32;
    fn waveOutGetID(h: HWAVEOUT, id: *mut u32) -> u32;
    fn waveOutGetErrorTextW(e: u32, text: *mut u16, cch: u32) -> u32;
    fn waveOutGetErrorTextA(e: u32, text: *mut u8, cch: u32) -> u32;
    fn waveOutMessage(h: HWAVEOUT, msg: u32, p1: usize, p2: usize) -> u32;
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
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn CreateThread(
        attr: *const c_void,
        stack: usize,
        f: extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> usize;
}

// ---------------------------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------------------------

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

/// `say!(t, "name {} {:#x}", a, b)`: one observation line.
macro_rules! say {
    ($t:expr, $($arg:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($arg)*);
        $t.line(b.as_str());
    }};
}

/// Name of an MMRESULT as the header spells it (for readable lines).
fn mm(r: u32) -> &'static str {
    match r {
        0 => "MMSYSERR_NOERROR",
        1 => "MMSYSERR_ERROR",
        2 => "MMSYSERR_BADDEVICEID",
        3 => "MMSYSERR_NOTENABLED",
        4 => "MMSYSERR_ALLOCATED",
        5 => "MMSYSERR_INVALHANDLE",
        6 => "MMSYSERR_NODRIVER",
        7 => "MMSYSERR_NOMEM",
        8 => "MMSYSERR_NOTSUPPORTED",
        9 => "MMSYSERR_BADERRNUM",
        10 => "MMSYSERR_INVALFLAG",
        11 => "MMSYSERR_INVALPARAM",
        12 => "MMSYSERR_HANDLEBUSY",
        13 => "MMSYSERR_INVALIDALIAS",
        20 => "MMSYSERR_NODRIVERCB",
        21 => "MMSYSERR_MOREDATA",
        32 => "WAVERR_BADFORMAT",
        33 => "WAVERR_STILLPLAYING",
        34 => "WAVERR_UNPREPARED",
        35 => "WAVERR_SYNC",
        _ => "other",
    }
}

fn msgname(m: u32) -> &'static str {
    match m {
        MM_WOM_OPEN => "WOM_OPEN",
        MM_WOM_CLOSE => "WOM_CLOSE",
        MM_WOM_DONE => "WOM_DONE",
        _ => "other",
    }
}

/// `r` must be `want`: records `ok name` or `not ok name`; and writes `name = <MMRESULT name>` as an observation.
fn rc(t: &mut Test, name: &str, r: u32, want: u32) -> bool {
    say!(t, "{} = {} ({})", name, mm(r), r);
    t.expect(name, r as i64, want as i64)
}

// ---------------------------------------------------------------------------------------------
// Formats and buffers
// ---------------------------------------------------------------------------------------------

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

const HDR_SIZE: u32 = core::mem::size_of::<WAVEHDR>() as u32;

/// 100 ms at 48000 Hz, stereo, 16 bit: 4800 frames of silence.
const BUF_BYTES: usize = 19200;
const NBUF: usize = 6;
static mut AUDIO: [[u8; BUF_BYTES]; NBUF] = [[0; BUF_BYTES]; NBUF];
static mut HDRS: [WAVEHDR; NBUF] = [const {
    WAVEHDR {
        lpData: core::ptr::null_mut(),
        dwBufferLength: 0,
        dwBytesRecorded: 0,
        dwUser: 0,
        dwFlags: 0,
        dwLoops: 0,
        lpNext: core::ptr::null_mut(),
        reserved: 0,
    }
}; NBUF];

/// Header number `i` (0..NBUF) pointing at its own `bytes` bytes of silence; flags cleared.
fn hdr(i: usize, bytes: usize) -> *mut WAVEHDR {
    // SAFETY: single-threaded setup of statics that only the test and the (serialised) driver touch; `i` and `bytes`
    // are within the arrays.
    unsafe {
        let h = core::ptr::addr_of_mut!(HDRS[i]);
        (*h).lpData = core::ptr::addr_of_mut!(AUDIO[i]) as *mut u8;
        (*h).dwBufferLength = bytes as u32;
        (*h).dwBytesRecorded = 0;
        (*h).dwUser = 0;
        (*h).dwFlags = 0;
        (*h).dwLoops = 0;
        (*h).lpNext = core::ptr::null_mut();
        (*h).reserved = 0;
        h
    }
}

fn flags(h: *mut WAVEHDR) -> u32 {
    // SAFETY: `h` points into HDRS.
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*h).dwFlags)) }
}

/// Which header of HDRS a pointer is (255 if none).
fn hdr_index(p: usize) -> usize {
    for i in 0..NBUF {
        // SAFETY: address-of a static element only.
        if unsafe { core::ptr::addr_of!(HDRS[i]) } as usize == p {
            return i;
        }
    }
    255
}

fn fl(f: u32) -> Buf<64> {
    let mut b = Buf::<64>::new();
    let names = [(1, "DONE"), (2, "PREPARED"), (4, "BEGINLOOP"), (8, "ENDLOOP"), (16, "INQUEUE")];
    let mut first = true;
    for (bit, n) in names {
        if f & bit != 0 {
            if !first {
                let _ = write!(b, "|");
            }
            first = false;
            let _ = write!(b, "{}", n);
        }
    }
    if f & !0x1f != 0 {
        if !first {
            let _ = write!(b, "|");
        }
        first = false;
        let _ = write!(b, "{:#x}", f & !0x1f);
    }
    if first {
        let _ = write!(b, "0");
    }
    b
}

// ---------------------------------------------------------------------------------------------
// Callbacks: what arrives, in which order, on which thread
// ---------------------------------------------------------------------------------------------

const LOG_MAX: usize = 64;
/// One word per callback: msg in bits 0..16, header index in 16..24, flags-at-callback in 24..32, same-thread-as-main
/// in bit 32, `instance matches` in bit 33, dwParam2 zero in bit 34.
static LOG: [AtomicUsize; LOG_MAX] = [const { AtomicUsize::new(0) }; LOG_MAX];
static LOGN: AtomicU32 = AtomicU32::new(0);
static MAIN_TID: AtomicU32 = AtomicU32::new(0);
static EXPECT_INST: AtomicUsize = AtomicUsize::new(0);

fn log_reset() {
    LOGN.store(0, Ordering::SeqCst);
    MAIN_TID.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
}

fn log_push(msg: u32, p1: usize, p2: usize, inst: usize) {
    let idx = if msg == MM_WOM_DONE { hdr_index(p1) } else { 254 };
    let f = if msg == MM_WOM_DONE && idx < NBUF {
        // SAFETY: p1 is the header the driver completed (it is in HDRS).
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*(p1 as *const WAVEHDR)).dwFlags)) }
    } else {
        0
    };
    let same = unsafe { GetCurrentThreadId() } == MAIN_TID.load(Ordering::SeqCst);
    let mut w = (msg as usize & 0xffff) | (idx << 16) | ((f as usize & 0xff) << 24);
    if same {
        w |= 1 << 32;
    }
    if inst == EXPECT_INST.load(Ordering::SeqCst) {
        w |= 1 << 33;
    }
    if p2 == 0 {
        w |= 1 << 34;
    }
    let n = LOGN.fetch_add(1, Ordering::SeqCst) as usize;
    if n < LOG_MAX {
        LOG[n].store(w, Ordering::SeqCst);
    }
}

/// CALLBACK_FUNCTION target: `void CALLBACK waveOutProc(HWAVEOUT, UINT, DWORD_PTR inst, DWORD_PTR p1, DWORD_PTR p2)`.
extern "system" fn wave_proc(_h: HWAVEOUT, msg: u32, inst: usize, p1: usize, p2: usize) {
    log_push(msg, p1, p2, inst);
}

fn log_count() -> usize {
    (LOGN.load(Ordering::SeqCst) as usize).min(LOG_MAX)
}

fn count_msg(msg: u32) -> usize {
    let mut n = 0;
    for i in 0..log_count() {
        if (LOG[i].load(Ordering::SeqCst) & 0xffff) as u32 == msg {
            n += 1;
        }
    }
    n
}

/// Print the callback log: `cb <n> WOM_DONE hdr=<i> flags=<..> thread=<main|other> inst=<ok|bad> p2=<0|nonzero>`.
fn log_print(t: &mut Test, label: &str) {
    let n = log_count();
    say!(t, "{} callbacks = {}", label, n);
    for i in 0..n {
        let w = LOG[i].load(Ordering::SeqCst);
        let msg = (w & 0xffff) as u32;
        let idx = (w >> 16) & 0xff;
        let f = ((w >> 24) & 0xff) as u32;
        let same = (w >> 32) & 1 != 0;
        let inst = (w >> 33) & 1 != 0;
        let p2z = (w >> 34) & 1 != 0;
        let b = fl(f);
        if msg == MM_WOM_DONE {
            say!(
                t,
                "{} cb {} {} hdr={} flags={} thread={} inst={} p2={}",
                label,
                i,
                msgname(msg),
                idx,
                b.as_str(),
                if same { "main" } else { "other" },
                if inst { "ok" } else { "bad" },
                if p2z { "0" } else { "nonzero" }
            );
        } else {
            say!(
                t,
                "{} cb {} {} thread={} inst={} p2={}",
                label,
                i,
                msgname(msg),
                if same { "main" } else { "other" },
                if inst { "ok" } else { "bad" },
                if p2z { "0" } else { "nonzero" }
            );
        }
    }
}

/// Wait until `cond()` or `ms` milliseconds; true if `cond()` became true.
fn wait_for(ms: u32, mut cond: impl FnMut() -> bool) -> bool {
    // SAFETY: plain calls.
    let start = unsafe { GetTickCount64() };
    loop {
        if cond() {
            return true;
        }
        if unsafe { GetTickCount64() } - start >= ms as u64 {
            return false;
        }
        unsafe { Sleep(2) };
    }
}

fn new_event() -> usize {
    // SAFETY: auto-reset... no: manual-reset, initially not signalled, unnamed.
    unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) }
}

fn evt_wait(e: usize, ms: u32) -> bool {
    // SAFETY: `e` is an event handle.
    unsafe { WaitForSingleObject(e, ms) == WAIT_OBJECT_0 }
}

/// Open the default device (device 0) with a callback; returns (MMRESULT, handle).
fn open_with(id: u32, f: &WAVEFORMATEX, cb: usize, inst: usize, flags: u32) -> (u32, HWAVEOUT) {
    let mut h: HWAVEOUT = 0;
    // SAFETY: `h` and `f` are valid for the call.
    let r = unsafe { waveOutOpen(&mut h, id, f, cb, inst, flags) };
    (r, h)
}

/// Open device 0 as 48000 Hz stereo 16 bit with a function callback logging into LOG.
fn open_fn(inst: usize) -> (u32, HWAVEOUT) {
    EXPECT_INST.store(inst, Ordering::SeqCst);
    let f = pcm(48000, 2, 16);
    open_with(0, &f, wave_proc as usize, inst, CALLBACK_FUNCTION)
}

/// Open device 0 as 48000 Hz stereo 16 bit with no callback.
fn open_plain() -> (u32, HWAVEOUT) {
    let f = pcm(48000, 2, 16);
    open_with(0, &f, 0, 0, CALLBACK_NULL)
}

fn prep(h: HWAVEOUT, p: *mut WAVEHDR) -> u32 {
    // SAFETY: `p` points into HDRS.
    unsafe { waveOutPrepareHeader(h, p, HDR_SIZE) }
}

fn unprep(h: HWAVEOUT, p: *mut WAVEHDR) -> u32 {
    // SAFETY: as above.
    unsafe { waveOutUnprepareHeader(h, p, HDR_SIZE) }
}

fn write(h: HWAVEOUT, p: *mut WAVEHDR) -> u32 {
    // SAFETY: as above.
    unsafe { waveOutWrite(h, p, HDR_SIZE) }
}

fn pos(h: HWAVEOUT, ty: u32) -> (u32, u32, u32) {
    let mut m = MMTIME::new(ty);
    // SAFETY: valid MMTIME.
    let r = unsafe { waveOutGetPosition(h, &mut m, 12) };
    (r, m.typ(), m.dw())
}

fn device_count() -> u32 {
    // SAFETY: no arguments.
    unsafe { waveOutGetNumDevs() }
}

fn hp(i: usize) -> *mut WAVEHDR {
    // SAFETY: address of an element of the static header array.
    unsafe { core::ptr::addr_of_mut!(HDRS[i]) }
}

/// Header `i` set up for `bytes` bytes of silence, prepared and written; returns the write result.
fn queue(h: HWAVEOUT, i: usize, bytes: usize) -> u32 {
    let p = hdr(i, bytes);
    let r = prep(h, p);
    if r != 0 {
        return r;
    }
    write(h, p)
}

/// Header `i` set up and prepared only.
fn prepared(h: HWAVEOUT, i: usize, bytes: usize) -> u32 {
    let p = hdr(i, bytes);
    prep(h, p)
}

fn is_done(i: usize) -> bool {
    flags(hp(i)) & WHDR_DONE != 0
}

/// Wait up to 5 s for headers 0..n to be done.
fn wait_done(n: usize) -> bool {
    wait_for(5000, || (0..n).all(is_done))
}

/// Pause long enough for anything that is going to happen (and, with `ms` small, to check that nothing does).
fn settle(ms: u32) {
    wait_for(ms, || false);
}

/// Ascii text of a UTF-16 name (non-ascii shown as '?').
fn name_of(u: &[u16]) -> Buf<64> {
    let mut b = Buf::<64>::new();
    for &c in u {
        if c == 0 {
            break;
        }
        let _ = write!(b, "{}", if c < 128 { c as u8 as char } else { '?' });
    }
    b
}

fn name_of_a(u: &[u8]) -> Buf<64> {
    let mut b = Buf::<64>::new();
    for &c in u {
        if c == 0 {
            break;
        }
        let _ = write!(b, "{}", if c < 128 { c as char } else { '?' });
    }
    b
}

fn has_device(t: &mut Test) -> bool {
    let n = device_count();
    t.check("has output device", n >= 1, n as i64)
}

fn close(h: HWAVEOUT) -> u32 {
    // SAFETY: plain call.
    unsafe { waveOutClose(h) }
}

fn zeroed_caps_w() -> WAVEOUTCAPSW {
    // SAFETY: plain-old-data, all zero is valid.
    unsafe { core::mem::zeroed() }
}

fn caps_w(id: usize) -> (u32, WAVEOUTCAPSW) {
    let mut c = zeroed_caps_w();
    // SAFETY: valid buffer of the stated size.
    let r = unsafe { waveOutGetDevCapsW(id, &mut c, core::mem::size_of::<WAVEOUTCAPSW>() as u32) };
    (r, c)
}

/// Query, open, play 50 ms of silence, check the position in bytes and samples after the buffer is done, close.
/// `align` is the format's bytes per frame.
fn play_check(t: &mut Test, label: &str, f: *const WAVEFORMATEX, rate: u32, align: u32) {
    let frames = rate / 20;
    let bytes = (frames * align) as usize;
    let mut h: HWAVEOUT = 0;
    // SAFETY: valid format and handle pointers.
    let q = unsafe { waveOutOpen(&mut h, 0, f, 0, 0, WAVE_FORMAT_QUERY) };
    say!(t, "{}: query {}", label, mm(q));
    t.check("query accepts the format", q == MMSYSERR_NOERROR, q as i64);
    t.check("query leaves the handle alone", h == 0, h as i64);
    // SAFETY: as above.
    let r = unsafe { waveOutOpen(&mut h, 0, f, 0, 0, CALLBACK_NULL) };
    say!(t, "{}: open {}", label, mm(r));
    t.check("open accepts the format", r == MMSYSERR_NOERROR && h != 0, r as i64);
    if r != 0 {
        return;
    }
    let p = hdr(0, bytes);
    t.expect("prepare", prep(h, p) as i64, 0);
    t.expect("write", write(h, p) as i64, 0);
    let done = wait_done(1);
    t.check("buffer done", done, 0);
    let mut b = 0u32;
    let mut s = 0u32;
    wait_for(2000, || {
        b = pos(h, TIME_BYTES).2;
        b == bytes as u32
    });
    let (rb, tb, vb) = pos(h, TIME_BYTES);
    let (rs, ts, vs) = pos(h, TIME_SAMPLES);
    s += vs;
    say!(t, "{}: bytes position type {} equals written {}", label, tb, vb == bytes as u32);
    say!(t, "{}: samples position type {} equals frames {}", label, ts, s == frames);
    t.check("byte position equals the bytes written", rb == 0 && tb == TIME_BYTES && vb == bytes as u32, vb as i64);
    t.check("sample position equals the frames written", rs == 0 && ts == TIME_SAMPLES && vs == frames, vs as i64);
    t.expect("unprepare", unprep(h, p) as i64, 0);
    t.expect("close", close(h) as i64, 0);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    if !has_device(t) {
        return;
    }
    log_reset();
    let (r, h) = open_fn(0xa1);
    rc(t, "open", r, MMSYSERR_NOERROR);
    for i in 0..3 {
        t.expect("queue", queue(h, i, 19200) as i64, 0);
    }
    settle(30);
    t.check("not finished yet", count_msg(WOM_DONE) == 0, count_msg(WOM_DONE) as i64);
    // SAFETY: plain call.
    rc(t, "reset", unsafe { waveOutReset(h) }, MMSYSERR_NOERROR);
    t.expect("all three WOM_DONE have arrived when reset returns", count_msg(WOM_DONE) as i64, 3);
    for i in 0..3 {
        t.expect("flags", flags(hp(i)) as i64, (WHDR_DONE | WHDR_PREPARED) as i64);
    }
    t.expect("position is 0 after the reset", pos(h, TIME_BYTES).2 as i64, 0);
    settle(100);
    log_print(t, "reset");
    t.expect("no further callback", log_count() as i64, 4);
    for i in 0..3 {
        let x = LOG[1 + i].load(Ordering::SeqCst);
        t.expect("in the order written", ((x >> 16) & 0xff) as i64, i as i64);
    }
    for i in 0..3 {
        rc(t, "unprepare", unprep(h, hp(i)), MMSYSERR_NOERROR);
    }
    close(h);
}
