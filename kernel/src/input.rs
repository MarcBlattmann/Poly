//! The input event layer (ADR 0103): every keyboard, mouse and tablet, the
//! readers of `/dev/input/event*` and the screen console's keys.
//!
//! ```text
//!  PS/2 keyboard (IRQ 1) ─┐  report(dev, events): the hub, under an interrupt-safe lock
//!  PS/2 mouse (IRQ 12) ───┼─▶ poly_input::hub::Hub ──▶ client queues ──▶ /dev/input/eventN (read, poll, ioctl)
//!  virtio-input (MSI-X) ──┘          │                     ▲ woken by the input thread
//!                                    └─▶ console keys ──▶ keyboard::next_event (the screen terminal)
//!  input thread: soft repeat (Hub::tick), wakes event file readers, polls devices without interrupts
//! ```
//!
//! The logic is the host-tested `poly_input::hub` (devices, frames,
//! filtering, soft repeat, LEDs, grabs) and `poly_input::evdev` (the bytes
//! and ioctls of the event interface). This module gives it a lock,
//! wakeups and the device nodes:
//!
//! - Drivers [`register`] a device (with a hook that lights its LEDs) and
//!   [`report_at`] events from wherever they come, interrupt handlers
//!   included: the hub never sleeps or allocates while reporting.
//! - Readers of the console keys wait with [`wait_console`]; event file
//!   readers wait on a `poly_vfs` wait queue per device, which only the
//!   input thread notifies (its waiters may allocate and take locks that
//!   interrupt handlers must not).
//! - `/dev/input/event<n>` is character device 13, 64 + n, as the Linux
//!   device list (`Documentation/admin-guide/devices.txt`, "input core")
//!   gives it; each open is a client of the hub with a queue of its own.

use crate::sync::SpinLock;
use crate::thread::{self, WaitQueue};
use crate::{kprintln, time};
use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use poly_input::evdev::{self, Request};
use poly_input::hub::{self, ClientId, ConsoleKey, Delivered, DeviceInfo, Hub, QUEUE_EVENTS, Queue};
use poly_input::{Key, Leds};
use poly_vfs::device::CharDevice;
use poly_vfs::{At, Cred, DeviceId, Errno, FileType, FsContext, OpenFlags, PollEvents, Vfs};

/// Lights a device's LEDs: called with the registration's data and the
/// evdev LED bits (bit `n` for `LED_*` code `n`), in thread context.
pub type LedHook = fn(usize, u64);

static HUB: SpinLock<Hub> = SpinLock::new(Hub::new());

/// What the kernel keeps per device besides the hub's state.
struct Node {
    /// Where readers of `/dev/input/event<n>` wait.
    queue: Arc<poly_vfs::WaitQueue>,
    leds: Option<(LedHook, usize)>,
}

/// Indexed like the hub's devices; taken before the hub's lock when both
/// are needed, and never from interrupt handlers.
static NODES: SpinLock<Vec<Node>> = SpinLock::new(Vec::new());

/// Devices whose clients got events the input thread has not announced.
static DIRTY: AtomicU64 = AtomicU64::new(0);
/// The input thread has work (set before waking it).
static PENDING: AtomicBool = AtomicBool::new(false);
static WAKE: WaitQueue = WaitQueue::new();
/// The hub's next soft-repeat deadline (`u64::MAX`: none), stored under
/// its lock.
static NEXT_REPEAT: AtomicU64 = AtomicU64::new(u64::MAX);
/// Bumped whenever console keys arrive.
static CONSOLE_SEQ: AtomicU64 = AtomicU64::new(0);
static CONSOLE: WaitQueue = WaitQueue::new();
static STARTED: AtomicBool = AtomicBool::new(false);

/// The Linux input major and the first event device minor.
pub const INPUT_MAJOR: u32 = 13;
pub const EVDEV_MINOR_BASE: u32 = 64;
/// Event devices the minor range gives (64-95).
pub const MAX_EVDEV: usize = 32;
/// How often the input thread looks at devices without interrupts.
const POLL_NS: u64 = 10 * time::NANOS_PER_MILLI;

/// Start the input thread (repeats, wakeups). Once, after the scheduler
/// and the timer run, before any driver reports.
pub fn init() {
    if STARTED.swap(true, SeqCst) {
        return;
    }
    if let Err(e) = thread::spawn_named("input", input_thread, 0) {
        kprintln!("input: cannot start the input thread: {e:?}; no key repeat, event files not woken");
    }
}

