//! `waveOut*`: wave output on `\Device\PolyAudio` (ADR 0244), as the sheets in
//! `specs/windows/winmm.dll/` describe it. The decisions (formats, header flags, the play order
//! and loops, texts, capabilities) are `poly_kernelbase::waveout`; this file keeps the handles and
//! talks to the device and the callbacks.
//!
//! - **A handle** is a number (never reused) for a [`Wave`] in one table under one lock. Each has
//!   its own stream on the device, in the format it was opened with, with a buffer of 100 ms.
//! - **Its worker thread** feeds the queued headers to the stream as it has room, notices which
//!   are done from the stream's played-frame count, marks them done and calls back (`WOM_DONE`
//!   comes on that thread, in order). It sleeps on a `WAIT` request for the next header's end (or
//!   10 ms of frames, to refill), and on an event the application's calls set.
//! - **Open and close** call back on the calling thread (`WOM_OPEN`, `WOM_CLOSE`). **Reset** has
//!   the worker give every queued header back, with its `WOM_DONE`, before it returns.
//! - **No lock is held** while a callback runs: a callback may write and unprepare.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::ptr::{null, null_mut, read_volatile, write_volatile};
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicUsize, Ordering::SeqCst};
use poly_audio::nt::{self as wire, DeviceInfo, OpenReq, StreamInfo};
use poly_kernelbase::waveout::{self as wo, Player, Queued, Unprepare, WaveFormat};

type Handle = *mut c_void;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtCreateFile(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        iosb: *mut IoStatusBlock,
        allocation: *const i64,
        file_attributes: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: *const u8,
        ea_len: u32,
    ) -> u32;
    fn NtDeviceIoControlFile(
        handle: Handle,
        event: Handle,
        apc: *const u8,
        apc_context: *const u8,
        iosb: *mut IoStatusBlock,
        code: u32,
        input: *const u8,
        input_len: u32,
        output: *mut u8,
        output_len: u32,
    ) -> u32;
    fn NtClose(handle: Handle) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn AcquireSRWLockExclusive(lock: *mut usize);
    fn ReleaseSRWLockExclusive(lock: *mut usize);
    fn CreateEventW(attributes: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, ms: u32) -> u32;
    fn WaitForMultipleObjects(count: u32, handles: *const Handle, all: i32, ms: u32) -> u32;
    fn CreateThread(
        attributes: *const u8,
        stack: usize,
        start: extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> Handle;
    fn GetCurrentThreadId() -> u32;
    fn LoadLibraryW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const c_void;
}

/// `IO_STATUS_BLOCK` (`wdm.h`).
#[repr(C)]
#[derive(Debug, Default)]
struct IoStatusBlock {
    status: usize,
    information: usize,
}

/// `UNICODE_STRING` (`ntdef.h`).
#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

/// `OBJECT_ATTRIBUTES` (`ntdef.h`).
#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: Handle,
    name: *const UnicodeString,
    attributes: u32,
    security: *const u8,
    qos: *const u8,
}

/// `WAVEHDR` (`mmeapi.h`): 48 bytes (32 in a 32-bit process).
#[repr(C)]
pub struct WaveHdr {
    data: *const u8,
    len: u32,
    recorded: u32,
    user: usize,
    flags: u32,
    loops: u32,
    next: usize,
    reserved: usize,
}

const HDR_SIZE: u32 = core::mem::size_of::<WaveHdr>() as u32;
const STATUS_SUCCESS: u32 = 0;
const STATUS_PENDING: u32 = 0x103;
const WAIT_OBJECT_0: u32 = 0;
const INFINITE: u32 = 0xFFFF_FFFF;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_OPEN: u32 = 1;
const OBJ_CASE_INSENSITIVE: u32 = 0x40;
/// The stream's buffer and the worker's refill step, in milliseconds of the stream.
const BUFFER_MS: u32 = 100;
const STEP_MS: u32 = 10;
/// How long the worker sleeps at most without being woken.
const SAFETY_MS: u32 = 50;
/// The first handle number: far from small made-up values.
const FIRST_HANDLE: usize = 0x0000_5741_0000;

// ----- the lock and the table

/// One lock (an SRW lock) over every handle's state.
struct Locked<T> {
    lock: UnsafeCell<usize>,
    value: UnsafeCell<T>,
}

// SAFETY: `value` is only reached through `with`, under the SRW lock.
unsafe impl<T> Sync for Locked<T> {}

