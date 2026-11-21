//! The audio client (`IAudioClient`, `IAudioClient2`, `IAudioClient3`) and its services
//! (`IAudioRenderClient`, `IAudioClock`, `IAudioSessionControl`, `ISimpleAudioVolume`), shared
//! mode, render only. The rules are `poly_kernelbase::wasapi`'s ([`Stream`]); this file is the COM
//! shell and the engine.
//!
//! **The engine** is a thread per initialised client. It owns a `\Device\PolyAudio` stream in the
//! client's format (the kernel converts and mixes it, ADR 0244). While the client runs it does two
//! things, which the application sees apart:
//!
//! - it **sends** released frames ahead, so that the stream holds two engine periods (10 ms
//!   each): the lead over the mixer, which takes two periods at once when the device finished two
//!   (a Windows host's audio timer does that). Silence goes in only when the application has
//!   nothing more and the stream would run dry;
//! - it **consumes**: at each tick a period leaves the client's buffer and the client's event is
//!   set, so the padding falls by one period per event, as the sheets observe
//!   (`IAudioClient.Events.yaml`). `Start` consumes the first period at once.
//!
//! It ticks on its own clock, every 10 ms on an absolute schedule (the performance counter: the
//! host's clock, which the device's audio runs on too), so the events come at the engine's period
//! however the device finishes its periods (in pairs, on a Windows host). It does not consume while
//! the device stream holds more than four periods (the device fell behind), so no frame is dropped.
//! The session's state notifications go out from the engine's thread.

use crate::com::*;
use crate::endpoint::CoTaskMemAlloc;
use crate::polyaudio::{self, Handle};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::null;
use poly_audio::Sample;
use poly_audio::nt as wire;
use poly_com::guid::Guid;
use poly_kernelbase::wasapi::{self as wa, Registrations, Stream, StreamFormat};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateEventW(attributes: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, ms: u32) -> u32;
    fn CreateThread(
        attributes: *const u8,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> Handle;
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    /// Poly's own service: a line in the kernel log.
    fn PolyDebugPrint(text: *const u8, length: usize) -> u32;
}

/// The stream's figures in the kernel log, at each stop and when the client goes: the periods
/// the engine fed, the frames of silence it put in for an application that wrote too little, and
/// the underruns of the PolyAudio stream (the mixer found it empty).
fn log_stats(st: &State, why: &str) {
    let info = polyaudio::query(st.kernel);
    let text = alloc::format!(
        "mmdevapi: stream {why}: {} periods fed, {} frames of silence for the application, {} underruns ({} frames) on the device stream, {} ticks skipped (device behind), {} schedule restarts",
        st.periods_fed,
        st.app_silence,
        info.underruns,
        info.silence,
        st.skipped,
        st.restarts
    );
    // SAFETY: our text.
    unsafe { PolyDebugPrint(text.as_ptr(), text.len()) };
}

const INFINITE: u32 = 0xFFFF_FFFF;
/// An engine period, in 100 ns units: the engine ticks this often (10 ms, whatever the rate).
const PERIOD_HNS: u64 = 100_000;
/// Ticks owed after a late wake-up come at least this far apart (100 ns units): the application
/// woken by one has read its padding and written before the next.
const MIN_SPACING_HNS: u64 = 60_000;
/// A wake-up later than this many periods starts the schedule again (the machine stalled).
const MAX_OWED_PERIODS: u64 = 10;
/// The engine does not consume while the device stream holds more than this many periods (the
/// device fell behind the engine's clock): no frame is ever dropped.
const MAX_QUEUED_PERIODS: u32 = 4;

/// What a client and its services share.
pub struct Shared {
    state: Locked<State>,
    /// Set when the engine should look (start, stop, reset, release).
    wake: Handle,
}

// SAFETY: the handle is a kernel handle, usable from any thread; the state is behind its lock.
unsafe impl Send for Shared {}
// SAFETY: as above.
unsafe impl Sync for Shared {}

struct State {
    stream: Option<Stream>,
    format: Option<StreamFormat>,
    /// The engine's PolyAudio stream, its `WAIT` request and the thread.
    kernel: Handle,
    engine: Handle,
    /// The client object was released: the engine ends.
    dead: bool,
    /// `GetBuffer`'s memory (the whole buffer: its address does not move).
    scratch: Vec<u8>,
    sessions: Registrations,
    /// Session states to announce, oldest first.
    notes: Vec<u32>,
    /// The services, made once (each holds a reference of ours).
    services: [usize; 4],
    /// Periods fed, and frames of silence put in for the application ([`log_stats`]).
    periods_fed: u64,
    app_silence: u64,
    /// When the engine's next tick is due, and when it last ticked (100 ns units, the performance
    /// counter's time).
    next_tick: u64,
    last_tick: u64,
    /// Ticks that consumed nothing (the device was behind), and schedule restarts ([`log_stats`]).
    skipped: u64,
    restarts: u64,
}

