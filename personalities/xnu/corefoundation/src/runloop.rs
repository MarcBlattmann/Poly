//! CFRunLoop with its sources (version 0), timers and observers
//! (`<CoreFoundation/CFRunLoop.h>`), as the CFRunLoop spec sheets describe
//! a run: Entry, then passes of BeforeTimers, BeforeSources (signalled
//! sources by order), BeforeWaiting, the wait, AfterWaiting and due timers,
//! until a source was handled (when asked), the loop was stopped, the time
//! ran out or the mode has nothing left; then Exit.
//!
//! A loop sleeps in a Mach receive with a timeout on a port of its own
//! (`mach_port_allocate`, `mach_msg`). Waking it from another thread sends
//! to that port; v0 has one thread, so a wake-up comes from the loop's own
//! thread and only makes its next wait return at once (a waiting loop
//! cannot be called from its own thread). Sending needs `mach_msg2_trap`,
//! which Poly's kernel does not have yet (ADR 0238).

use crate::base::{self, Boolean, CFIndex, Header, Ref, Shared, id};
use crate::string::{self, ConstStr, cfstr};
use alloc::vec::Vec;
use core::ffi::c_void;

pub const RUN_FINISHED: i32 = 1;
pub const RUN_STOPPED: i32 = 2;
pub const RUN_TIMED_OUT: i32 = 3;
pub const RUN_HANDLED_SOURCE: i32 = 4;

/// `CFRunLoopActivity` bits.
pub const ENTRY: u64 = 1;
pub const BEFORE_TIMERS: u64 = 2;
pub const BEFORE_SOURCES: u64 = 4;
pub const BEFORE_WAITING: u64 = 32;
pub const AFTER_WAITING: u64 = 64;
pub const EXIT: u64 = 128;

static DEFAULT_MODE: ConstStr = cfstr!("kCFRunLoopDefaultMode");
static COMMON_MODES: ConstStr = cfstr!("kCFRunLoopCommonModes");

#[unsafe(no_mangle)]
pub static kCFRunLoopDefaultMode: Shared<ConstStr> = Shared(&DEFAULT_MODE);
#[unsafe(no_mangle)]
pub static kCFRunLoopCommonModes: Shared<ConstStr> = Shared(&COMMON_MODES);

fn default_mode() -> Ref {
    string::constant(&DEFAULT_MODE)
}

fn common_modes() -> Ref {
    string::constant(&COMMON_MODES)
}

/// `CFRunLoopSourceContext`, version 0.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SourceContext {
    pub version: CFIndex,
    pub info: *mut c_void,
    pub retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    pub release: Option<unsafe extern "C" fn(*const c_void)>,
    pub copy_description: Option<unsafe extern "C" fn(*const c_void) -> Ref>,
    pub equal: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> Boolean>,
    pub hash: Option<unsafe extern "C" fn(*const c_void) -> usize>,
    pub schedule: Option<unsafe extern "C" fn(*mut c_void, Ref, Ref)>,
    pub cancel: Option<unsafe extern "C" fn(*mut c_void, Ref, Ref)>,
    pub perform: Option<unsafe extern "C" fn(*mut c_void)>,
}

/// `CFRunLoopTimerContext` and `CFRunLoopObserverContext` (the same five
/// fields).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SmallContext {
    pub version: CFIndex,
    pub info: *mut c_void,
    pub retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    pub release: Option<unsafe extern "C" fn(*const c_void)>,
    pub copy_description: Option<unsafe extern "C" fn(*const c_void) -> Ref>,
}

const NO_SMALL: SmallContext =
    SmallContext { version: 0, info: core::ptr::null_mut(), retain: None, release: None, copy_description: None };

#[repr(C)]
pub struct Source {
    h: Header,
    order: CFIndex,
    ctx: SourceContext,
    valid: bool,
    signalled: bool,
}

type TimerCallout = Option<unsafe extern "C" fn(Ref, *mut c_void)>;
type ObserverCallout = Option<unsafe extern "C" fn(Ref, u64, *mut c_void)>;

#[repr(C)]
pub struct Timer {
    h: Header,
    next: f64,
    interval: f64,
    tolerance: f64,
    order: CFIndex,
    callout: TimerCallout,
    ctx: SmallContext,
    valid: bool,
}

#[repr(C)]
pub struct Observer {
    h: Header,
    activities: u64,
    repeats: bool,
    order: CFIndex,
    callout: ObserverCallout,
    ctx: SmallContext,
    valid: bool,
}