impl<T> Locked<T> {
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        // SAFETY: an SRW lock word, zero-initialized (SRWLOCK_INIT); the value is reached only
        // while the lock is held, and `f` does not call anything that takes it again.
        unsafe {
            AcquireSRWLockExclusive(self.lock.get());
            let r = f(&mut *self.value.get());
            ReleaseSRWLockExclusive(self.lock.get());
            r
        }
    }
}

struct Table {
    waves: Vec<Box<Wave>>,
}

static TABLE: Locked<Table> = Locked { lock: UnsafeCell::new(0), value: UnsafeCell::new(Table { waves: Vec::new() }) };
static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(FIRST_HANDLE);
/// The process's volume (`waveOutSetVolume`): left in the low 16 bits.
static VOLUME: AtomicU32 = AtomicU32::new(0xFFFF_FFFF);
/// Whether the machine has a sound card: 0 not asked yet, 1 yes, 2 no.
static PRESENT: AtomicU8 = AtomicU8::new(0);

/// How a handle calls back.
#[derive(Clone, Copy)]
struct Callback {
    kind: u32,
    target: usize,
    instance: usize,
}

struct Wave {
    handle: usize,
    /// The id it was opened with (0, or `WAVE_MAPPER`).
    device: u32,
    file: Handle,
    block: u32,
    rate: u32,
    callback: Callback,
    player: Player,
    /// Bytes fed but not yet a whole frame.
    carry: Vec<u8>,
    paused: bool,
    /// The played-frame count when it was paused: the position stays there until the restart
    /// (the device still plays out the periods it was handed).
    pause_pos: Option<u64>,
    /// The stream was started (and not stopped since).
    running: bool,
    closing: bool,
    reset: bool,
    /// The worker's thread and id; its wake event (auto-reset), the `WAIT` request's event
    /// (manual) and the event a reset waits for.
    thread: Handle,
    tid: u32,
    wake: Handle,
    io_event: Handle,
    reset_done: Handle,
    iosb: IoStatusBlock,
    wait_out: [u8; 8],
    wait_pending: bool,
}

// SAFETY: a Wave's raw handles are kernel handles, usable from any thread; its fields are
// reached under the table's lock.
unsafe impl Send for Wave {}

fn find(t: &mut Table, handle: usize) -> Option<&mut Wave> {
    t.waves.iter_mut().map(|b| &mut **b).find(|w| w.handle == handle)
}

// ----- the device

/// `\Device\PolyAudio` in UTF-16.
const DEVICE_NAME: [u16; 17] = {
    let s = wire::DEVICE_NAME.as_bytes();
    let mut w = [0u16; 17];
    let mut i = 0;
    while i < 17 {
        w[i] = s[i] as u16;
        i += 1;
    }
    w
};

/// A new handle on the device (overlapped: a `WAIT` may be pending).
fn open_device() -> Option<Handle> {
    let name = UnicodeString { length: 34, maximum_length: 34, buffer: DEVICE_NAME.as_ptr() };
    let attrs = ObjectAttributes {
        length: core::mem::size_of::<ObjectAttributes>() as u32,
        root: null_mut(),
        name: &name,
        attributes: OBJ_CASE_INSENSITIVE,
        security: null(),
        qos: null(),
    };
    let mut h: Handle = null_mut();
    let mut iosb = IoStatusBlock::default();
    // SAFETY: every pointer is to a live local; no EA buffer.
    let s = unsafe {
        NtCreateFile(
            &mut h,
            GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE,
            &attrs,
            &mut iosb,
            null(),
            0,
            3,
            FILE_OPEN,
            0,
            null(),
            0,
        )
    };
    (s == STATUS_SUCCESS).then_some(h)
}

/// A request that finishes at once: its status and `Information`.
fn ioctl(h: Handle, code: u32, input: &[u8], out: &mut [u8]) -> (u32, usize) {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: the buffers and the status block outlive the call; these requests never pend.
    let s = unsafe {
        NtDeviceIoControlFile(
            h,
            null_mut(),
            null(),
            null(),
            &mut iosb,
            code,
            input.as_ptr(),
            input.len() as u32,
            out.as_mut_ptr(),
            out.len() as u32,
        )
    };
    (s, iosb.information)
}

fn query(h: Handle) -> StreamInfo {
    let mut out = [0u8; StreamInfo::SIZE];
    ioctl(h, wire::QUERY, &[], &mut out);
    StreamInfo::decode(&out).unwrap_or_default()
}