// SAFETY: raw handles and pointers of objects usable from any thread.
unsafe impl Send for State {}

impl Shared {
    fn new() -> Arc<Shared> {
        // SAFETY: plain event creation.
        let wake = unsafe { CreateEventW(null(), 0, 0, null()) };
        Arc::new(Shared {
            state: Locked::new(State {
                stream: None,
                format: None,
                kernel: core::ptr::null_mut(),
                engine: core::ptr::null_mut(),
                dead: false,
                scratch: Vec::new(),
                sessions: Registrations::default(),
                notes: Vec::new(),
                services: [0; 4],
                periods_fed: 0,
                app_silence: 0,
                next_tick: 0,
                last_tick: 0,
                skipped: 0,
                restarts: 0,
            }),
            wake,
        })
    }

    fn poke(&self) {
        // SAFETY: our event.
        unsafe { SetEvent(self.wake) };
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let engine = self.state.with(|st| st.engine);
        // SAFETY: our handles, unused from here on.
        unsafe {
            CloseHandle(self.wake);
            if !engine.is_null() {
                CloseHandle(engine);
            }
        }
    }
}

/// The client object's data: when it goes, the engine stops and the services are let go.
pub struct Client(Arc<Shared>);

impl Drop for Client {
    fn drop(&mut self) {
        let services = self.0.state.with(|st| {
            st.dead = true;
            core::mem::take(&mut st.services)
        });
        self.0.poke();
        for (i, p) in services.into_iter().enumerate() {
            if p != 0 {
                // SAFETY: the reference the client held on its service.
                unsafe { release_service(i, p as *mut c_void) };
            }
        }
    }
}

// ----- the client

const CLIENT_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IAUDIO_CLIENT, IID_IAUDIO_CLIENT2, IID_IAUDIO_CLIENT3];
static CLIENT_VTBL: Vtbl<21> = Vtbl([
    query::<Client> as *const c_void,
    add_ref::<Client> as *const c_void,
    release::<Client> as *const c_void,
    initialize as *const c_void,
    get_buffer_size as *const c_void,
    get_stream_latency as *const c_void,
    get_current_padding as *const c_void,
    is_format_supported as *const c_void,
    get_mix_format as *const c_void,
    get_device_period as *const c_void,
    start as *const c_void,
    stop as *const c_void,
    reset as *const c_void,
    set_event_handle as *const c_void,
    get_service as *const c_void,
    // IAudioClient2: IsOffloadCapable, SetClientProperties, GetBufferSizeLimits (no sheet).
    not_impl3 as *const c_void,
    not_impl2 as *const c_void,
    not_impl5 as *const c_void,
    // IAudioClient3.
    get_shared_mode_engine_period as *const c_void,
    get_current_shared_mode_engine_period as *const c_void,
    initialize_shared_audio_stream as *const c_void,
]);

/// A new client into `out` as `iid`.
///
/// # Safety
/// The caller's pointers.
pub unsafe fn new_client(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let object = new_object(&CLIENT_VTBL, CLIENT_IIDS, Client(Shared::new()));
    // SAFETY: (caller) a fresh object.
    unsafe { hand_out::<Client>(object, iid, out) }
}

/// # Safety
/// `p` is one of our clients.
unsafe fn shared<'a>(p: *mut c_void) -> &'a Arc<Shared> {
    // SAFETY: (caller).
    unsafe { &data::<Client>(p).0 }
}

/// The caller's format: its `WAVEFORMATEX` and, for an extensible one, the extensible part.
///
/// # Safety
/// `fmt` is null or the caller's format.
unsafe fn format_bytes(fmt: *const u8) -> Option<Vec<u8>> {
    if fmt.is_null() {
        return None;
    }
    // SAFETY: (caller) at least a `WAVEFORMATEX`; its extensible part when it says it has one.
    unsafe {
        let head = core::slice::from_raw_parts(fmt, wa::WAVEFORMATEX_SIZE);
        let tag = u16::from_le_bytes([head[0], head[1]]);
        let extra = u16::from_le_bytes([head[16], head[17]]);
        let n =
            if tag == wa::WAVE_FORMAT_EXTENSIBLE && extra >= 22 { wa::EXTENSIBLE_SIZE } else { wa::WAVEFORMATEX_SIZE };
        Some(core::slice::from_raw_parts(fmt, n).to_vec())
    }
}