/// Add a device to the hub; returns its number (`/dev/input/event<n>`).
pub fn register(info: DeviceInfo, leds: Option<(LedHook, usize)>) -> usize {
    let queue = Arc::new(poly_vfs::WaitQueue::new());
    let mut nodes = NODES.lock();
    let name = info.name.clone();
    let dev = HUB.lock().add(info);
    nodes.push(Node { queue, leds });
    drop(nodes);
    kprintln!("input: event{dev}: {name}");
    dev
}

/// Devices registered so far.
pub fn count() -> usize {
    HUB.lock().len()
}

/// The device's description, as registered (absolute axes with their
/// current values).
pub fn info(dev: usize) -> Option<DeviceInfo> {
    HUB.lock().info(dev).cloned()
}

/// The LEDs of `dev` that are on (evdev LED bits).
pub fn leds(dev: usize) -> u64 {
    HUB.lock().leds(dev)
}

/// Device `dev` reports `events` (each type, code, value), which happened
/// at `time_ns`. Interrupt handlers may call this.
pub fn report_at(dev: usize, time_ns: u64, events: &[(u16, u16, i32)]) {
    let (d, armed) = {
        let mut h = HUB.lock();
        let before = NEXT_REPEAT.load(SeqCst);
        let d = h.report_all(dev, time_ns, events);
        let next = h.next_deadline().unwrap_or(u64::MAX);
        NEXT_REPEAT.store(next, SeqCst);
        (d, next < before)
    };
    delivered(d, armed);
}

/// Wake whoever `d` concerns; `kick` the input thread regardless (a
/// repeat was armed).
fn delivered(d: Delivered, kick: bool) {
    if d.console {
        CONSOLE_SEQ.fetch_add(1, SeqCst);
        CONSOLE.wake_all();
    }
    if d.devices != 0 {
        DIRTY.fetch_or(d.devices, SeqCst);
    }
    if d.devices != 0 || kick {
        PENDING.store(true, SeqCst);
        WAKE.wake_all();
    }
}

// --- The console --------------------------------------------------------------

/// Counts console key arrivals: [`wait_console`] waits for it to move.
pub fn console_seq() -> u64 {
    CONSOLE_SEQ.load(SeqCst)
}

/// The oldest key for the console, if any.
pub fn console_pop() -> Option<ConsoleKey> {
    HUB.lock().console_pop()
}

/// Put a key in the console queue as if a keyboard had sent it (bytes
/// injected with no PS/2 keyboard to decode them for). False if full.
pub fn console_push(key: Key, value: i32) -> bool {
    let ok = HUB.lock().console_push(key, value, time::now_ns());
    delivered(Delivered { devices: 0, console: true }, false);
    ok
}

/// Console keys lost because nobody read them in time.
pub fn console_dropped() -> u64 {
    HUB.lock().console_dropped()
}

/// Sleep until console keys arrive after `seen` ([`console_seq`]) or the
/// clock reads `deadline_ns`: true if they did.
pub fn wait_console(seen: u64, deadline_ns: u64) -> bool {
    CONSOLE.wait_until_deadline(|| CONSOLE_SEQ.load(SeqCst) != seen, deadline_ns)
}

/// The console's lock state changed: light the lock LEDs of every console
/// keyboard to match. Thread context (the hooks may sleep).
pub fn set_console_leds(leds: Leds) {
    let bits = leds.to_evdev();
    let now = time::now_ns();
    let mut changed = Vec::new();
    let d = {
        let mut h = HUB.lock();
        let devs: Vec<usize> = h.console_keyboards().collect();
        let mut d = Delivered::default();
        for dev in devs {
            let (c, out) = h.set_leds(dev, bits, now);
            d = d.merge(out);
            if c {
                changed.push((dev, h.leds(dev)));
            }
        }
        d
    };
    delivered(d, false);
    for (dev, bits) in changed {
        light(dev, bits);
    }
}

/// Call the LED hook of `dev`, outside every lock.
fn light(dev: usize, bits: u64) {
    let hook = NODES.lock().get(dev).and_then(|n| n.leds);
    if let Some((f, data)) = hook {
        f(data, bits);
    }
}

/// Switch `dev` between the hub's repeat and the device's own (tests).
pub fn set_soft_repeat(dev: usize, soft: bool) -> bool {
    let mut h = HUB.lock();
    let ok = h.set_soft_repeat(dev, soft).is_ok();
    NEXT_REPEAT.store(h.next_deadline().unwrap_or(u64::MAX), SeqCst);
    ok
}