/// Does the machine have a sound card (asked once)?
fn present() -> bool {
    match PRESENT.load(SeqCst) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    let yes = open_device().is_some_and(|h| {
        let mut out = [0u8; DeviceInfo::SIZE];
        let ok = ioctl(h, wire::DEVICE, &[], &mut out).0 == STATUS_SUCCESS;
        // SAFETY: our handle.
        unsafe { NtClose(h) };
        ok
    });
    PRESENT.store(if yes { 1 } else { 2 }, SeqCst);
    yes
}

/// Is `id` a device id (0 or the mapper), with a card there?
fn is_device_id(id: usize) -> bool {
    (id == 0 || id == wo::WAVE_MAPPER as usize || id == usize::MAX) && present()
}

fn set_gain(h: Handle, volume: u32) {
    let mut g = [0u8; 8];
    g[..4].copy_from_slice(&wo::gain(volume as u16).to_le_bytes());
    g[4..].copy_from_slice(&wo::gain((volume >> 16) as u16).to_le_bytes());
    ioctl(h, wire::SET_GAIN, &g, &mut []);
}

// ----- callbacks

type WaveOutProc = extern "system" fn(usize, u32, usize, usize, usize);
type PostMessageW = extern "system" fn(usize, u32, usize, isize) -> i32;
type PostThreadMessageW = extern "system" fn(u32, u32, usize, isize) -> i32;
type IsWindow = extern "system" fn(usize) -> i32;

/// `user32.dll`'s function `name` (NUL-terminated), loaded when a window or thread callback
/// first needs it.
fn user32(name: &[u8]) -> *const c_void {
    static MODULE: AtomicUsize = AtomicUsize::new(0);
    let mut m = MODULE.load(SeqCst);
    if m == 0 {
        let lib: Vec<u16> = "user32.dll\0".encode_utf16().collect();
        // SAFETY: a NUL-terminated name.
        m = unsafe { LoadLibraryW(lib.as_ptr()) } as usize;
        MODULE.store(m, SeqCst);
    }
    if m == 0 {
        return null();
    }
    // SAFETY: a loaded module and a NUL-terminated name.
    unsafe { GetProcAddress(m as Handle, name.as_ptr()) }
}

fn is_window(hwnd: usize) -> bool {
    let f = user32(b"IsWindow\0");
    if f.is_null() {
        return false;
    }
    // SAFETY: `IsWindow`'s signature (`winuser.h`).
    let is = unsafe { core::mem::transmute::<*const c_void, IsWindow>(f) };
    is(hwnd) != 0
}

/// Tell the application `msg` about `handle` (`param` is the header for `WOM_DONE`).
fn notify(cb: Callback, handle: usize, msg: u32, param: usize) {
    match cb.kind {
        wo::CALLBACK_FUNCTION if cb.target != 0 => {
            // SAFETY: the application's `waveOutProc`.
            let f = unsafe { core::mem::transmute::<usize, WaveOutProc>(cb.target) };
            f(handle, msg, cb.instance, param, 0);
        }
        wo::CALLBACK_EVENT => {
            // SAFETY: the application's event handle (a bad one only fails).
            unsafe { SetEvent(cb.target as Handle) };
        }
        wo::CALLBACK_WINDOW => {
            let f = user32(b"PostMessageW\0");
            if !f.is_null() {
                // SAFETY: `PostMessageW`'s signature.
                let post = unsafe { core::mem::transmute::<*const c_void, PostMessageW>(f) };
                post(cb.target, msg, handle, param as isize);
            }
        }
        wo::CALLBACK_THREAD => {
            let f = user32(b"PostThreadMessageW\0");
            if !f.is_null() {
                // SAFETY: `PostThreadMessageW`'s signature.
                let post = unsafe { core::mem::transmute::<*const c_void, PostThreadMessageW>(f) };
                post(cb.target as u32, msg, handle, param as isize);
            }
        }
        _ => {}
    }
}

// ----- headers

fn flags_of(h: *mut WaveHdr) -> u32 {
    // SAFETY: the application's header, valid from prepare to unprepare.
    unsafe { read_volatile(&raw const (*h).flags) }
}

fn set_flags(h: *mut WaveHdr, flags: u32) {
    // SAFETY: as in `flags_of`.
    unsafe { write_volatile(&raw mut (*h).flags, flags) };
}

/// Mark the header done (before its notification).
fn finish(h: usize) {
    let h = h as *mut WaveHdr;
    // SAFETY: a header that was queued: the application keeps it until it is done.
    unsafe { write_volatile(&raw mut (*h).recorded, 0) };
    set_flags(h, wo::finished(flags_of(h)));
}

// ----- the worker