/// A CoTaskMem copy of the mix format.
fn mix_block() -> *mut u8 {
    let m = wa::mix_format();
    // SAFETY: a fresh CoTaskMem block of the format's size.
    unsafe {
        let p = CoTaskMemAlloc(m.len());
        if !p.is_null() {
            core::ptr::copy_nonoverlapping(m.as_ptr(), p, m.len());
        }
        p
    }
}

/// Set the client up: the stream's rules, the engine's PolyAudio stream and thread.
fn setup(sh: &Arc<Shared>, f: StreamFormat, flags: u32, duration: i64) -> Hresult {
    let stream = Stream::new(&f, flags, duration);
    // The engine's PolyAudio stream holds only its lead (two periods, at most
    // [`MAX_QUEUED_PERIODS`] before it skips), not the client's buffer, which may be
    // seconds long (`IAudioClient.Initialize.yaml`, `buffer_duration`: no upper limit seen):
    // 100 ms, ten engine periods.
    let frames = f.format.rate / 10;
    // The client's buffer is the application's size: refused, not aborted, when it does not fit.
    let len = stream.buffer as usize * stream.block as usize;
    let mut scratch = Vec::new();
    if scratch.try_reserve_exact(len).is_err() {
        return E_OUTOFMEMORY;
    }
    scratch.resize(len, 0);
    let Some(kernel) = polyaudio::open_stream(f.format, frames) else { return E_OUTOFMEMORY };
    sh.state.with(|st| {
        st.scratch = scratch;
        st.stream = Some(stream);
        st.format = Some(f);
        st.kernel = kernel;
    });
    let param = Arc::into_raw(Arc::clone(sh)) as *mut c_void;
    let mut tid = 0u32;
    // SAFETY: the engine takes the reference `param` stands for.
    let thread = unsafe { CreateThread(null(), 0, engine, param, 0, &mut tid) };
    sh.state.with(|st| st.engine = thread);
    S_OK
}

unsafe extern "system" fn initialize(
    p: *mut c_void,
    mode: u32,
    flags: u32,
    duration: i64,
    _periodicity: i64,
    fmt: *const u8,
    _session: *const Guid,
) -> Hresult {
    // SAFETY: our client; the caller's format.
    let (sh, bytes) = unsafe { (shared(p), format_bytes(fmt)) };
    if sh.state.with(|st| st.stream.is_some()) {
        return hr(wa::AUDCLNT_E_ALREADY_INITIALIZED);
    }
    match wa::check_initialize(mode, flags, duration, bytes.as_deref()) {
        Ok(f) => setup(sh, f, flags, duration),
        Err(e) => hr(e),
    }
}

unsafe extern "system" fn initialize_shared_audio_stream(
    p: *mut c_void,
    flags: u32,
    period: u32,
    fmt: *const u8,
    _session: *const Guid,
) -> Hresult {
    // SAFETY: our client; the caller's format.
    let (sh, bytes) = unsafe { (shared(p), format_bytes(fmt)) };
    if sh.state.with(|st| st.stream.is_some()) {
        return hr(wa::AUDCLNT_E_ALREADY_INITIALIZED);
    }
    match wa::check_initialize(wa::AUDCLNT_SHAREMODE_SHARED, flags, 0, bytes.as_deref()) {
        Ok(f) if period != wa::period_frames(f.format.rate) => hr(wa::AUDCLNT_E_INVALID_DEVICE_PERIOD),
        Ok(f) => setup(sh, f, flags, 0),
        Err(e) => hr(e),
    }
}

/// Run `f` on the initialised stream (with the state), else `AUDCLNT_E_NOT_INITIALIZED`.
fn with_stream(p: *mut c_void, f: impl FnOnce(&mut Stream, &mut State) -> Hresult) -> Hresult {
    // SAFETY: our client.
    let sh = unsafe { shared(p) };
    sh.state.with(|st| match st.stream.take() {
        None => hr(wa::AUDCLNT_E_NOT_INITIALIZED),
        Some(mut s) => {
            let r = f(&mut s, st);
            st.stream = Some(s);
            r
        }
    })
}

