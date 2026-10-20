//! Networking: one interface, `eth0`, with the `poly-net` TCP/IP stack on
//! the virtio-net device (or on no device at all, which still gives
//! loopback), a network thread that drives it, and blocking sockets on
//! top ([`socket`]) for the system call layers.
//!
//! ```text
//!  socket::Socket ──call(op)──┐                       MSI-X entry 0
//!   (blocking: wait for a     ▼                             │ on_interrupt: WAKE += 1, wake the thread
//!    readiness change)   IFACE: Mutex<Interface>            ▼
//!         ▲              { Stack, Device }  ◄──poll──  network thread: poll, re-arm interrupts,
//!         │                                             sleep until WAKE moves or the stack's deadline
//!         └──── CHANGED += 1, wake waiters ◄────────────┘
//! ```
//!
//! - **The stack** (`poly_net::Stack`) never blocks and has no clock: every
//!   socket call only queues work, and [`poll`](poly_net::Stack::poll)
//!   moves frames and runs timers, returning the next deadline (ADR 0028).
//!   It lives with its device in one [`Interface`] behind a sleeping
//!   [`Mutex`]: the interrupt handler never touches either.
//! - **The network thread** polls when the device interrupts (MSI-X; the
//!   handler only counts and wakes it), when a socket call made the stack's
//!   deadline earlier, and at that deadline. Without MSI-X it also polls
//!   every [`POLL_INTERVAL_NS`].
//! - **Socket calls** go through [`call`]: the operation, then a `poll`
//!   right away (so a send leaves now, not at the thread's next turn), then
//!   [`CHANGED`] moves and every waiter re-checks its socket. A blocking
//!   call is a loop of "try, and if `EAGAIN` wait for the next change"
//!   ([`wait_for_change`]). The generation moves (with the interface
//!   locked) whenever a poll moved packets or ran a due timer, and each
//!   attempt returns the value it saw ([`call_and_generation`]), so a
//!   waiter misses no change made after its attempt and sleeps through
//!   polls that changed nothing.
//! - **Socket files** ([`file::InetSocket`], ADR 0048) are the sockets of
//!   user space: open files whose wait queues are woken per socket. After
//!   each of those polls, with the interface locked, [`notify`] asks the
//!   stack which sockets changed (`Stack::take_changed`) and, once the
//!   lock is released, wakes only their queues.
//! - **Configuration** comes from the command line (`poly.net=`, parsed by
//!   `poly_net::setting`): `poly.net=10.0.2.15/24,gw=10.0.2.2,dns=10.0.2.3`,
//!   `poly.net=qemu` for exactly that (QEMU's user networking), or
//!   `poly.net=off`. Without it the interface has no IPv4 address. IPv6
//!   gets its link-local address when there is a device.
//! - **Randomness** for the stack's secret (initial sequence numbers and
//!   port choice, RFC 6528) and for a MAC address when the device has none
//!   comes from `arch::user::random_bytes` (RDRAND, or the TSC mixed).

pub mod file;
pub mod notify;
pub mod socket;
mod tests;
mod virtio;

use crate::boot_info::BootInfo;
use crate::pci::MsixVectors;
use crate::sync::{Mutex, SetOnce};
use crate::thread::{self, ThreadId, WaitQueue};
use crate::{arch, kprintln, time};
use alloc::boxed::Box;
use core::net::Ipv4Addr;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use poly_net::{Config, Instant, MacAddr, NetDevice, Setting, Stack, StaticIpv4, TxError};
use poly_pci::Bdf;

pub use poly_net::{Domain, Errno, Protocol, Readiness, RecvFlags, RecvInfo, Shutdown, SockType, SocketHandle};

/// The interface's name.
pub const NAME: &str = "eth0";
/// Kernel command line key for the IPv4 configuration (see the module
/// documentation).
pub const SETTING_KEY: &str = "poly.net";
/// How often the network thread polls a device without interrupts.
pub const POLL_INTERVAL_NS: u64 = 10 * time::NANOS_PER_MILLI;