/// Feed what fits (under the lock).
fn feed(w: &mut Wave, info: &StreamInfo) {
    let block = w.block as usize;
    let room = (info.buffer_frames - info.queued) as usize * block;
    let mut room = room.saturating_sub(w.carry.len()) as u32;
    while room > 0 {
        let Some(p) = w.player.next_piece(room) else { break };
        let h = p.id as *const WaveHdr;
        // SAFETY: a queued header and its data, which the application keeps until it is done.
        let data = unsafe { (*h).data };
        if !data.is_null() {
            // SAFETY: `offset + len` lies inside the header's buffer (`dwBufferLength`).
            let bytes = unsafe { core::slice::from_raw_parts(data.add(p.offset as usize), p.len as usize) };
            w.carry.extend_from_slice(bytes);
        } else {
            w.carry.resize(w.carry.len() + p.len as usize, 0);
        }
        w.player.fed_piece(p.len);
        room -= p.len;
    }
    let whole = w.carry.len() / block * block;
    if whole > 0 {
        let (s, taken) = ioctl(w.file, wire::WRITE, &w.carry[..whole], &mut []);
        if s == STATUS_SUCCESS {
            w.carry.drain(..taken * block);
        }
    }
    if !w.paused && !w.running && (info.queued > 0 || whole > 0) {
        ioctl(w.file, wire::START, &[], &mut []);
        w.running = true;
    }
}

/// Give every queued header back (under the lock): the stream stops and counts from 0.
fn reset_queue(w: &mut Wave) -> Vec<usize> {
    ioctl(w.file, wire::RESET, &[], &mut []);
    if w.wait_pending {
        // The reset cancelled the wait.
        // SAFETY: our event.
        unsafe { WaitForSingleObject(w.io_event, 200) };
        w.wait_pending = false;
    }
    w.running = false;
    w.paused = false;
    w.pause_pos = None;
    w.carry.clear();
    let ids = w.player.reset();
    for &id in &ids {
        finish(id);
    }
    ids
}

/// Ask for a wake-up once the stream played `target` frames (under the lock).
fn arm_wait(w: &mut Wave, target: u64) {
    let input = target.to_le_bytes();
    // SAFETY: `iosb` and `wait_out` live in the boxed Wave until the request has completed
    // (close waits for it); the event is ours.
    let s = unsafe {
        NtDeviceIoControlFile(
            w.file,
            w.io_event,
            null(),
            null(),
            &mut w.iosb,
            wire::WAIT,
            input.as_ptr(),
            8,
            w.wait_out.as_mut_ptr(),
            8,
        )
    };
    w.wait_pending = s == STATUS_PENDING;
}

extern "system" fn worker(param: *mut c_void) -> u32 {
    let handle = param as usize;
    loop {
        let mut done = Vec::new();
        let mut reset = false;
        let step = TABLE.with(|t| -> Option<(Callback, Handle, Handle, Handle, bool, bool)> {
            let w = find(t, handle)?;
            if w.closing {
                return None;
            }
            if w.reset {
                w.reset = false;
                reset = true;
                done = reset_queue(w);
            }
            if w.wait_pending {
                // SAFETY: our event.
                if unsafe { WaitForSingleObject(w.io_event, 0) } == WAIT_OBJECT_0 {
                    w.wait_pending = false;
                }
            }
            let mut info = query(w.file);
            feed(w, &info);
            info.played = played_of(w, &info);
            let played = info.played * u64::from(w.block);
            for id in w.player.done(played, w.block) {
                finish(id);
                done.push(id);
            }
            if w.player.is_empty() && w.running && info.queued == 0 && w.carry.is_empty() {
                ioctl(w.file, wire::STOP, &[], &mut []);
                w.running = false;
            }
            let mut again = false;
            if w.running && !w.paused && !w.wait_pending {
                let step = u64::from((w.rate * STEP_MS / 1000).max(1));
                let next = w.player.next_end().map(|e| e / u64::from(w.block)).filter(|&e| e > info.played);
                let target = next.unwrap_or(u64::MAX).min(info.played + step);
                arm_wait(w, target);
                // Already played that far: look again at once.
                again = !w.wait_pending;
            }
            Some((w.callback, w.wake, w.io_event, w.reset_done, w.wait_pending, again))
        });
        let Some((cb, wake, io_event, reset_done, waiting, again)) = step else { return 0 };
        for id in done {
            notify(cb, handle, wo::WOM_DONE, id);
        }
        if reset {
            if cb.kind == wo::CALLBACK_EVENT {
                notify(cb, handle, wo::WOM_DONE, 0);
            }
            // SAFETY: our event.
            unsafe { SetEvent(reset_done) };
        }
        if again {
            continue;
        }
        let events = [wake, io_event];
        let n = if waiting { 2 } else { 1 };
        // SAFETY: our events.
        unsafe { WaitForMultipleObjects(n, events.as_ptr(), 0, SAFETY_MS) };
    }
}