unsafe extern "system" fn get_buffer_size(p: *mut c_void, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    with_stream(p, |s, _| {
        // SAFETY: the caller's out parameter.
        unsafe { *out = s.buffer };
        S_OK
    })
}

unsafe extern "system" fn get_stream_latency(p: *mut c_void, out: *mut i64) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    with_stream(p, |_, _| {
        // SAFETY: the caller's out parameter.
        unsafe { *out = 0 };
        S_OK
    })
}

unsafe extern "system" fn get_current_padding(p: *mut c_void, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    with_stream(p, |s, _| {
        // SAFETY: the caller's out parameter.
        unsafe { *out = s.padding() };
        S_OK
    })
}

unsafe extern "system" fn is_format_supported(
    _: *mut c_void,
    mode: u32,
    fmt: *const u8,
    closest: *mut *mut u8,
) -> Hresult {
    // SAFETY: the caller's format.
    let bytes = unsafe { format_bytes(fmt) };
    if !closest.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *closest = core::ptr::null_mut() };
    }
    let r = wa::is_format_supported(mode, bytes.as_deref(), !closest.is_null());
    if r == wa::S_FALSE {
        // SAFETY: checked non-null by the rule (`S_FALSE` only with a closest-match pointer).
        unsafe { *closest = mix_block() };
    }
    hr(r)
}

unsafe extern "system" fn get_mix_format(_: *mut c_void, out: *mut *mut u8) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    let p = mix_block();
    // SAFETY: the caller's out parameter.
    unsafe { *out = p };
    if p.is_null() { E_OUTOFMEMORY } else { S_OK }
}

unsafe extern "system" fn get_device_period(_: *mut c_void, default: *mut i64, minimum: *mut i64) -> Hresult {
    if default.is_null() && minimum.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameters (each if given).
    unsafe {
        if !default.is_null() {
            *default = wa::DEFAULT_PERIOD_HNS;
        }
        if !minimum.is_null() {
            *minimum = wa::MINIMUM_PERIOD_HNS;
        }
    }
    S_OK
}

/// Silence in the stream's format.
fn silence_byte(st: &State) -> u8 {
    if st.format.is_some_and(|f| f.format.sample == Sample::U8) { 0x80 } else { 0 }
}

/// Send released frames ahead until the engine's stream holds two periods (the lead over the
/// mixer, which may take two periods at once when the device finished two). Silence goes in only
/// when the application has nothing more and the stream would run dry.
fn send_ahead(s: &mut Stream, st: &mut State, mut queued: u32) {
    let target = 2 * s.period;
    while queued < target {
        let bytes = s.send(target - queued);
        if bytes.is_empty() {
            // Nothing more from the application: when the stream would run dry, silence keeps the
            // lead (the device goes on, and so do the events: `IAudioClient.Events.yaml`).
            if queued < s.period {
                let silence = vec![silence_byte(st); (s.period * s.block) as usize];
                while queued < target {
                    polyaudio::ioctl(st.kernel, wire::WRITE, &silence, &mut []);
                    st.app_silence += u64::from(s.period);
                    queued += s.period;
                }
            }
            return;
        }
        polyaudio::ioctl(st.kernel, wire::WRITE, &bytes, &mut []);
        queued += (bytes.len() / s.block as usize) as u32;
    }
}

/// One engine tick: a period leaves the client's buffer and the client's event is set (the
/// padding at each wake-up is the buffer less one period, `IAudioClient.Events.yaml`), unless the
/// device fell behind the engine's clock (the stream holds more than [`MAX_QUEUED_PERIODS`]); then
/// the device stream is topped up ([`send_ahead`]). The client's event, to set.
fn tick(s: &mut Stream, st: &mut State) -> Option<usize> {
    let queued = polyaudio::query(st.kernel).queued;
    let mut event = None;
    if queued <= MAX_QUEUED_PERIODS * s.period {
        s.consume(u64::from(s.period));
        st.periods_fed += 1;
        event = s.event;
    } else {
        st.skipped += 1;
    }
    send_ahead(s, st, polyaudio::query(st.kernel).queued);
    event
}