/// The device under the stack.
#[derive(Debug)]
pub enum Device {
    Virtio(Box<virtio::Driver>),
    /// No network device: the stack still loops back 127.0.0.0/8 and its
    /// own addresses; anything for the wire is dropped.
    None(NoDevice),
}

/// The stand-in for a missing network device.
#[derive(Debug, Default)]
pub struct NoDevice {
    /// Frames the stack tried to send.
    pub dropped: u64,
}

impl NetDevice for NoDevice {
    fn mac(&self) -> MacAddr {
        MacAddr([0; 6])
    }

    fn mtu(&self) -> usize {
        poly_virtio_net::DEFAULT_MTU
    }

    fn transmit(&mut self, _: &[u8]) -> Result<(), TxError> {
        self.dropped += 1;
        Ok(())
    }

    fn receive(&mut self, _: &mut [u8]) -> Option<usize> {
        None
    }
}

impl Device {
    /// Is the link up? Always false without a device.
    pub fn link_up(&self) -> bool {
        match self {
            Device::Virtio(d) => d.link_up(),
            Device::None(_) => false,
        }
    }

    /// The device broke its protocol and was reset.
    pub fn failed(&self) -> bool {
        matches!(self, Device::Virtio(d) if d.is_failed())
    }

    /// Ask for interrupts again (interrupt-driven devices); true if work
    /// arrived meanwhile and the stack must be polled before sleeping.
    fn rearm(&mut self) -> bool {
        match self {
            Device::Virtio(d) if INTERRUPT_DRIVEN.load(SeqCst) => d.enable_interrupts(),
            _ => false,
        }
    }
}

impl NetDevice for Device {
    fn mac(&self) -> MacAddr {
        match self {
            Device::Virtio(d) => d.mac(),
            Device::None(d) => d.mac(),
        }
    }

    fn mtu(&self) -> usize {
        match self {
            Device::Virtio(d) => d.mtu(),
            Device::None(d) => d.mtu(),
        }
    }

    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        match self {
            Device::Virtio(d) => d.transmit(frame),
            Device::None(d) => d.transmit(frame),
        }
    }

    fn receive(&mut self, buf: &mut [u8]) -> Option<usize> {
        match self {
            Device::Virtio(d) => d.receive(buf),
            Device::None(d) => d.receive(buf),
        }
    }

    fn can_transmit(&self) -> bool {
        match self {
            Device::Virtio(d) => d.can_transmit(),
            Device::None(d) => d.can_transmit(),
        }
    }
}

/// The stack and its device.
#[derive(Debug)]
pub struct Interface {
    pub stack: Stack,
    pub dev: Device,
}

impl Interface {
    /// Poll the stack. Also says whether packets moved (frames in or out,
    /// packets through the stack's loopback): only then, or when a timer
    /// was due, can socket readiness have changed.
    fn poll(&mut self) -> (Option<Instant>, bool) {
        POLLS.fetch_add(1, SeqCst);
        let before = self.traffic();
        let next = self.stack.poll(&mut self.dev, now());
        (next, self.traffic() != before)
    }

    fn traffic(&self) -> (u64, u64, u64) {
        let s = self.stack.stats();
        (s.rx_frames, s.tx_frames, s.loop_packets)
    }
}

/// What [`init`] set up, for `ifconfig` and the tests.
#[derive(Debug)]
pub struct Info {
    /// The virtio-net function, if there is one.
    pub bdf: Option<Bdf>,
    /// The static IPv4 configuration, if any.
    pub ipv4: Option<StaticIpv4>,
    /// The kernel vector of MSI-X entry 0, when interrupt-driven.
    pub vector: Option<u8>,
    pub msix: Option<MsixVectors>,
    pub thread: Option<ThreadId>,
}