// --- The input thread ---------------------------------------------------------

fn input_thread(_: usize) -> usize {
    loop {
        let next = NEXT_REPEAT.load(SeqCst);
        let poll = crate::virtio_input::needs_polling();
        let deadline = if poll { next.min(time::now_ns().saturating_add(POLL_NS)) } else { next };
        WAKE.wait_until_deadline(|| PENDING.load(SeqCst) || NEXT_REPEAT.load(SeqCst) < next, deadline);
        PENDING.store(false, SeqCst);
        if poll {
            crate::virtio_input::poll_all();
        }
        let now = time::now_ns();
        let d = {
            let mut h = HUB.lock();
            let d = h.tick(now);
            NEXT_REPEAT.store(h.next_deadline().unwrap_or(u64::MAX), SeqCst);
            d
        };
        if d.console {
            CONSOLE_SEQ.fetch_add(1, SeqCst);
            CONSOLE.wake_all();
        }
        let dirty = DIRTY.swap(0, SeqCst) | d.devices;
        if dirty != 0 {
            notify(dirty);
        }
    }
}

/// Tell the readers of the devices in `mask` that events came.
fn notify(mask: u64) {
    let queues: Vec<Arc<poly_vfs::WaitQueue>> = NODES
        .lock()
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & Delivered::device_bit(*i) != 0)
        .map(|(_, n)| n.queue.clone())
        .collect();
    for q in queues {
        q.notify(PollEvents::READABLE);
    }
}

// --- /dev/input/event* --------------------------------------------------------

fn errno(e: hub::Error) -> Errno {
    match e {
        hub::Error::Again => Errno::EAGAIN,
        hub::Error::Invalid => Errno::EINVAL,
        hub::Error::NotTty => Errno::ENOTTY,
        hub::Error::Busy => Errno::EBUSY,
        hub::Error::NoDevice => Errno::ENODEV,
        hub::Error::NoEntry => Errno::ENOENT,
    }
}

/// The device number of `/dev/input/event<n>`.
pub fn evdev_id(n: usize) -> DeviceId {
    DeviceId::new(INPUT_MAJOR, EVDEV_MINOR_BASE + n as u32)
}

/// A node `/dev/input/event<n>`: each open makes an [`EventFile`].
struct EventNode(usize);

impl core::fmt::Debug for EventNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "EventNode(event{})", self.0)
    }
}

impl CharDevice for EventNode {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> poly_vfs::Result<usize> {
        Err(Errno::ENODEV)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> poly_vfs::Result<usize> {
        Err(Errno::ENODEV)
    }

    fn open(&self, _: OpenFlags) -> poly_vfs::Result<Option<Arc<dyn CharDevice>>> {
        let queue = Queue::new(QUEUE_EVENTS);
        let wait = NODES.lock().get(self.0).map(|n| n.queue.clone()).ok_or(Errno::ENODEV)?;
        let client = HUB.lock().open(self.0, queue).ok_or(Errno::ENODEV)?;
        Ok(Some(Arc::new(EventFile { dev: self.0, client, wait })))
    }
}

/// One open of an event device: a client of the hub.
struct EventFile {
    dev: usize,
    client: ClientId,
    wait: Arc<poly_vfs::WaitQueue>,
}

impl core::fmt::Debug for EventFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "EventFile(event{}, client {})", self.dev, self.client)
    }
}

impl Drop for EventFile {
    fn drop(&mut self) {
        let queue = HUB.lock().close(self.dev, self.client);
        // Freed outside the lock.
        drop(queue);
    }
}

impl CharDevice for EventFile {
    /// Whole events; `EAGAIN` when there are none (the file layer waits on
    /// [`wait_queue`](Self::wait_queue) for a blocking reader).
    fn read(&self, _: u64, buf: &mut [u8], _: bool) -> poly_vfs::Result<usize> {
        let offset = time::realtime_offset_ns();
        let mut h = HUB.lock();
        evdev::read(&mut h, self.dev, self.client, buf, offset).map_err(errno)
    }

    /// Whole events, injected into the device (an `EV_LED` lights its LED).
    fn write(&self, _: u64, buf: &[u8], _: bool) -> poly_vfs::Result<usize> {
        let now = time::now_ns();
        let (result, before, after, armed) = {
            let mut h = HUB.lock();
            let before = h.leds(self.dev);
            let old_next = NEXT_REPEAT.load(SeqCst);
            let result = evdev::write(&mut h, self.dev, self.client, buf, now);
            let next = h.next_deadline().unwrap_or(u64::MAX);
            NEXT_REPEAT.store(next, SeqCst);
            (result, before, h.leds(self.dev), next < old_next)
        };
        let (n, d) = result.map_err(errno)?;
        delivered(d, armed);
        if before != after {
            light(self.dev, after);
        }
        Ok(n)
    }