unsafe extern "system" fn start(p: *mut c_void) -> Hresult {
    let mut event = None;
    let r = with_stream(p, |s, st| {
        if let Err(e) = s.start() {
            return hr(e);
        }
        // The first period leaves at once (the first event finds the buffer one period short);
        // from then on one leaves for each period the device played.
        s.consume(u64::from(s.period));
        st.periods_fed += 1;
        event = s.event;
        st.last_tick = qpc_hns();
        st.next_tick = st.last_tick + PERIOD_HNS;
        send_ahead(s, st, polyaudio::query(st.kernel).queued);
        polyaudio::ioctl(st.kernel, wire::START, &[], &mut []);
        st.notes.push(wa::SESSION_ACTIVE);
        S_OK
    });
    if r == S_OK {
        // A running stream holds a reference on its client (`IAudioClient.Stream.yaml`, `clients`:
        // released while it runs, the client still counts one); `Stop` gives it back.
        // SAFETY: our client.
        unsafe { add_ref::<Client>(p) };
        if let Some(e) = event {
            // SAFETY: the application's event.
            unsafe { SetEvent(e as Handle) };
        }
        // SAFETY: our client.
        unsafe { shared(p) }.poke();
    }
    r
}

unsafe extern "system" fn stop(p: *mut c_void) -> Hresult {
    let r = with_stream(p, |s, st| {
        let r = s.stop();
        if r == wa::S_OK {
            polyaudio::ioctl(st.kernel, wire::STOP, &[], &mut []);
            st.notes.push(wa::SESSION_INACTIVE);
            log_stats(st, "stopped");
        }
        hr(r)
    });
    // SAFETY: our client.
    unsafe { shared(p) }.poke();
    if r == S_OK {
        // The running stream's reference (`start`); it may be the last.
        // SAFETY: our client.
        unsafe { release::<Client>(p) };
    }
    r
}

unsafe extern "system" fn reset(p: *mut c_void) -> Hresult {
    with_stream(p, |s, st| {
        if let Err(e) = s.reset() {
            return hr(e);
        }
        polyaudio::ioctl(st.kernel, wire::RESET, &[], &mut []);
        S_OK
    })
}

unsafe extern "system" fn set_event_handle(p: *mut c_void, event: Handle) -> Hresult {
    with_stream(p, |s, _| match s.set_event(event as usize) {
        Ok(()) => S_OK,
        Err(e) => hr(e),
    })
}

unsafe extern "system" fn get_service(p: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_POINTER;
        }
    }
    // SAFETY: as above.
    let want = unsafe { iid.read_unaligned() };
    let index = match want {
        w if w == IID_IAUDIO_RENDER_CLIENT => 0,
        w if w == IID_IAUDIO_CLOCK => 1,
        w if w == IID_IAUDIO_SESSION_CONTROL => 2,
        w if w == IID_ISIMPLE_AUDIO_VOLUME => 3,
        w if w == IID_IAUDIO_CAPTURE_CLIENT => 4,
        _ => return E_NOINTERFACE,
    };
    // SAFETY: our client.
    let sh = unsafe { shared(p) };
    let object = sh.state.with(|st| {
        if st.stream.is_none() {
            return Err(hr(wa::AUDCLNT_E_NOT_INITIALIZED));
        }
        if index == 4 {
            return Err(hr(wa::AUDCLNT_E_WRONG_ENDPOINT_TYPE));
        }
        if st.services[index] == 0 {
            st.services[index] = new_service(index, Arc::clone(sh)) as usize;
        }
        Ok(st.services[index] as *mut c_void)
    });
    match object {
        // SAFETY: our service object (the client's reference keeps it).
        Ok(o) => unsafe { service_query(index, o, iid, out) },
        Err(e) => e,
    }
}

unsafe extern "system" fn get_shared_mode_engine_period(
    _: *mut c_void,
    fmt: *const u8,
    default: *mut u32,
    fundamental: *mut u32,
    minimum: *mut u32,
    maximum: *mut u32,
) -> Hresult {
    if fmt.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's format: its rate is at bytes 4..8.
    let rate = unsafe { (fmt.add(4) as *const u32).read_unaligned() };
    let period = wa::period_frames(rate);
    for out in [default, fundamental, minimum, maximum] {
        if !out.is_null() {
            // SAFETY: the caller's out parameters.
            unsafe { *out = period };
        }
    }
    S_OK
}

unsafe extern "system" fn get_current_shared_mode_engine_period(
    _: *mut c_void,
    fmt: *mut *mut u8,
    period: *mut u32,
) -> Hresult {
    if fmt.is_null() || period.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameters.
    unsafe {
        *fmt = mix_block();
        *period = wa::period_frames(wa::MIX_RATE);
    }
    S_OK
}

// ----- the services