static IFACE: Mutex<Option<Interface>> = Mutex::new(None);
static INFO: SetOnce<Info> = SetOnce::new();
/// Bumped by the interrupt handler and by [`kick`]: the network thread
/// sleeps until it moves (or its deadline).
static WAKE: AtomicU64 = AtomicU64::new(0);
static NET_WAIT: WaitQueue = WaitQueue::new();
/// When the network thread will poll next (monotonic ns; `u64::MAX`:
/// only when woken), or 0 while it is polling.
static PLANNED: AtomicU64 = AtomicU64::new(0);
static INTERRUPT_DRIVEN: AtomicBool = AtomicBool::new(false);
/// Moves whenever socket readiness may have changed.
static CHANGED: AtomicU64 = AtomicU64::new(0);
static CHANGE_WAIT: WaitQueue = WaitQueue::new();
static INTERRUPTS: AtomicU64 = AtomicU64::new(0);
static POLLS: AtomicU64 = AtomicU64::new(0);

/// The stack's clock: the monotonic clock in milliseconds.
pub fn now() -> Instant {
    Instant::from_millis(time::now_ns() / time::NANOS_PER_MILLI)
}

fn instant_ns(t: Instant) -> u64 {
    t.millis().saturating_mul(time::NANOS_PER_MILLI)
}

/// MSI-X handler: count, and wake the network thread. It touches neither
/// the stack nor the device (both sit behind a sleeping mutex).
fn on_interrupt(_: usize) {
    INTERRUPTS.fetch_add(1, SeqCst);
    kick();
}

/// Make the network thread poll soon.
pub fn kick() {
    WAKE.fetch_add(1, SeqCst);
    NET_WAIT.wake_all();
}

/// Record that socket readiness may have changed. Called with the
/// interface locked; wake the waiters with [`wake_waiters`] once it is
/// released.
fn mark_changed() {
    CHANGED.fetch_add(1, SeqCst);
}

fn wake_waiters() {
    CHANGE_WAIT.wake_all();
}

/// Block until the generation moves away from `seen` (socket readiness may
/// have changed) or the monotonic clock reaches `deadline_ns`. False on
/// the timeout. Any thread but the idle thread may wait.
pub fn wait_for_change(seen: u64, deadline_ns: Option<u64>) -> bool {
    let changed = || CHANGED.load(SeqCst) != seen;
    match deadline_ns {
        None => {
            CHANGE_WAIT.wait_until(changed);
            true
        }
        Some(d) => CHANGE_WAIT.wait_until_deadline(changed, d),
    }
}

/// Run a socket operation on the stack, then poll it (so what the operation
/// queued goes out now), wake the waiters if packets moved, and make sure
/// the network thread wakes for any earlier deadline. Panics before
/// [`init`].
pub fn call<R>(op: impl FnOnce(&mut Stack) -> R) -> R {
    call_and_generation(op).0
}

/// [`call`], also returning the readiness generation as of the operation:
/// when the result says "try again later", wait for a change from it
/// ([`wait_for_change`]). The generation is read with the interface
/// locked, after the operation but before the poll's own changes are
/// marked, so a waiter misses nothing that happened after its attempt and
/// is not woken by attempts that changed nothing. This is how a blocking
/// call (or `poll(2)` over several sockets) sleeps.
pub fn call_and_generation<R>(op: impl FnOnce(&mut Stack) -> R) -> (R, u64) {
    run_call(op, false)
}

/// [`call`] for operations that change readiness for other threads
/// waiting on the same socket (`shutdown`, `close`): always marks a change.
pub fn call_and_wake<R>(op: impl FnOnce(&mut Stack) -> R) -> R {
    run_call(op, true).0
}

fn run_call<R>(op: impl FnOnce(&mut Stack) -> R, changes: bool) -> (R, u64) {
    let (r, next, seen, moved, sockets) = {
        let mut guard = IFACE.lock();
        let iface = guard.as_mut().expect("net::init has not run");
        let r = op(&mut iface.stack);
        let (next, moved) = iface.poll();
        let seen = CHANGED.load(SeqCst);
        if moved || changes {
            mark_changed();
        }
        (r, next, seen, moved || changes, notify::collect(&mut iface.stack))
    };
    if moved {
        wake_waiters();
    }
    notify::wake(sockets);
    let planned = PLANNED.load(SeqCst);
    if planned == 0 || next.is_some_and(|t| instant_ns(t) < planned) {
        kick();
    }
    (r, seen)
}