struct Mode {
    /// The name (retained).
    name: Ref,
    sources: Vec<Ref>,
    timers: Vec<Ref>,
    observers: Vec<Ref>,
    /// The loop keeps something of its own here (the main loop's default
    /// mode), so the mode is never empty.
    own_source: bool,
}

#[repr(C)]
pub struct RunLoop {
    h: Header,
    modes: Vec<Mode>,
    /// The names of the common modes (retained).
    common: Vec<Ref>,
    /// Items added for `kCFRunLoopCommonModes`, added to new common modes.
    common_items: Vec<Ref>,
    current: Ref,
    stopped: bool,
    waiting: bool,
    wake_pending: bool,
    port: Option<u32>,
}

static MAIN: crate::Global<Ref> = crate::Global::new(core::ptr::null());

fn rl_mut<'a>(cf: Ref) -> &'a mut RunLoop {
    if base::type_of(cf) != id::RUN_LOOP {
        base::crash("not a CFRunLoop");
    }
    // SAFETY: a run loop object; one thread; callers never hold it across a
    // callback into the program.
    unsafe { &mut *(cf as *mut RunLoop) }
}

fn source<'a>(cf: Ref) -> &'a mut Source {
    if base::type_of(cf) != id::SOURCE {
        base::crash("not a CFRunLoopSource");
    }
    // SAFETY: a source object; one thread.
    unsafe { &mut *(cf as *mut Source) }
}

fn timer<'a>(cf: Ref) -> &'a mut Timer {
    if base::type_of(cf) != id::TIMER {
        base::crash("not a CFRunLoopTimer");
    }
    // SAFETY: a timer object; one thread.
    unsafe { &mut *(cf as *mut Timer) }
}

fn observer<'a>(cf: Ref) -> &'a mut Observer {
    if base::type_of(cf) != id::OBSERVER {
        base::crash("not a CFRunLoopObserver");
    }
    // SAFETY: an observer object; one thread.
    unsafe { &mut *(cf as *mut Observer) }
}

fn main_loop() -> Ref {
    // SAFETY: one thread (v0).
    let m = unsafe { MAIN.get() };
    if m.is_null() {
        let mode_name = default_mode();
        let rl = base::make(core::ptr::null(), id::RUN_LOOP, false, |mut h| {
            h.info |= base::IMMORTAL;
            RunLoop {
                h,
                modes: alloc::vec![Mode {
                    name: mode_name,
                    sources: Vec::new(),
                    timers: Vec::new(),
                    observers: Vec::new(),
                    own_source: true,
                }],
                common: alloc::vec![mode_name],
                common_items: Vec::new(),
                current: core::ptr::null(),
                stopped: false,
                waiting: false,
                wake_pending: false,
                port: crate::sys::new_port(),
            }
        });
        *m = rl as Ref;
    }
    *m
}

fn same_name(a: Ref, b: Ref) -> bool {
    base::equal(a, b)
}

/// The mode of that name, made if `create`.
fn mode_index(rl: Ref, name: Ref, create: bool) -> Option<usize> {
    let l = rl_mut(rl);
    if let Some(i) = l.modes.iter().position(|m| same_name(m.name, name)) {
        return Some(i);
    }
    if !create {
        return None;
    }
    let n = string::CFStringCreateCopy(core::ptr::null(), name);
    l.modes.push(Mode { name: n, sources: Vec::new(), timers: Vec::new(), observers: Vec::new(), own_source: false });
    Some(l.modes.len() - 1)
}

fn is_empty(m: &Mode) -> bool {
    !m.own_source && m.sources.is_empty() && m.timers.is_empty() && m.observers.is_empty()
}

/// The mode names an add or remove for `name` applies to.
fn targets(rl: Ref, name: Ref) -> Vec<Ref> {
    if same_name(name, common_modes()) { rl_mut(rl).common.clone() } else { alloc::vec![name] }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Source,
    Timer,
    Observer,
}

fn list(m: &mut Mode, k: Kind) -> &mut Vec<Ref> {
    match k {
        Kind::Source => &mut m.sources,
        Kind::Timer => &mut m.timers,
        Kind::Observer => &mut m.observers,
    }
}