const RENDER_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IAUDIO_RENDER_CLIENT];
static RENDER_VTBL: Vtbl<5> = Vtbl([
    query::<Arc<Shared>> as *const c_void,
    add_ref::<Arc<Shared>> as *const c_void,
    release::<Arc<Shared>> as *const c_void,
    render_get_buffer as *const c_void,
    render_release_buffer as *const c_void,
]);
const CLOCK_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IAUDIO_CLOCK];
static CLOCK_VTBL: Vtbl<6> = Vtbl([
    clock_query as *const c_void,
    add_ref::<Arc<Shared>> as *const c_void,
    release::<Arc<Shared>> as *const c_void,
    clock_frequency as *const c_void,
    clock_position as *const c_void,
    clock_characteristics as *const c_void,
]);
/// `IAudioClock2` on the clock: a tear-off (its slot 3 is not the clock's), which holds a
/// reference on the clock and answers every other interface as the clock does, so the clock's
/// `IUnknown` is the identity of both.
struct Clock2 {
    clock: *mut c_void,
}

impl Drop for Clock2 {
    fn drop(&mut self) {
        // SAFETY: the reference on the clock this tear-off held.
        unsafe { release::<Arc<Shared>>(self.clock) };
    }
}

const CLOCK2_IIDS: &[Guid] = &[IID_IAUDIO_CLOCK2];
static CLOCK2_VTBL: Vtbl<4> = Vtbl([
    clock2_query as *const c_void,
    add_ref::<Clock2> as *const c_void,
    release::<Clock2> as *const c_void,
    clock2_device_position as *const c_void,
]);

/// The clock's `QueryInterface`: its own interfaces, and `IAudioClock2` as a new tear-off.
unsafe extern "system" fn clock_query(p: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: the caller's pointers (checked like `query` does); `p` is our clock.
    unsafe {
        if !out.is_null() && !iid.is_null() && iid.read_unaligned() == IID_IAUDIO_CLOCK2 {
            add_ref::<Arc<Shared>>(p);
            *out = new_object(&CLOCK2_VTBL, CLOCK2_IIDS, Clock2 { clock: p });
            return S_OK;
        }
        query::<Arc<Shared>>(p, iid, out)
    }
}

unsafe extern "system" fn clock2_query(p: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: the caller's pointers; `p` is our tear-off.
    unsafe {
        if !out.is_null() && !iid.is_null() && iid.read_unaligned() == IID_IAUDIO_CLOCK2 {
            return query::<Clock2>(p, iid, out);
        }
        clock_query(data::<Clock2>(p).clock, iid, out)
    }
}

/// `IAudioClock2::GetDevicePosition` (Learn): the device position in frames (no sheet: here the
/// clock's position in the stream's frames) and the performance counter of it in 100 ns units.
unsafe extern "system" fn clock2_device_position(p: *mut c_void, position: *mut u64, qpc: *mut u64) -> Hresult {
    if position.is_null() {
        return E_POINTER;
    }
    // SAFETY: our tear-off; its clock's data is the client's.
    let sh = unsafe { service_shared(data::<Clock2>(p).clock) };
    let frames = sh.state.with(|st| {
        let block = st.format.map_or(0, |f| u64::from(f.block));
        st.stream.as_ref().map_or(0, |s| if block == 0 { 0 } else { s.position() / block })
    });
    // SAFETY: the caller's out parameters.
    unsafe {
        *position = frames;
        if !qpc.is_null() {
            *qpc = qpc_hns();
        }
    }
    S_OK
}

const SESSION_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IAUDIO_SESSION_CONTROL];
static SESSION_VTBL: Vtbl<12> = Vtbl([
    query::<Arc<Shared>> as *const c_void,
    add_ref::<Arc<Shared>> as *const c_void,
    release::<Arc<Shared>> as *const c_void,
    session_state as *const c_void,
    // GetDisplayName, SetDisplayName, GetIconPath, SetIconPath, GetGroupingParam,
    // SetGroupingParam (no sheet).
    not_impl2 as *const c_void,
    not_impl3 as *const c_void,
    not_impl2 as *const c_void,
    not_impl3 as *const c_void,
    not_impl2 as *const c_void,
    not_impl3 as *const c_void,
    session_register as *const c_void,
    session_unregister as *const c_void,
]);
const VOLUME_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_ISIMPLE_AUDIO_VOLUME];
/// `ISimpleAudioVolume`: the object is there (`GetService` gives it); no sheet specifies its methods.
static VOLUME_VTBL: Vtbl<7> = Vtbl([
    query::<Arc<Shared>> as *const c_void,
    add_ref::<Arc<Shared>> as *const c_void,
    release::<Arc<Shared>> as *const c_void,
    not_impl_f32_ptr as *const c_void,
    not_impl2 as *const c_void,
    not_impl3 as *const c_void,
    not_impl2 as *const c_void,
]);