    fn ioctl(&self, request: u32, arg: u64) -> poly_vfs::Result<u64> {
        let req = Request::decode(request).ok_or(Errno::ENOTTY)?;
        let mut input = [0u8; 24];
        let n = req.input_len();
        if n > 0 {
            crate::user::copy_from_user(&mut input[..n], arg).map_err(|_| Errno::EFAULT)?;
        }
        let reply = {
            let mut h = HUB.lock();
            let r = evdev::ioctl(&mut h, self.dev, self.client, req, arg, &input[..n]);
            NEXT_REPEAT.store(h.next_deadline().unwrap_or(u64::MAX), SeqCst);
            r
        }
        .map_err(errno)?;
        if !reply.out.is_empty() {
            crate::user::copy_to_user(arg, &reply.out).map_err(|_| Errno::EFAULT)?;
        }
        Ok(reply.value)
    }

    fn poll(&self) -> PollEvents {
        match HUB.lock().pending(self.dev, self.client) {
            Ok(0) => PollEvents::WRITABLE,
            Ok(_) => PollEvents::READABLE | PollEvents::WRITABLE,
            Err(_) => PollEvents::ERR | PollEvents::HUP,
        }
    }

    fn wait_queue(&self) -> Option<&Arc<poly_vfs::WaitQueue>> {
        Some(&self.wait)
    }
}

/// Register the event devices and make `/dev/input/event<n>` for each
/// device registered so far (at boot, the drivers have run by then).
pub fn install_nodes(vfs: &Vfs, ctx: &FsContext) {
    let n = count().min(MAX_EVDEV);
    let root = Cred::root();
    match vfs.mkdir(ctx, &root, At::Cwd, b"/dev/input", 0o755) {
        Ok(()) | Err(Errno::EEXIST) => {}
        Err(e) => {
            kprintln!("input: /dev/input: {}; no event devices", e.name());
            return;
        }
    }
    let mut made = 0;
    for dev in 0..n {
        let id = evdev_id(dev);
        if vfs.register_char_device(id, Arc::new(EventNode(dev))).is_err() {
            continue;
        }
        let path = format!("/dev/input/event{dev}");
        // Readable by root only, as on Linux (where group `input` may read
        // as well): key presses are private.
        match vfs.mknod(ctx, &root, At::Cwd, path.as_bytes(), FileType::CharDevice, 0o660, id) {
            Ok(()) => made += 1,
            Err(Errno::EEXIST) => {}
            Err(e) => kprintln!("input: {path}: {}", e.name()),
        }
    }
    kprintln!(
        "input: {n} event device(s), {made} node(s) made in /dev/input (char {INPUT_MAJOR}, {EVDEV_MINOR_BASE}+n)"
    );
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use poly_input::codes::{ev, key, syn};

/// The events a kernel-side open of `/dev/input/event<dev>` reads now, as
/// (type, code, value).
pub fn read_events(files: &crate::fs::Files, fd: poly_vfs::Fd) -> Vec<(u16, u16, i32)> {
    let mut buf = [0u8; 24 * 32];
    let mut out = Vec::new();
    while let Ok(n) = files.read(fd, &mut buf) {
        if n == 0 {
            break;
        }
        for e in buf[..n].chunks_exact(evdev::EVENT_SIZE) {
            out.push(evdev::decode(e.try_into().unwrap_or(&[0; 24])));
        }
    }
    out
}

/// Open the event file of `dev` from the kernel.
pub fn open_event_file(files: &crate::fs::Files, dev: usize) -> Result<poly_vfs::Fd, &'static str> {
    let path = format!("/dev/input/event{dev}");
    files.open(path.as_bytes(), OpenFlags::RDWR | OpenFlags::NONBLOCK, 0).map_err(|e| {
        kprintln!("input test: {path}: {}", e.name());
        "cannot open the event file"
    })
}

kernel_test! {
    fn event_files_exist_for_every_device() -> TestResult {
        let files = crate::fs::Files::new();
        for dev in 0..count().min(MAX_EVDEV) {
            let path = format!("/dev/input/event{dev}");
            let st = files.stat(path.as_bytes()).map_err(|_| "an event device has no node")?;
            // S_IFCHR (sys/stat.h).
            if st.mode & 0o170_000 != 0o020_000 || st.rdev != evdev_id(dev) {
                return Err("an event node is not char 13, 64+n");
            }
        }
        Ok(())
    }
}