/// The played-frame count the application sees: held while paused.
fn played_of(w: &Wave, info: &StreamInfo) -> u64 {
    w.pause_pos.map_or(info.played, |p| p.min(info.played))
}

/// Wake the worker of `w`.
fn wake(w: &Wave) {
    // SAFETY: our event.
    unsafe { SetEvent(w.wake) };
}

// ----- the functions

/// `waveOutGetNumDevs`.
#[unsafe(no_mangle)]
pub extern "system" fn waveOutGetNumDevs() -> u32 {
    u32::from(present())
}

/// The caps of `id` (a device id or an open handle): whether it is the mapper.
fn caps_of(id: usize) -> Result<bool, u32> {
    if is_device_id(id) {
        return Ok(id != 0);
    }
    TABLE.with(|t| find(t, id).map(|_| false)).ok_or(wo::MMSYSERR_BADDEVICEID)
}

/// `waveOutGetDevCapsW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutGetDevCapsW(id: usize, caps: *mut u8, size: u32) -> u32 {
    if caps.is_null() {
        return wo::MMSYSERR_INVALPARAM;
    }
    let mapper = match caps_of(id) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if size == 0 {
        return wo::MMSYSERR_NOERROR;
    }
    if (size as usize) < wo::CAPS_W_SIZE {
        return wo::MMSYSERR_INVALPARAM;
    }
    let b = wo::caps(mapper, true);
    // SAFETY: the caller's buffer of at least 84 bytes.
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), caps, b.len()) };
    wo::MMSYSERR_NOERROR
}

/// `waveOutGetDevCapsA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutGetDevCapsA(id: usize, caps: *mut u8, size: u32) -> u32 {
    if caps.is_null() {
        return wo::MMSYSERR_INVALPARAM;
    }
    let mapper = match caps_of(id) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let b = wo::caps(mapper, false);
    let n = (size as usize).min(b.len());
    // SAFETY: the caller's buffer of `size` bytes.
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), caps, n) };
    wo::MMSYSERR_NOERROR
}