fn new_service(index: usize, sh: Arc<Shared>) -> *mut c_void {
    match index {
        0 => new_object(&RENDER_VTBL, RENDER_IIDS, sh),
        1 => new_object(&CLOCK_VTBL, CLOCK_IIDS, sh),
        2 => new_object(&SESSION_VTBL, SESSION_IIDS, sh),
        _ => new_object(&VOLUME_VTBL, VOLUME_IIDS, sh),
    }
}

/// # Safety
/// `o` is our service object of kind `index`; the caller's pointers.
unsafe fn service_query(index: usize, o: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: (caller) every service's data is an `Arc<Shared>`; index 1 is the clock.
    unsafe { if index == 1 { clock_query(o, iid, out) } else { query::<Arc<Shared>>(o, iid, out) } }
}

/// # Safety
/// A reference of ours on a service object.
unsafe fn release_service(_index: usize, o: *mut c_void) {
    // SAFETY: (caller).
    unsafe { release::<Arc<Shared>>(o) };
}

/// # Safety
/// `p` is one of our service objects.
unsafe fn service_shared<'a>(p: *mut c_void) -> &'a Arc<Shared> {
    // SAFETY: (caller).
    unsafe { data::<Arc<Shared>>(p) }
}

unsafe extern "system" fn render_get_buffer(p: *mut c_void, frames: u32, out: *mut *mut u8) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter; our service.
    unsafe { *out = core::ptr::null_mut() };
    let sh = unsafe { service_shared(p) };
    sh.state.with(|st| {
        let Some(s) = st.stream.as_mut() else { return hr(wa::AUDCLNT_E_NOT_INITIALIZED) };
        match s.get_buffer(frames) {
            Ok(true) => {
                // SAFETY: the caller's out parameter; the scratch buffer holds the whole buffer.
                unsafe { *out = st.scratch.as_mut_ptr() };
                S_OK
            }
            Ok(false) => S_OK,
            Err(e) => hr(e),
        }
    })
}

unsafe extern "system" fn render_release_buffer(p: *mut c_void, frames: u32, flags: u32) -> Hresult {
    // SAFETY: our service.
    let sh = unsafe { service_shared(p) };
    sh.state.with(|st| {
        let State { stream, scratch, .. } = st;
        let Some(s) = stream.as_mut() else { return hr(wa::AUDCLNT_E_NOT_INITIALIZED) };
        match s.release_buffer(frames, flags, scratch) {
            Ok(()) => S_OK,
            Err(e) => hr(e),
        }
    })
}

unsafe extern "system" fn clock_frequency(p: *mut c_void, out: *mut u64) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our service.
    let sh = unsafe { service_shared(p) };
    let f = sh.state.with(|st| st.format.map_or(0, |f| u64::from(f.format.rate) * u64::from(f.block)));
    // SAFETY: the caller's out parameter.
    unsafe { *out = f };
    S_OK
}

/// The performance counter in 100 ns units.
fn qpc_hns() -> u64 {
    let (mut c, mut f) = (0i64, 0i64);
    // SAFETY: locals.
    unsafe {
        QueryPerformanceCounter(&mut c);
        QueryPerformanceFrequency(&mut f);
    }
    if f <= 0 { 0 } else { (c as i128 * 10_000_000 / f as i128) as u64 }
}

unsafe extern "system" fn clock_position(p: *mut c_void, position: *mut u64, qpc: *mut u64) -> Hresult {
    if position.is_null() {
        return E_POINTER;
    }
    // SAFETY: our service.
    let sh = unsafe { service_shared(p) };
    let pos = sh.state.with(|st| st.stream.as_ref().map_or(0, Stream::position));
    // SAFETY: the caller's out parameters.
    unsafe {
        *position = pos;
        if !qpc.is_null() {
            *qpc = qpc_hns();
        }
    }
    S_OK
}

unsafe extern "system" fn clock_characteristics(_: *mut c_void, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = wa::AUDIOCLOCK_CHARACTERISTIC_FIXED_FREQ };
    S_OK
}