fn add(rl: Ref, item: Ref, name: Ref, k: Kind) {
    if same_name(name, common_modes()) && !rl_mut(rl).common_items.contains(&item) {
        rl_mut(rl).common_items.push(base::retain(item));
    }
    for n in targets(rl, name) {
        let Some(i) = mode_index(rl, n, true) else { continue };
        let mode_name = rl_mut(rl).modes[i].name;
        let l = list(&mut rl_mut(rl).modes[i], k);
        if l.contains(&item) {
            continue;
        }
        l.push(base::retain(item));
        if k == Kind::Source {
            let s = source(item);
            if let Some(f) = s.ctx.schedule {
                // SAFETY: the program's function with its own info.
                unsafe { f(s.ctx.info, rl, mode_name) };
            }
        }
    }
}

fn remove(rl: Ref, item: Ref, name: Ref, k: Kind) {
    if same_name(name, common_modes()) {
        let ci = &mut rl_mut(rl).common_items;
        if let Some(p) = ci.iter().position(|&x| x == item) {
            let x = ci.remove(p);
            base::release(x);
        }
    }
    for n in targets(rl, name) {
        let Some(i) = mode_index(rl, n, false) else { continue };
        let mode_name = rl_mut(rl).modes[i].name;
        let l = list(&mut rl_mut(rl).modes[i], k);
        let Some(p) = l.iter().position(|&x| x == item) else { continue };
        l.remove(p);
        if k == Kind::Source {
            let s = source(item);
            if let Some(f) = s.ctx.cancel {
                // SAFETY: the program's function.
                unsafe { f(s.ctx.info, rl, mode_name) };
            }
        }
        base::release(item);
    }
}

fn contains(rl: Ref, item: Ref, name: Ref, k: Kind) -> bool {
    let name = if same_name(name, common_modes()) { default_mode() } else { name };
    match mode_index(rl, name, false) {
        Some(i) => list(&mut rl_mut(rl).modes[i], k).contains(&item),
        None => false,
    }
}

/// Take `item` out of every mode of the main loop (invalidation).
fn remove_everywhere(item: Ref, k: Kind) {
    let rl = main_loop();
    let names: Vec<Ref> = rl_mut(rl).modes.iter().map(|m| m.name).collect();
    for n in names {
        remove(rl, item, n, k);
    }
    let ci = &mut rl_mut(rl).common_items;
    if let Some(p) = ci.iter().position(|&x| x == item) {
        let x = ci.remove(p);
        base::release(x);
    }
}

fn notify(rl: Ref, mode: usize, activity: u64) {
    let mut obs: Vec<Ref> = rl_mut(rl).modes[mode]
        .observers
        .iter()
        .copied()
        .filter(|&o| observer(o).valid && observer(o).activities & activity != 0)
        .collect();
    obs.sort_by_key(|&o| observer(o).order);
    for o in obs {
        base::retain(o);
        let ob = observer(o);
        if ob.valid {
            let (f, info, repeats) = (ob.callout, ob.ctx.info, ob.repeats);
            if let Some(f) = f {
                // SAFETY: the program's function.
                unsafe { f(o, activity, info) };
            }
            if !repeats {
                CFRunLoopObserverInvalidate(o);
            }
        }
        base::release(o);
    }
}

/// Perform the signalled sources of the mode by order; with `one`, only
/// the first. Whether any ran.
fn do_sources(rl: Ref, mode: usize, one: bool) -> bool {
    let mut ready: Vec<Ref> =
        rl_mut(rl).modes[mode].sources.iter().copied().filter(|&s| source(s).valid && source(s).signalled).collect();
    ready.sort_by_key(|&s| source(s).order);
    let mut any = false;
    for s in ready {
        base::retain(s);
        let src = source(s);
        if src.valid && src.signalled {
            src.signalled = false;
            let (f, info) = (src.ctx.perform, src.ctx.info);
            if let Some(f) = f {
                // SAFETY: the program's function.
                unsafe { f(info) };
            }
            any = true;
        }
        base::release(s);
        if any && one {
            break;
        }
    }
    any
}

fn any_signalled(rl: Ref, mode: usize) -> bool {
    rl_mut(rl).modes[mode].sources.iter().any(|&s| source(s).valid && source(s).signalled)
}

fn next_timer(rl: Ref, mode: usize) -> Option<f64> {
    rl_mut(rl).modes[mode].timers.iter().filter(|&&t| timer(t).valid).map(|&t| timer(t).next).reduce(f64::min)
}