/// `waveOutOpen`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutOpen(
    out: *mut usize,
    device: u32,
    format: *const u8,
    callback: usize,
    instance: usize,
    flags: u32,
) -> u32 {
    if let Err(e) = wo::check_open_flags(flags) {
        return e;
    }
    let query = flags & wo::WAVE_FORMAT_QUERY != 0;
    if format.is_null() || (!query && out.is_null()) {
        return wo::MMSYSERR_INVALPARAM;
    }
    let kind = flags & wo::CALLBACK_TYPEMASK;
    if kind == wo::CALLBACK_WINDOW && !is_window(callback) {
        return wo::MMSYSERR_INVALPARAM;
    }
    let fail = |e: u32| {
        if !query {
            // SAFETY: the caller's handle variable.
            unsafe { *out = 0 };
        }
        e
    };
    if !(device == 0 || device == wo::WAVE_MAPPER) || !present() {
        return fail(wo::MMSYSERR_BADDEVICEID);
    }
    // The format: `WAVEFORMATEX`, and the extensible part when it says it has one.
    let mut bytes = [0u8; wo::WAVEFORMATEX_SIZE + wo::EXTENSIBLE_EXTRA as usize];
    // SAFETY: the caller's format, at least a `WAVEFORMATEX`.
    unsafe { core::ptr::copy_nonoverlapping(format, bytes.as_mut_ptr(), wo::WAVEFORMATEX_SIZE) };
    let tag = u16::from_le_bytes([bytes[0], bytes[1]]);
    let extra = u16::from_le_bytes([bytes[16], bytes[17]]);
    let len = if tag == wo::WAVE_FORMAT_EXTENSIBLE && extra >= wo::EXTENSIBLE_EXTRA {
        // SAFETY: the format says it has its extensible part.
        unsafe {
            core::ptr::copy_nonoverlapping(
                format.add(wo::WAVEFORMATEX_SIZE),
                bytes.as_mut_ptr().add(wo::WAVEFORMATEX_SIZE),
                wo::EXTENSIBLE_EXTRA as usize,
            )
        };
        bytes.len()
    } else {
        wo::WAVEFORMATEX_SIZE
    };
    let wf = match wo::check_format(&bytes[..len], query) {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    if query {
        return wo::MMSYSERR_NOERROR;
    }
    let cb = Callback { kind, target: callback, instance };
    let Some(handle) = open_wave(device, wf, cb) else { return fail(wo::MMSYSERR_NOMEM) };
    // SAFETY: the caller's handle variable.
    unsafe { *out = handle };
    notify(cb, handle, wo::WOM_OPEN, 0);
    wo::MMSYSERR_NOERROR
}

/// Make a handle with its stream and worker.
fn open_wave(device: u32, wf: WaveFormat, cb: Callback) -> Option<usize> {
    let file = open_device()?;
    let frames = (wf.format.rate * BUFFER_MS / 1000).max(64);
    let req = OpenReq { format: wf.format, buffer_frames: frames }.encode();
    if ioctl(file, wire::OPEN, &req, &mut []).0 != STATUS_SUCCESS {
        // SAFETY: our handle.
        unsafe { NtClose(file) };
        return None;
    }
    set_gain(file, VOLUME.load(SeqCst));
    // SAFETY: plain event creation.
    let (wake, io_event, reset_done) = unsafe {
        (CreateEventW(null(), 0, 0, null()), CreateEventW(null(), 1, 0, null()), CreateEventW(null(), 0, 0, null()))
    };
    let handle = NEXT_HANDLE.fetch_add(0x10, SeqCst);
    let wave = Box::new(Wave {
        handle,
        device,
        file,
        block: wf.block,
        rate: wf.format.rate,
        callback: cb,
        player: Player::default(),
        carry: Vec::new(),
        paused: false,
        pause_pos: None,
        running: false,
        closing: false,
        reset: false,
        thread: null_mut(),
        tid: 0,
        wake,
        io_event,
        reset_done,
        iosb: IoStatusBlock::default(),
        wait_out: [0; 8],
        wait_pending: false,
    });
    TABLE.with(|t| t.waves.push(wave));
    let mut tid = 0u32;
    // SAFETY: the worker takes the handle number as its parameter.
    let thread = unsafe { CreateThread(null(), 0, worker, handle as *mut c_void, 0, &mut tid) };
    TABLE.with(|t| {
        if let Some(w) = find(t, handle) {
            w.thread = thread;
            w.tid = tid;
        }
    });
    Some(handle)
}

/// `waveOutClose`.
#[unsafe(no_mangle)]
pub extern "system" fn waveOutClose(handle: usize) -> u32 {
    let taken = TABLE.with(|t| -> Result<Box<Wave>, u32> {
        let i = t.waves.iter().position(|w| w.handle == handle).ok_or(wo::MMSYSERR_INVALHANDLE)?;
        if !t.waves[i].player.is_empty() {
            return Err(wo::WAVERR_STILLPLAYING);
        }
        let mut w = t.waves.remove(i);
        w.closing = true;
        Ok(w)
    });
    let w = match taken {
        Ok(w) => w,
        Err(e) => return e,
    };
    wake(&w);
    // SAFETY: our thread and handles; the worker ends now that its handle is gone.
    unsafe {
        if GetCurrentThreadId() == w.tid {
            // Closed from its own callback: the worker ends after it; its state is let go.
            notify(w.callback, handle, wo::WOM_CLOSE, 0);
            Box::leak(w);
            return wo::MMSYSERR_NOERROR;
        }
        WaitForSingleObject(w.thread, INFINITE);
        CloseHandle(w.thread);
        NtClose(w.file);
        if w.wait_pending {
            // Closing the file cancels the wait; its status block is written before we go.
            WaitForSingleObject(w.io_event, 1000);
        }
        CloseHandle(w.wake);
        CloseHandle(w.io_event);
        CloseHandle(w.reset_done);
    }
    notify(w.callback, handle, wo::WOM_CLOSE, 0);
    wo::MMSYSERR_NOERROR
}

/// The common checks of a header argument.
fn header(h: *mut WaveHdr, size: u32) -> Result<(), u32> {
    if h.is_null() || size < HDR_SIZE { Err(wo::MMSYSERR_INVALPARAM) } else { Ok(()) }
}

fn valid(handle: usize) -> bool {
    TABLE.with(|t| find(t, handle).is_some())
}

/// `waveOutPrepareHeader`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutPrepareHeader(handle: usize, h: *mut WaveHdr, size: u32) -> u32 {
    if let Err(e) = header(h, size) {
        return e;
    }
    let flags = match wo::prepare(flags_of(h)) {
        Ok(f) => f,
        Err(e) => return e,
    };
    if !valid(handle) {
        return wo::MMSYSERR_INVALHANDLE;
    }
    set_flags(h, flags);
    wo::MMSYSERR_NOERROR
}