/// Look at the interface without polling it (`ifconfig`). `None` before
/// [`init`].
pub fn with_interface<R>(f: impl FnOnce(&mut Interface) -> R) -> Option<R> {
    IFACE.lock().as_mut().map(f)
}

/// What [`init`] set up; `None` before.
pub fn info() -> Option<&'static Info> {
    INFO.get()
}

/// Counters of the kernel side: interrupts taken and polls run.
pub fn counters() -> (u64, u64) {
    (INTERRUPTS.load(SeqCst), POLLS.load(SeqCst))
}

/// Is the device interrupt-driven (else the network thread polls it)?
pub fn interrupt_driven() -> bool {
    INTERRUPT_DRIVEN.load(SeqCst)
}

/// The network thread: poll, re-arm the device's interrupts (polling again
/// if something arrived meanwhile), wake socket waiters if anything moved,
/// then sleep until an interrupt or a kick, or the stack's deadline.
fn network_thread(_: usize) -> usize {
    let mut link = None;
    let mut failed = false;
    let mut due = u64::MAX;
    let mut changed;
    loop {
        PLANNED.store(0, SeqCst);
        let seen = WAKE.load(SeqCst);
        let (next, link_up, dev_failed, sockets) = {
            let mut guard = IFACE.lock();
            let Some(iface) = guard.as_mut() else { return 1 };
            let (mut next, mut moved) = iface.poll();
            // Bounded: a device that keeps delivering is polled again at
            // once anyway (the stack then returns a deadline of now).
            for _ in 0..8 {
                if !iface.dev.rearm() {
                    break;
                }
                let (n, m) = iface.poll();
                (next, moved) = (n, moved || m);
            }
            let link_up = iface.dev.link_up();
            changed = moved || time::now_ns() >= due || link != Some(link_up);
            if changed {
                mark_changed();
            }
            (next, link_up, iface.dev.failed(), notify::collect(&mut iface.stack))
        };
        if changed {
            wake_waiters();
        }
        notify::wake(sockets);
        let now_ns = time::now_ns();
        if link.is_some_and(|l| l != link_up) {
            kprintln!("net: {NAME}: link {}", if link_up { "up" } else { "down" });
        }
        link = Some(link_up);
        if dev_failed && !failed {
            kprintln!("net: {NAME}: the device broke the virtqueue protocol and was reset; it is down for good");
            failed = true;
        }
        let mut deadline = next.map_or(u64::MAX, instant_ns);
        if !INTERRUPT_DRIVEN.load(SeqCst) && !failed && matches!(info(), Some(i) if i.bdf.is_some()) {
            deadline = deadline.min(now_ns.saturating_add(POLL_INTERVAL_NS));
        }
        due = deadline;
        PLANNED.store(deadline.max(1), SeqCst);
        if deadline <= now_ns {
            thread::yield_now();
        } else if deadline == u64::MAX {
            NET_WAIT.wait_until(|| WAKE.load(SeqCst) != seen);
        } else {
            NET_WAIT.wait_until_deadline(|| WAKE.load(SeqCst) != seen, deadline);
        }
    }
}