/// Fire the timers of the mode that are due, earliest first.
fn do_timers(rl: Ref, mode: usize) {
    let now = crate::date::now();
    let mut due: Vec<Ref> =
        rl_mut(rl).modes[mode].timers.iter().copied().filter(|&t| timer(t).valid && timer(t).next <= now).collect();
    due.sort_by(|&a, &b| timer(a).next.total_cmp(&timer(b).next).then(timer(a).order.cmp(&timer(b).order)));
    for t in due {
        base::retain(t);
        let tm = timer(t);
        if tm.valid {
            let (f, info, interval) = (tm.callout, tm.ctx.info, tm.interval);
            if interval > 0.0 {
                let mut next = tm.next + interval;
                if next <= now {
                    // Missed firings are skipped, not repeated.
                    next += ((now - next) / interval).floor_like() * interval + interval;
                }
                tm.next = next;
            }
            if let Some(f) = f {
                // SAFETY: the program's function.
                unsafe { f(t, info) };
            }
            if interval <= 0.0 {
                CFRunLoopTimerInvalidate(t);
            }
        }
        base::release(t);
    }
}

/// `floor` without the standard library.
trait FloorLike {
    fn floor_like(self) -> f64;
}

impl FloorLike for f64 {
    fn floor_like(self) -> f64 {
        if !self.is_finite() || self.abs() >= 4_503_599_627_370_496.0 {
            return self;
        }
        let t = self as i64 as f64;
        if t > self { t - 1.0 } else { t }
    }
}

/// Sleep until `until` (absolute time) or a wake-up.
fn wait(rl: Ref, until: f64) {
    let l = rl_mut(rl);
    if l.wake_pending {
        l.wake_pending = false;
        return;
    }
    let left = until - crate::date::now();
    if left <= 0.0 {
        return;
    }
    let ms = (left * 1000.0).min(f64::from(u32::MAX - 1)) as u32 + 1;
    match l.port {
        Some(p) => {
            l.waiting = true;
            crate::sys::wait_port(p, ms);
            rl_mut(rl).waiting = false;
        }
        None => while crate::date::now() < until {},
    }
}

/// Run the current thread's loop in mode `name`.
fn run(name: Ref, seconds: f64, return_after: bool) -> i32 {
    let rl = main_loop();
    if name.is_null() || same_name(name, common_modes()) {
        crate::sys::stderr(
            alloc::format!(
                "CFRunLoopRunSpecific (pid {}): a run loop cannot run in kCFRunLoopCommonModes or a NULL mode\n",
                crate::sys::pid()
            )
            .as_bytes(),
        );
        return RUN_FINISHED;
    }
    let Some(mode) = mode_index(rl, name, false) else { return RUN_FINISHED };
    if is_empty(&rl_mut(rl).modes[mode]) {
        return RUN_FINISHED;
    }
    let start = crate::date::now();
    let deadline = if seconds > 0.0 { start + seconds.min(1e10) } else { start };
    let previous = rl_mut(rl).current;
    rl_mut(rl).current = rl_mut(rl).modes[mode].name;
    rl_mut(rl).stopped = false;
    notify(rl, mode, ENTRY);
    let result = loop {
        notify(rl, mode, BEFORE_TIMERS);
        notify(rl, mode, BEFORE_SOURCES);
        let handled = do_sources(rl, mode, return_after);
        if handled && return_after {
            break RUN_HANDLED_SOURCE;
        }
        if rl_mut(rl).stopped {
            break RUN_STOPPED;
        }
        let now = crate::date::now();
        let poll = handled || any_signalled(rl, mode) || seconds <= 0.0 || now >= deadline;
        if !poll {
            let until = next_timer(rl, mode).map_or(deadline, |t| t.min(deadline));
            notify(rl, mode, BEFORE_WAITING);
            wait(rl, until);
            notify(rl, mode, AFTER_WAITING);
        }
        do_timers(rl, mode);
        if rl_mut(rl).stopped {
            break RUN_STOPPED;
        }
        if seconds <= 0.0 || crate::date::now() >= deadline {
            break RUN_TIMED_OUT;
        }
        if is_empty(&rl_mut(rl).modes[mode]) {
            break RUN_FINISHED;
        }
    };
    notify(rl, mode, EXIT);
    rl_mut(rl).stopped = false;
    rl_mut(rl).current = previous;
    result
}

/// # Safety
/// The count reached zero.
pub unsafe fn destroy(cf: Ref) {
    // SAFETY: an object of one of the run loop types, made here.
    unsafe {
        match base::type_of(cf) {
            id::SOURCE => {
                let s = source(cf);
                if let Some(r) = s.ctx.release {
                    r(s.ctx.info);
                }
                base::destroy::<Source>(cf)
            }
            id::TIMER => {
                let t = timer(cf);
                if let Some(r) = t.ctx.release {
                    r(t.ctx.info);
                }
                base::destroy::<Timer>(cf)
            }
            id::OBSERVER => {
                let o = observer(cf);
                if let Some(r) = o.ctx.release {
                    r(o.ctx.info);
                }
                base::destroy::<Observer>(cf)
            }
            _ => {}
        }
    }
}