/// `waveOutUnprepareHeader`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutUnprepareHeader(handle: usize, h: *mut WaveHdr, size: u32) -> u32 {
    if let Err(e) = header(h, size) {
        return e;
    }
    match wo::unprepare(flags_of(h)) {
        Err(e) => e,
        Ok(Unprepare::NotPrepared) => wo::MMSYSERR_NOERROR,
        Ok(Unprepare::Prepared { queued, flags }) => {
            if !valid(handle) {
                return wo::MMSYSERR_INVALHANDLE;
            }
            if queued {
                return wo::WAVERR_STILLPLAYING;
            }
            set_flags(h, flags);
            wo::MMSYSERR_NOERROR
        }
    }
}

/// `waveOutWrite`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutWrite(handle: usize, h: *mut WaveHdr, size: u32) -> u32 {
    if let Err(e) = header(h, size) {
        return e;
    }
    TABLE.with(|t| {
        let Some(w) = find(t, handle) else { return wo::MMSYSERR_INVALHANDLE };
        let flags = match wo::write(flags_of(h)) {
            Ok(f) => f,
            Err(e) => return e,
        };
        set_flags(h, flags);
        // SAFETY: the application's prepared header.
        let (len, loops) = unsafe { ((*h).len, (*h).loops) };
        w.player.push(Queued {
            id: h as usize,
            len,
            begin_loop: flags & wo::WHDR_BEGINLOOP != 0,
            end_loop: flags & wo::WHDR_ENDLOOP != 0,
            loops,
        });
        wake(w);
        wo::MMSYSERR_NOERROR
    })
}

/// Run `f` on a handle's state; `MMSYSERR_INVALHANDLE` if it is none.
fn with_wave(handle: usize, f: impl FnOnce(&mut Wave)) -> u32 {
    TABLE.with(|t| match find(t, handle) {
        Some(w) => {
            f(w);
            wo::MMSYSERR_NOERROR
        }
        None => wo::MMSYSERR_INVALHANDLE,
    })
}

/// `waveOutPause`.
#[unsafe(no_mangle)]
pub extern "system" fn waveOutPause(handle: usize) -> u32 {
    with_wave(handle, |w| {
        if !w.paused {
            w.pause_pos = Some(query(w.file).played);
        }
        w.paused = true;
        if w.running {
            ioctl(w.file, wire::STOP, &[], &mut []);
            w.running = false;
        }
    })
}

/// `waveOutRestart`.
#[unsafe(no_mangle)]
pub extern "system" fn waveOutRestart(handle: usize) -> u32 {
    with_wave(handle, |w| {
        if w.paused {
            w.paused = false;
            w.pause_pos = None;
            wake(w);
        }
    })
}

/// `waveOutBreakLoop`.
#[unsafe(no_mangle)]
pub extern "system" fn waveOutBreakLoop(handle: usize) -> u32 {
    with_wave(handle, |w| {
        w.player.break_loop();
        wake(w);
    })
}

/// `waveOutReset`: the worker gives the queued headers back, with their `WOM_DONE`, before this
/// returns (called from a callback, on the worker's own thread, it does so itself).
#[unsafe(no_mangle)]
pub extern "system" fn waveOutReset(handle: usize) -> u32 {
    // SAFETY: a plain call.
    let me = unsafe { GetCurrentThreadId() };
    let mut inline = None;
    let mut wait = None;
    let r = with_wave(handle, |w| {
        if w.tid == me {
            inline = Some((w.callback, reset_queue(w)));
        } else {
            w.reset = true;
            wake(w);
            wait = Some(w.reset_done);
        }
    });
    if let Some((cb, ids)) = inline {
        for id in ids {
            notify(cb, handle, wo::WOM_DONE, id);
        }
        if cb.kind == wo::CALLBACK_EVENT {
            notify(cb, handle, wo::WOM_DONE, 0);
        }
    }
    if let Some(ev) = wait {
        // SAFETY: the handle's event; a close cannot happen meanwhile on another thread without
        // the application racing itself.
        unsafe { WaitForSingleObject(ev, INFINITE) };
    }
    r
}