/// Bring up networking: read `poly.net=`, find and start the virtio-net
/// device (or none), build the stack, start the network thread and connect
/// the device's interrupt. Call once, after `pci::init`, with interrupts
/// and the scheduler running.
pub fn init(boot: &BootInfo) {
    let setting = match boot.arg(SETTING_KEY).map(poly_net::setting::parse) {
        None => None,
        Some(Ok(s)) => Some(s),
        Some(Err(e)) => {
            kprintln!("net: {SETTING_KEY}={}: {e:?}; no IPv4 address", boot.arg(SETTING_KEY).unwrap_or(""));
            None
        }
    };
    let random = arch::user::random_bytes();
    let secret = u64::from_le_bytes(random[..8].try_into().unwrap_or([0; 8]));
    let mac_seed: [u8; 6] = random[8..14].try_into().unwrap_or([0; 6]);
    let found = match setting {
        Some(Setting::Off) => {
            let any = crate::pci::devices().iter().any(virtio::is_virtio_net);
            kprintln!("net: {SETTING_KEY}=off{}", if any { ": the virtio-net device is left alone" } else { "" });
            None
        }
        _ => virtio::probe(mac_seed),
    };
    let ipv4 = match setting {
        Some(Setting::Static(s)) if found.is_some() => Some(s),
        _ => None,
    };
    let cfg = Config {
        ipv4: ipv4.map(|s| s.addr),
        gateway: ipv4.and_then(|s| s.gateway),
        ipv6: found.is_some(),
        secret,
        ..Config::default()
    };
    let (bdf, mut msix, dev) = match found {
        Some(f) => (Some(f.bdf), f.msix, Device::Virtio(Box::new(f.driver))),
        None => (None, None, Device::None(NoDevice::default())),
    };
    let stack = Stack::for_device(&dev, cfg);
    let ll = stack.ipv6_link_local();
    *IFACE.lock() = Some(Interface { stack, dev });
    let thread = match thread::spawn_named("net", network_thread, 0) {
        Ok(t) => Some(t),
        Err(e) => {
            kprintln!("net: cannot start the network thread: {e:?}; networking is off");
            None
        }
    };
    let mut vector = None;
    if let Some(m) = &mut msix {
        match virtio::connect_interrupt(m, on_interrupt, 0) {
            Ok(v) => {
                vector = Some(v);
                INTERRUPT_DRIVEN.store(true, SeqCst);
            }
            Err(e) => kprintln!("net: {NAME}: cannot connect the MSI-X interrupt ({e:?}); polled"),
        }
    }
    let _ = INFO.set(Info { bdf, ipv4, vector, msix, thread });
    // Collect whatever arrived before the interrupt was connected, and arm
    // it (the thread does both on its first turn).
    kick();
    match (bdf, vector) {
        (None, _) => kprintln!("net: no network device; loopback only"),
        (Some(bdf), Some(v)) => kprintln!(
            "net: {NAME}: virtio-net {bdf}; queues on MSI-X entry {} -> vector {v:#x}; interrupt-driven",
            virtio::MSIX_ENTRY
        ),
        (Some(bdf), None) => {
            kprintln!("net: {NAME}: virtio-net {bdf}; polled every {} ms", POLL_INTERVAL_NS / time::NANOS_PER_MILLI)
        }
    }
    if bdf.is_some() {
        match ipv4 {
            Some(s) => kprintln!(
                "net: {NAME}: inet {}/{}{}{}; inet6 {}",
                s.addr.addr,
                s.addr.prefix_len,
                s.gateway.map_or(alloc::string::String::new(), |g| alloc::format!(" gateway {g}")),
                s.dns.map_or(alloc::string::String::new(), |d| alloc::format!(" dns {d}")),
                Ipv6(ll),
            ),
            None => kprintln!(
                "net: {NAME}: no IPv4 address (add {SETTING_KEY}=<addr>/<prefix>[,gw=<addr>][,dns=<addr>] or {SETTING_KEY}=qemu); inet6 {}",
                Ipv6(ll)
            ),
        }
    }
}

/// Formats an optional link-local address as `fe80::.../64`.
struct Ipv6(Option<core::net::Ipv6Addr>);

impl core::fmt::Display for Ipv6 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(a) => write!(f, "{a}/64"),
            None => f.write_str("none"),
        }
    }
}

/// The DNS server from the configuration, for a resolver to come.
pub fn dns_server() -> Option<Ipv4Addr> {
    info().and_then(|i| i.ipv4).and_then(|s| s.dns)
}