fn small_context(ctx: *const SmallContext) -> SmallContext {
    if ctx.is_null() {
        return NO_SMALL;
    }
    // SAFETY: the API's contract: a context.
    let mut c = unsafe { *ctx };
    if let Some(r) = c.retain {
        // SAFETY: the program's function.
        c.info = unsafe { r(c.info) } as *mut c_void;
    }
    c
}

// ----- CFRunLoop -----

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopGetTypeID() -> usize {
    id::RUN_LOOP
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopGetCurrent() -> Ref {
    main_loop()
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopGetMain() -> Ref {
    main_loop()
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopRunInMode(mode: Ref, seconds: f64, return_after: Boolean) -> i32 {
    run(mode, seconds, return_after != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopRun() {
    loop {
        let r = run(default_mode(), 1e10, false);
        if r == RUN_STOPPED || r == RUN_FINISHED {
            break;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopStop(rl: Ref) {
    let l = rl_mut(rl);
    if !l.current.is_null() {
        l.stopped = true;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopWakeUp(rl: Ref) {
    // v0 has one thread, so the call comes from the loop's own thread and
    // the loop is not waiting now: its next wait returns at once. From
    // another thread this becomes a message to the loop's port, once the
    // kernel can send (mach_msg2_trap, ADR 0238).
    rl_mut(rl).wake_pending = true;
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopIsWaiting(rl: Ref) -> Boolean {
    rl_mut(rl).waiting as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopCopyCurrentMode(rl: Ref) -> Ref {
    let c = rl_mut(rl).current;
    if c.is_null() { c } else { base::retain(c) }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopCopyAllModes(rl: Ref) -> Ref {
    let names: Vec<Ref> = rl_mut(rl).modes.iter().map(|m| m.name).collect();
    crate::array::from_refs(core::ptr::null(), &names, &crate::array::TYPE_CALLBACKS, false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopAddCommonMode(rl: Ref, name: Ref) {
    if rl_mut(rl).common.iter().any(|&n| same_name(n, name)) {
        return;
    }
    let n = string::CFStringCreateCopy(core::ptr::null(), name);
    rl_mut(rl).common.push(n);
    let items = rl_mut(rl).common_items.clone();
    for item in items {
        let k = match base::type_of(item) {
            id::SOURCE => Kind::Source,
            id::TIMER => Kind::Timer,
            _ => Kind::Observer,
        };
        add(rl, item, n, k);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopAddSource(rl: Ref, s: Ref, mode: Ref) {
    if source(s).valid {
        add(rl, s, mode, Kind::Source);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopRemoveSource(rl: Ref, s: Ref, mode: Ref) {
    remove(rl, s, mode, Kind::Source);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopContainsSource(rl: Ref, s: Ref, mode: Ref) -> Boolean {
    contains(rl, s, mode, Kind::Source) as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopAddTimer(rl: Ref, t: Ref, mode: Ref) {
    if timer(t).valid {
        add(rl, t, mode, Kind::Timer);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopRemoveTimer(rl: Ref, t: Ref, mode: Ref) {
    remove(rl, t, mode, Kind::Timer);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopContainsTimer(rl: Ref, t: Ref, mode: Ref) -> Boolean {
    contains(rl, t, mode, Kind::Timer) as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopAddObserver(rl: Ref, o: Ref, mode: Ref) {
    if observer(o).valid {
        add(rl, o, mode, Kind::Observer);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopRemoveObserver(rl: Ref, o: Ref, mode: Ref) {
    remove(rl, o, mode, Kind::Observer);
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopContainsObserver(rl: Ref, o: Ref, mode: Ref) -> Boolean {
    contains(rl, o, mode, Kind::Observer) as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopGetNextTimerFireDate(rl: Ref, mode: Ref) -> f64 {
    mode_index(rl, mode, false).and_then(|m| next_timer(rl, m)).unwrap_or(0.0)
}

// ----- CFRunLoopSource -----

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopSourceGetTypeID() -> usize {
    id::SOURCE
}

/// # Safety
/// `ctx` points to a version 0 `CFRunLoopSourceContext`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopSourceCreate(alloc: Ref, order: CFIndex, ctx: *const SourceContext) -> Ref {
    if ctx.is_null() {
        base::crash("CFRunLoopSourceCreate: NULL context");
    }
    // SAFETY: the caller's contract.
    let mut c = unsafe { *ctx };
    if let Some(r) = c.retain {
        // SAFETY: the program's function.
        c.info = unsafe { r(c.info) } as *mut c_void;
    }
    base::make(alloc, id::SOURCE, false, |h| Source { h, order, ctx: c, valid: true, signalled: false }) as Ref
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopSourceGetOrder(s: Ref) -> CFIndex {
    source(s).order
}

/// # Safety
/// `ctx` points to a context to fill.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopSourceGetContext(s: Ref, ctx: *mut SourceContext) {
    if !ctx.is_null() {
        // SAFETY: the caller's contract.
        unsafe { *ctx = source(s).ctx };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopSourceSignal(s: Ref) {
    let src = source(s);
    if src.valid {
        src.signalled = true;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopSourceIsValid(s: Ref) -> Boolean {
    source(s).valid as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopSourceInvalidate(s: Ref) {
    if !source(s).valid {
        return;
    }
    base::retain(s);
    remove_everywhere(s, Kind::Source);
    let src = source(s);
    src.valid = false;
    src.signalled = false;
    base::release(s);
}

// ----- CFRunLoopTimer -----

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerGetTypeID() -> usize {
    id::TIMER
}

/// # Safety
/// `ctx` is null or a `CFRunLoopTimerContext`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopTimerCreate(
    alloc: Ref,
    fire: f64,
    interval: f64,
    _flags: u64,
    order: CFIndex,
    callout: TimerCallout,
    ctx: *const SmallContext,
) -> Ref {
    let c = small_context(ctx);
    let interval = if interval > 0.0 { interval } else { 0.0 };
    base::make(alloc, id::TIMER, false, |h| Timer {
        h,
        next: fire,
        interval,
        tolerance: 0.0,
        order,
        callout,
        ctx: c,
        valid: true,
    }) as Ref
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerGetNextFireDate(t: Ref) -> f64 {
    timer(t).next
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerSetNextFireDate(t: Ref, fire: f64) {
    timer(t).next = fire;
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerGetInterval(t: Ref) -> f64 {
    timer(t).interval
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerDoesRepeat(t: Ref) -> Boolean {
    (timer(t).interval > 0.0) as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerGetOrder(t: Ref) -> CFIndex {
    timer(t).order
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerGetTolerance(t: Ref) -> f64 {
    timer(t).tolerance
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerSetTolerance(t: Ref, tolerance: f64) {
    timer(t).tolerance = if tolerance > 0.0 { tolerance } else { 0.0 };
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerIsValid(t: Ref) -> Boolean {
    timer(t).valid as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopTimerInvalidate(t: Ref) {
    if !timer(t).valid {
        return;
    }
    base::retain(t);
    timer(t).valid = false;
    remove_everywhere(t, Kind::Timer);
    base::release(t);
}

/// # Safety
/// `ctx` points to a context to fill.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopTimerGetContext(t: Ref, ctx: *mut SmallContext) {
    if !ctx.is_null() {
        // SAFETY: the caller's contract.
        unsafe { *ctx = timer(t).ctx };
    }
}

// ----- CFRunLoopObserver -----

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopObserverGetTypeID() -> usize {
    id::OBSERVER
}

/// # Safety
/// `ctx` is null or a `CFRunLoopObserverContext`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFRunLoopObserverCreate(
    alloc: Ref,
    activities: u64,
    repeats: Boolean,
    order: CFIndex,
    callout: ObserverCallout,
    ctx: *const SmallContext,
) -> Ref {
    let c = small_context(ctx);
    base::make(alloc, id::OBSERVER, false, |h| Observer {
        h,
        activities,
        repeats: repeats != 0,
        order,
        callout,
        ctx: c,
        valid: true,
    }) as Ref
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopObserverGetActivities(o: Ref) -> u64 {
    observer(o).activities
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopObserverDoesRepeat(o: Ref) -> Boolean {
    observer(o).repeats as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopObserverGetOrder(o: Ref) -> CFIndex {
    observer(o).order
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopObserverIsValid(o: Ref) -> Boolean {
    observer(o).valid as Boolean
}

#[unsafe(no_mangle)]
pub extern "C" fn CFRunLoopObserverInvalidate(o: Ref) {
    if !observer(o).valid {
        return;
    }
    base::retain(o);
    observer(o).valid = false;
    remove_everywhere(o, Kind::Observer);
    base::release(o);
}