/// `waveOutGetPosition`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutGetPosition(handle: usize, time: *mut u32, _size: u32) -> u32 {
    if time.is_null() {
        return wo::MMSYSERR_INVALPARAM;
    }
    let mut pos = None;
    let r = with_wave(handle, |w| pos = Some((played_of(w, &query(w.file)), w.block)));
    let Some((frames, block)) = pos else { return r };
    // SAFETY: the caller's `MMTIME` (12 bytes: the type, then the value).
    unsafe {
        let asked = time.read_unaligned();
        if asked == wo::TIME_SAMPLES {
            time.add(1).write_unaligned(frames as u32);
        } else {
            time.write_unaligned(wo::TIME_BYTES);
            time.add(1).write_unaligned((frames * u64::from(block)) as u32);
        }
    }
    wo::MMSYSERR_NOERROR
}

/// `waveOutGetID`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutGetID(handle: usize, id: *mut u32) -> u32 {
    if id.is_null() {
        return wo::MMSYSERR_INVALPARAM;
    }
    let mut dev = 0;
    let r = with_wave(handle, |w| dev = w.device);
    if r == wo::MMSYSERR_NOERROR {
        // SAFETY: the caller's variable.
        unsafe { *id = dev };
    }
    r
}

/// The device a handle-or-id names, for the volume and messages: `Ok(is the mapper id)`.
fn device_or_handle(id: usize) -> Result<bool, u32> {
    if is_device_id(id) {
        return Ok(id != 0);
    }
    if valid(id) { Ok(false) } else { Err(wo::MMSYSERR_BADDEVICEID) }
}

/// `waveOutGetVolume`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutGetVolume(id: usize, volume: *mut u32) -> u32 {
    if let Err(e) = device_or_handle(id) {
        return e;
    }
    if volume.is_null() {
        return wo::MMSYSERR_INVALPARAM;
    }
    // SAFETY: the caller's variable.
    unsafe { *volume = VOLUME.load(SeqCst) };
    wo::MMSYSERR_NOERROR
}

/// `waveOutSetVolume`: the process's volume, for every handle.
#[unsafe(no_mangle)]
pub extern "system" fn waveOutSetVolume(id: usize, volume: u32) -> u32 {
    if let Err(e) = device_or_handle(id) {
        return e;
    }
    VOLUME.store(volume, SeqCst);
    TABLE.with(|t| {
        for w in &t.waves {
            set_gain(w.file, volume);
        }
    });
    wo::MMSYSERR_NOERROR
}

/// `waveOutMessage`: only the mapper's preferred device is answered.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutMessage(id: usize, msg: u32, p1: usize, p2: usize) -> u32 {
    let mapper = match device_or_handle(id) {
        Ok(m) => m,
        Err(e) => return e,
    };
    if msg == wo::DRVM_MAPPER_PREFERRED_GET && mapper {
        // SAFETY: the caller's two DWORDs (a null one is skipped).
        unsafe {
            if p1 != 0 {
                *(p1 as *mut u32) = 0;
            }
            if p2 != 0 {
                *(p2 as *mut u32) = 0;
            }
        }
        return wo::MMSYSERR_NOERROR;
    }
    wo::MMSYSERR_NOTSUPPORTED
}

/// `waveOutGetErrorTextW` (`size` in characters).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutGetErrorTextW(code: u32, text: *mut u16, size: u32) -> u32 {
    if text.is_null() {
        return wo::MMSYSERR_INVALPARAM;
    }
    let t = wo::error_text(code);
    if size > 0 {
        let s = t.unwrap_or("");
        let n = s.len().min(size as usize - 1);
        // SAFETY: the caller's buffer of `size` characters.
        unsafe {
            for (i, b) in s.bytes().take(n).enumerate() {
                *text.add(i) = u16::from(b);
            }
            *text.add(n) = 0;
        }
    }
    if t.is_some() { wo::MMSYSERR_NOERROR } else { wo::MMSYSERR_BADERRNUM }
}

/// `waveOutGetErrorTextA` (`size` in bytes).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn waveOutGetErrorTextA(code: u32, text: *mut u8, size: u32) -> u32 {
    if text.is_null() {
        return wo::MMSYSERR_INVALPARAM;
    }
    let t = wo::error_text(code);
    if size > 0 {
        let s = t.unwrap_or("");
        let n = s.len().min(size as usize - 1);
        // SAFETY: the caller's buffer of `size` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), text, n);
            *text.add(n) = 0;
        }
    }
    if t.is_some() { wo::MMSYSERR_NOERROR } else { wo::MMSYSERR_BADERRNUM }
}