kernel_test! {
    fn event_file_reads_keys_and_is_woken() -> TestResult {
        let Some(dev) = crate::keyboard::device() else { return Ok(()) };
        crate::keyboard::drain_events();
        let files = crate::fs::Files::new();
        let fd = open_event_file(&files, dev)?;
        let file = files.file(fd).map_err(|_| "no open file")?;
        if file.poll().contains(PollEvents::IN) {
            return Err("a fresh event file is readable");
        }
        // A thread waiting on the file's queue is woken by the input thread.
        let woken = Arc::new(AtomicBool::new(false));
        let waker: Arc<dyn poly_vfs::Waker> = Arc::new(Flag(woken.clone()));
        let _registration = file.wait_queue().ok_or("no wait queue")?.register(PollEvents::READABLE, waker);
        // A down, A up as the keyboard sends them (set 1).
        crate::keyboard::inject(&[0x1e, 0x9e]);
        let deadline = time::now_ns() + time::NANOS_PER_SEC;
        while !woken.load(SeqCst) && time::now_ns() < deadline {
            thread::yield_now();
        }
        let events = read_events(&files, fd);
        crate::keyboard::drain_events();
        let _ = files.close(fd);
        let want = [(ev::KEY, key::A, 1), (ev::SYN, syn::REPORT, 0), (ev::KEY, key::A, 0), (ev::SYN, syn::REPORT, 0)];
        if events != want {
            kprintln!("input test: read {events:?}");
            return Err("the event file did not read A down and up");
        }
        if !woken.load(SeqCst) {
            return Err("the event file's readers were not woken");
        }
        Ok(())
    }
}

/// A waker that sets a flag.
struct Flag(Arc<AtomicBool>);

impl poly_vfs::Waker for Flag {
    fn wake(&self, _: PollEvents) {
        self.0.store(true, SeqCst);
    }
}

kernel_test! {
    fn a_grab_keeps_keys_from_the_console() -> TestResult {
        let Some(dev) = crate::keyboard::device() else { return Ok(()) };
        crate::keyboard::drain_events();
        let files = crate::fs::Files::new();
        let fd = open_event_file(&files, dev)?;
        let file = files.file(fd).map_err(|_| "no open file")?;
        // EVIOCGRAB takes its argument by value: no user memory involved.
        file.ioctl(evdev::EVIOCGRAB, 1).map_err(|_| "EVIOCGRAB failed")?;
        crate::keyboard::inject(&[0x30, 0xb0]);
        let console_got = crate::keyboard::try_next_event().is_some();
        file.ioctl(evdev::EVIOCGRAB, 0).map_err(|_| "releasing the grab failed")?;
        let events = read_events(&files, fd);
        let _ = files.close(fd);
        crate::keyboard::drain_events();
        if console_got {
            return Err("the console got keys from a grabbed keyboard");
        }
        if events.first() != Some(&(ev::KEY, key::B, 1)) {
            return Err("the grabbing client did not get the keys");
        }
        Ok(())
    }
}

kernel_test! {
    fn soft_repeat_runs_on_the_input_thread() -> TestResult {
        let Some(dev) = crate::keyboard::device() else { return Ok(()) };
        if !set_soft_repeat(dev, true) {
            return Err("the keyboard has no EV_REP");
        }
        crate::keyboard::drain_events();
        let files = crate::fs::Files::new();
        let fd = open_event_file(&files, dev)?;
        // Hold C for longer than the delay (250 ms) and a period (33 ms).
        crate::keyboard::inject(&[0x2e]);
        thread::sleep_until(time::now_ns() + 400 * time::NANOS_PER_MILLI);
        crate::keyboard::inject(&[0xae]);
        let events = read_events(&files, fd);
        let _ = files.close(fd);
        set_soft_repeat(dev, false);
        crate::keyboard::drain_events();
        let repeats = events.iter().filter(|e| **e == (ev::KEY, key::C, 2)).count();
        kprintln!("input: {repeats} soft repeat(s) of C held for 400 ms");
        if repeats == 0 || events.first() != Some(&(ev::KEY, key::C, 1)) || events.iter().rev().nth(1) != Some(&(ev::KEY, key::C, 0)) {
            kprintln!("input test: read {events:?}");
            return Err("holding a key did not repeat it");
        }
        Ok(())
    }
}