unsafe extern "system" fn session_state(p: *mut c_void, out: *mut u32) -> Hresult {
    if out.is_null() {
        return hr(wa::RPC_X_NULL_REF_POINTER);
    }
    // SAFETY: our service.
    let sh = unsafe { service_shared(p) };
    let running = sh.state.with(|st| st.stream.as_ref().is_some_and(|s| s.running));
    // SAFETY: the caller's out parameter.
    unsafe { *out = if running { wa::SESSION_ACTIVE } else { wa::SESSION_INACTIVE } };
    S_OK
}

unsafe extern "system" fn session_register(p: *mut c_void, events: *mut c_void) -> Hresult {
    // SAFETY: our service.
    let sh = unsafe { service_shared(p) };
    let r = sh.state.with(|st| st.sessions.register(events as usize));
    if r == wa::S_OK {
        // SAFETY: the caller's object; each registration holds a reference.
        unsafe { foreign_add_ref(events) };
    }
    hr(r)
}

unsafe extern "system" fn session_unregister(p: *mut c_void, events: *mut c_void) -> Hresult {
    // SAFETY: our service.
    let sh = unsafe { service_shared(p) };
    match sh.state.with(|st| st.sessions.unregister(events as usize)) {
        Ok(_) => {
            // SAFETY: the reference of that registration.
            unsafe { foreign_release(events) };
            S_OK
        }
        Err(e) => hr(e),
    }
}

// ----- the engine

/// `IAudioSessionEvents::OnStateChanged` (slot 8).
///
/// # Safety
/// `sink` is a live `IAudioSessionEvents` we hold a reference to.
unsafe fn on_state_changed(sink: *mut c_void, state: u32) {
    // SAFETY: (caller).
    unsafe {
        let vtbl = *(sink as *const *const usize);
        let f: unsafe extern "system" fn(*mut c_void, u32) -> Hresult = core::mem::transmute(*vtbl.add(8));
        f(sink, state);
    }
}

unsafe extern "system" fn engine(param: *mut c_void) -> u32 {
    // SAFETY: the reference `setup` made for the engine.
    let sh: Arc<Shared> = unsafe { Arc::from_raw(param as *const Shared) };
    loop {
        let mut notes: Vec<(usize, u32)> = Vec::new();
        let mut event = None;
        let step = sh.state.with(|st| {
            if st.dead {
                return None;
            }
            for state in core::mem::take(&mut st.notes) {
                for &sink in &st.sessions.list {
                    // SAFETY: a registered sink; a reference for the call.
                    unsafe { foreign_add_ref(sink as *mut c_void) };
                    notes.push((sink, state));
                }
            }
            let mut stream = st.stream.take()?;
            let running = stream.running;
            let mut wait_ms = INFINITE;
            if running {
                let now = qpc_hns();
                // A late wake-up catches up: the ticks owed come [`MIN_SPACING_HNS`] apart, so the
                // application woken by one has read its padding and written before the next. Only
                // a stall longer than [`MAX_OWED_PERIODS`] starts the schedule again.
                if now >= st.next_tick && now >= st.last_tick + MIN_SPACING_HNS {
                    if now - st.next_tick > MAX_OWED_PERIODS * PERIOD_HNS {
                        st.next_tick = now;
                        st.restarts += 1;
                    }
                    st.next_tick += PERIOD_HNS;
                    st.last_tick = now;
                    event = tick(&mut stream, st);
                }
                let due = st.next_tick.max(st.last_tick + MIN_SPACING_HNS);
                wait_ms = (due.saturating_sub(qpc_hns()) / 10_000 + 1) as u32;
            }
            st.stream = Some(stream);
            Some(wait_ms)
        });
        for (sink, state) in notes {
            // SAFETY: the reference taken above.
            unsafe {
                on_state_changed(sink as *mut c_void, state);
                foreign_release(sink as *mut c_void);
            }
        }
        if let Some(e) = event {
            // SAFETY: the application's event.
            unsafe { SetEvent(e as Handle) };
        }
        let Some(wait_ms) = step else { break };
        // SAFETY: our event.
        unsafe { WaitForSingleObject(sh.wake, wait_ms) };
    }
    // The client is gone: so is its stream.
    let kernel = sh.state.with(|st| {
        if st.periods_fed > 0 {
            log_stats(st, "released");
        }
        st.kernel
    });
    // SAFETY: our handle.
    unsafe { polyaudio::NtClose(kernel) };
    0
}
