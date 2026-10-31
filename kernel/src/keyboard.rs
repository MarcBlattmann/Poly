//! The PS/2 keyboard behind the i8042 controller (ADR 0020), and the
//! console's keys from every keyboard (ADR 0103).
//!
//! ```text
//!  IRQ 1 ─▶ on_irq: read the data port ─▶ Set1Decoder ─▶ input::report (event layer)
//!  IRQ 12 ─┘ (mouse bytes ─▶ mouse::on_byte)                   │ console keys, from every keyboard
//!                                                               ▼
//!  next_event / try_next_event (thread context) ◀── input::console_pop
//!     poly_input::Keyboard: modifiers and locks, layout + dead keys
//!     the lock LEDs of every keyboard follow the locks (input::set_console_leds;
//!     the PS/2 one with 0xED, acknowledged through the interrupt handler)
//! ```
//!
//! The controller is brought up polled by `poly_input::i8042::init` (both
//! ports disabled, buffer flushed, self-tests, keyboard reset, scancode set
//! 2 with the controller translating to set 1), then the mouse on its
//! second port (`i8042::init_aux`, see `mouse.rs`); then ISA IRQ 1 (and
//! 12) are routed through the I/O APIC and the interrupts enabled. The
//! interrupt handler decodes the bytes into key events for the event
//! layer, which queues them for `/dev/input/event*` readers and for the
//! console; the console reader turns them into characters. A keyboard
//! command (the LEDs) is answered through the handler too: while one is in
//! flight, it hands the acknowledgement to the waiting thread.
//!
//! The layout is US unless the command line says `poly.keymap=de-ch` (or
//! `de`); [`set_layout`] changes it. Num Lock starts on.

use crate::arch::i8042::I8042Ports;
use crate::boot_info::BootInfo;
use crate::sync::{Mutex, SpinLock};
use crate::thread::WaitQueue;
use crate::{acpi, arch, input, kprintln, time};
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU64, AtomicUsize, Ordering::SeqCst};
use poly_input::codes::{bus, ev, syn};
use poly_input::hub::{DeviceInfo, InputId};
use poly_input::i8042::{self, ACK, RESEND};
use poly_input::layout::{self, Layout};
use poly_input::{Keyboard, Leds, ScanEvent, Set1Decoder, keycode};

/// What [`next_event`] returns, re-exported for the terminal.
pub use poly_input::{Input, Key, KeyEvent, Modifiers, Output, Special};

/// The ISA IRQ of the controller's first (keyboard) port.
const KEYBOARD_IRQ: u8 = 1;
/// How often a waiting reader looks at the controller itself, in case an
/// interrupt was lost (an edge while the I/O APIC input was masked).
const POLL_NS: u64 = 100 * time::NANOS_PER_MILLI;
/// The event layer's name for the keyboard.
pub const DEVICE_NAME: &str = "PS/2 Keyboard";

/// The decoder, used by one producer at a time: the interrupt handler, or
/// a thread polling the controller or injecting bytes, on any CPU. They
/// also share the controller's ports.
static PRODUCER: SpinLock<Set1Decoder> = SpinLock::new(Set1Decoder::new());
static PRESENT: AtomicBool = AtomicBool::new(false);
/// The keyboard's device in the event layer (`usize::MAX`: none).
static DEVICE: AtomicUsize = AtomicUsize::new(usize::MAX);
/// The vector IRQ 1 arrives on (0: not routed).
static VECTOR: AtomicU8 = AtomicU8::new(0);

/// A keyboard command is waiting for its reply.
static AWAITING_REPLY: AtomicBool = AtomicBool::new(false);
/// The reply, `REPLIED | byte`, or 0.
static REPLY: AtomicU16 = AtomicU16::new(0);
const REPLIED: u16 = 0x100;
/// Woken when the reply arrives.
static REPLIES: WaitQueue = WaitQueue::new();

static IRQS: AtomicU64 = AtomicU64::new(0);
/// Bytes decoded (from the keyboard or injected).
static BYTES: AtomicU64 = AtomicU64::new(0);
static AUX_BYTES: AtomicU64 = AtomicU64::new(0);
static LED_UPDATES: AtomicU64 = AtomicU64::new(0);
static LED_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Serialises writes to the controller (a command and its data byte
/// belong together); held only for the write, with interrupts off.
pub(crate) static CONTROLLER: SpinLock<()> = SpinLock::new(());

/// The console's state, used by whoever reads its keys.
struct Driver {
    keyboard: Keyboard,
    /// What the keyboards' LEDs show.
    leds: Leds,
}

static DRIVER: Mutex<Driver> = Mutex::new(Driver { keyboard: Keyboard::new(&layout::US), leds: Leds(0) });

/// Find the controller and bring up the keyboard (and the mouse); logs
/// what happened. The kernel runs on without them if there are none.
pub fn init(boot: &BootInfo) {
    let layout = match boot.arg("poly.keymap") {
        None => &layout::US,
        Some(name) => layout::by_name(name).unwrap_or_else(|| {
            kprintln!("keyboard: unknown layout \"{name}\" (poly.keymap=us, de-ch or de); using us");
            &layout::US
        }),
    };
    {
        let mut d = DRIVER.lock();
        d.keyboard.set_layout(layout);
        d.keyboard.set_locks(Modifiers::NUM_LOCK);
    }
    bring_up(layout);
    // The lock LEDs of every keyboard there is (PS/2 or virtio).
    DRIVER.lock().sync_leds();
}

fn bring_up(layout: &Layout) {
    // ACPI 2.0+ firmware says whether the 8042 exists; without that, the
    // probe finds out (a missing controller reads 0xFF).
    if acpi::platform().and_then(|p| p.has_8042) == Some(false) {
        kprintln!("keyboard: no 8042 controller (ACPI FADT); no PS/2 keyboard");
        return;
    }
    let mut setup = match i8042::init(&I8042Ports) {
        Ok(s) => s,
        Err(e) => {
            kprintln!("keyboard: i8042: {e:?}; no PS/2 keyboard");
            return;
        }
    };
    let aux = i8042::init_aux(&I8042Ports, &mut setup);
    let vector = match arch::irq::route_isa_irq(KEYBOARD_IRQ, on_irq, 0) {
        Ok(v) => v,
        Err(e) => {
            kprintln!("keyboard: cannot route ISA IRQ {KEYBOARD_IRQ}: {e:?}; no PS/2 keyboard");
            return;
        }
    };
    let gsi = arch::apic::isa_route(KEYBOARD_IRQ).map_or(u32::from(KEYBOARD_IRQ), |r| r.gsi);
    let id = InputId { bustype: bus::I8042, vendor: 0, product: 1, version: 0 };
    let mut info = DeviceInfo::keyboard(DEVICE_NAME, id);
    info.phys = "i8042/serio0".into();
    DEVICE.store(input::register(info, Some((set_leds_hook, 0))), SeqCst);
    let mouse = crate::mouse::attach(aux, &mut setup);
    let enabled = {
        let _c = CONTROLLER.lock();
        i8042::enable_interrupts(&I8042Ports, &setup)
    };
    if let Err(e) = enabled {
        let _ = arch::irq::unroute_gsi(gsi, vector);
        kprintln!("keyboard: i8042: enabling the interrupt: {e:?}; no PS/2 keyboard");
        return;
    }
    VECTOR.store(vector, SeqCst);
    PRESENT.store(true, SeqCst);
    // A byte that came in before the interrupt was on raised no edge.
    poll_controller();
    kprintln!(
        "keyboard: i8042 config {:#04x} -> {:#04x}, {} stale byte(s) flushed; keyboard reset ok, scancode set 2{} translated to set 1",
        setup.firmware_config,
        setup.config,
        setup.flushed,
        if setup.set2_acknowledged { "" } else { " (not acknowledged)" },
    );
    kprintln!(
        "keyboard: ISA IRQ {KEYBOARD_IRQ} -> GSI {gsi} -> vector {vector:#x}; layout {} ({}), Num Lock on",
        layout.name,
        layout.description
    );
    if let Some(m) = mouse {
        m.log();
    }
}

/// Light the lock LEDs of keyboards that came after [`init`] (virtio
/// ones) as the console's locks are.
pub fn refresh_leds() {
    let leds = DRIVER.lock().leds;
    input::set_console_leds(leds);
}

/// Whether a PS/2 keyboard was found and is interrupt-driven.
pub fn is_present() -> bool {
    PRESENT.load(SeqCst)
}

/// The PS/2 keyboard's device in the event layer.
pub fn device() -> Option<usize> {
    let d = DEVICE.load(SeqCst);
    (d != usize::MAX).then_some(d)
}

/// The next console key event (presses and releases, from any keyboard)
/// and what it types, if one is buffered; never sleeps for input.
pub fn try_next_event() -> Option<Input> {
    let mut d = DRIVER.lock();
    let k = input::console_pop()?;
    let input = d.keyboard.apply(ScanEvent { key: k.key, pressed: k.value != 0 });
    d.sync_leds();
    Some(input)
}

/// The next key event, sleeping until one arrives. Without a keyboard that
/// is never (unless bytes are [`inject`]ed).
pub fn next_event() -> Input {
    loop {
        if let Some(input) = next_event_until(u64::MAX) {
            return input;
        }
    }
}

/// The next key event, sleeping until one arrives or the clock reads
/// `deadline_ns`.
pub fn next_event_until(deadline_ns: u64) -> Option<Input> {
    loop {
        let seen = input::console_seq();
        if let Some(input) = try_next_event() {
            return Some(input);
        }
        let now = time::now_ns();
        if now >= deadline_ns {
            return None;
        }
        let wake = deadline_ns.min(now.saturating_add(POLL_NS));
        if !input::wait_console(seen, wake) {
            poll_controller();
        }
    }
}

/// Like [`next_event_until`], for a reader that runs alongside the kernel
/// tests (the screen terminal's shell): while tests run (`ktest::running`),
/// it takes no events, since the keyboard tests read the keyboard
/// themselves, and just waits for the deadline. Whether tests run is
/// checked under the console's lock, right before taking an event.
pub fn next_shared_event_until(deadline_ns: u64) -> Option<Input> {
    let testing = crate::ktest::running;
    loop {
        let seen = input::console_seq();
        {
            let mut d = DRIVER.lock();
            if !testing()
                && let Some(k) = input::console_pop()
            {
                let input = d.keyboard.apply(ScanEvent { key: k.key, pressed: k.value != 0 });
                d.sync_leds();
                return Some(input);
            }
        }
        let now = time::now_ns();
        if now >= deadline_ns {
            return None;
        }
        let wake = deadline_ns.min(now.saturating_add(POLL_NS));
        if testing() {
            crate::thread::sleep_until(wake);
        } else if !input::wait_console(seen, wake) {
            poll_controller();
        }
    }
}

/// Switch the layout. A pending dead key is dropped.
pub fn set_layout(layout: &'static Layout) {
    DRIVER.lock().keyboard.set_layout(layout);
}

pub fn layout() -> &'static Layout {
    DRIVER.lock().keyboard.layout()
}

/// The modifiers held and the locks on, as of the last event read.
pub fn modifiers() -> Modifiers {
    DRIVER.lock().keyboard.modifiers()
}

/// Put bytes into the input path as if the PS/2 keyboard had sent them
/// (set 1 scancodes): for tests, and for feeding key presses from
/// elsewhere. Without a PS/2 keyboard they go to the console only. False
/// if the console's buffer was full and keys were dropped.
pub fn inject(bytes: &[u8]) -> bool {
    let dropped = input::console_dropped();
    {
        let mut decoder = PRODUCER.lock();
        let now = time::now_ns();
        for &b in bytes {
            decode(&mut decoder, b, now);
        }
    }
    input::console_dropped() == dropped
}

/// Decode one keyboard byte into the event layer.
fn decode(decoder: &mut Set1Decoder, byte: u8, now: u64) {
    if let Some(scan) = decoder.feed(byte) {
        let value = i32::from(scan.pressed);
        match device() {
            Some(dev) => {
                input::report_at(dev, now, &[(ev::KEY, keycode::code(scan.key), value), (ev::SYN, syn::REPORT, 0)])
            }
            None => {
                input::console_push(scan.key, value);
            }
        }
    }
    // Counted once the byte's event is in the event layer.
    BYTES.fetch_add(1, SeqCst);
}

/// Counters for the boot log and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub interrupts: u64,
    /// Bytes decoded (from the keyboard or injected).
    pub bytes: u64,
    /// Console keys lost because the buffer was full.
    pub dropped: u64,
    /// Bytes from the second (mouse) port.
    pub aux_bytes: u64,
    pub led_updates: u64,
    pub led_failures: u64,
}

pub fn stats() -> Stats {
    Stats {
        interrupts: IRQS.load(SeqCst),
        bytes: BYTES.load(SeqCst),
        dropped: input::console_dropped(),
        aux_bytes: AUX_BYTES.load(SeqCst),
        led_updates: LED_UPDATES.load(SeqCst),
        led_failures: LED_FAILURES.load(SeqCst),
    }
}

impl Driver {
    /// Make the keyboards' LEDs show the locks, if they do not.
    fn sync_leds(&mut self) {
        let want = self.keyboard.leds();
        if want == self.leds {
            return;
        }
        self.leds = want;
        input::set_console_leds(want);
    }
}

/// The event layer lights the PS/2 keyboard's LEDs (evdev LED bits).
fn set_leds_hook(_: usize, bits: u64) {
    if !is_present() {
        return;
    }
    let want = Leds::from_evdev(bits);
    // Not retried on failure: the next change tries again.
    if let Err(why) = keyboard_command(&[i8042::KBD_SET_LEDS, want.0]) {
        if LED_FAILURES.fetch_add(1, SeqCst) < 4 {
            kprintln!("keyboard: setting the LEDs to {:#05b} failed: {why}", want.0);
        }
    } else {
        LED_UPDATES.fetch_add(1, SeqCst);
    }
}

/// Send bytes to the keyboard, each acknowledged before the next, with the
/// interrupt on: the handler passes the reply over. Sleeps while waiting.
fn keyboard_command(bytes: &[u8]) -> Result<(), &'static str> {
    for &byte in bytes {
        let mut tries = 0;
        loop {
            REPLY.store(0, SeqCst);
            AWAITING_REPLY.store(true, SeqCst);
            let written = {
                let _c = CONTROLLER.lock();
                i8042::write_data(&I8042Ports, byte)
            };
            if written.is_err() {
                AWAITING_REPLY.store(false, SeqCst);
                return Err("the controller did not take the byte");
            }
            let deadline = time::now_ns() + i8042::KEYBOARD_TIMEOUT_NS;
            if !REPLIES.wait_until_deadline(|| REPLY.load(SeqCst) != 0, deadline) {
                // The interrupt may have been lost: look once more.
                poll_controller();
            }
            AWAITING_REPLY.store(false, SeqCst);
            match REPLY.load(SeqCst) {
                r if r == REPLIED | u16::from(ACK) => break,
                r if r == REPLIED | u16::from(RESEND) && tries < i8042::RETRIES => tries += 1,
                0 => return Err("no reply from the keyboard"),
                _ => return Err("the keyboard refused the command"),
            }
        }
    }
    Ok(())
}

/// Decode every byte waiting in the controller into the event layer (or
/// hand a command's reply over); mouse bytes go to the mouse. Runs in the
/// interrupt handlers, or with interrupts off.
fn drain() {
    let mut decoder = PRODUCER.lock();
    let mut reply = false;
    let now = time::now_ns();
    // Bounded: a controller that always reports a byte cannot hold us here.
    for _ in 0..16 {
        match i8042::read_keyboard_byte(&I8042Ports) {
            None => break,
            Some(Err(b)) => {
                AUX_BYTES.fetch_add(1, SeqCst);
                crate::mouse::on_byte(b, now);
            }
            Some(Ok(b)) if (b == ACK || b == RESEND) && AWAITING_REPLY.load(SeqCst) => {
                REPLY.store(REPLIED | u16::from(b), SeqCst);
                reply = true;
            }
            Some(Ok(b)) => decode(&mut decoder, b, now),
        }
    }
    if reply {
        REPLIES.wake_all();
    }
}

/// The handler of IRQ 1 and IRQ 12: both read the same controller.
pub(crate) fn on_irq(_: usize) {
    IRQS.fetch_add(1, SeqCst);
    drain();
}

/// Read the controller from thread context, as the interrupt handler would.
pub(crate) fn poll_controller() {
    if !is_present() {
        return;
    }
    drain();
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use alloc::string::String;
use alloc::vec::Vec;
use poly_input::set1::encode;

/// Kernel flag: `poly.test.keyboard=<text>` makes
/// `typed_text_matches_the_host_keys` wait for keys from the host (`cargo
/// xtask test` sends them through the QEMU monitor) and expect `<text>`
/// followed by Enter.
pub const TEST_KEYBOARD_FLAG: &str = "poly.test.keyboard";
/// Printed when that test starts waiting.
pub const TEST_READY: &str = "keyboard: ready for test input";

/// Throw away whatever is buffered (no key is typed during the tests).
pub(crate) fn drain_events() {
    while try_next_event().is_some() {}
}

/// The set 1 bytes for pressing and releasing each key in turn, with
/// `held` down around each.
fn tap_bytes(held: &[Key], keys: &[Key]) -> Vec<u8> {
    let mut v = Vec::new();
    for &k in keys {
        held.iter().for_each(|&h| v.extend_from_slice(&encode(h, true)));
        v.extend_from_slice(&encode(k, true));
        v.extend_from_slice(&encode(k, false));
        held.iter().rev().for_each(|&h| v.extend_from_slice(&encode(h, false)));
    }
    v
}

/// Read events until the buffer is empty; returns the text typed (special
/// keys as `<Name>`).
fn read_text() -> String {
    let mut s = String::new();
    while let Some(input) = try_next_event() {
        match input.output {
            Output::Special(sp) => s.push_str(&alloc::format!("<{sp:?}>")),
            o => s.extend(o.chars()),
        }
    }
    s
}

kernel_test! {
    fn controller_found_and_irq_routed() -> TestResult {
        if acpi::platform().and_then(|p| p.has_8042) != Some(true) {
            return Ok(()); // nothing to expect
        }
        if !is_present() {
            return Err("the FADT reports an 8042, but no keyboard was brought up");
        }
        let vector = VECTOR.load(SeqCst);
        let route = arch::apic::isa_route(KEYBOARD_IRQ).ok_or("ISA IRQ 1 has no route")?;
        let entry = arch::apic::gsi_entry(route.gsi).ok_or("no redirection entry for IRQ 1's GSI")?;
        if entry.vector() != vector || entry.is_masked() {
            return Err("IRQ 1's redirection entry is masked or points elsewhere");
        }
        if device().and_then(input::info).is_none_or(|i| i.name != DEVICE_NAME) {
            return Err("the keyboard is not a device of the event layer");
        }
        Ok(())
    }
}

kernel_test! {
    fn decodes_injected_bytes() -> TestResult {
        drain_events();
        let before = layout();
        set_layout(&layout::US);
        let mut bytes = tap_bytes(&[Key::ShiftLeft], &[Key::P]);
        bytes.extend(tap_bytes(&[], &[Key::O, Key::L, Key::Y, Key::Enter, Key::ArrowLeft]));
        bytes.extend(tap_bytes(&[Key::ControlLeft], &[Key::C]));
        if !inject(&bytes) {
            return Err("the input buffer overflowed");
        }
        let us = read_text();
        set_layout(&layout::DE_CH);
        // z (the Y key), AltGr-2 = @, ^ then a = â, ¨ then u = ü, AltGr-~ then n = ñ.
        let mut bytes = tap_bytes(&[], &[Key::Y]);
        bytes.extend(tap_bytes(&[Key::AltRight], &[Key::Digit2]));
        bytes.extend(tap_bytes(&[], &[Key::Equal, Key::A, Key::BracketRight, Key::U]));
        bytes.extend(tap_bytes(&[Key::AltRight], &[Key::Equal]));
        bytes.extend(tap_bytes(&[], &[Key::N]));
        inject(&bytes);
        let de_ch = read_text();
        set_layout(&layout::DE);
        // z, AltGr-Q = @, ß, ´ then e = é.
        let mut bytes = tap_bytes(&[], &[Key::Y]);
        bytes.extend(tap_bytes(&[Key::AltRight], &[Key::Q]));
        bytes.extend(tap_bytes(&[], &[Key::Minus, Key::Equal, Key::E]));
        inject(&bytes);
        let de = read_text();
        set_layout(before);
        crate::kprintln!("keyboard: injected: us {us:?}, de-ch {de_ch:?}, de {de:?}");
        if us != "Poly<Enter><Left>\u{3}" || de_ch != "z@âüñ" || de != "z@ßé" {
            return Err("injected scancodes did not decode to the expected text");
        }
        Ok(())
    }
}

kernel_test! {
    fn key_events_carry_modifiers() -> TestResult {
        drain_events();
        inject(&[0x1d, 0x1e]); // Ctrl down, A down
        let ctrl = try_next_event().ok_or("no event for Ctrl")?;
        // Buffered already, so this does not sleep.
        let a = next_event();
        inject(&[0x9e, 0x9d]);
        drain_events();
        let want = KeyEvent { key: Key::A, pressed: true, repeat: false, modifiers: Modifiers::CONTROL | Modifiers::NUM_LOCK };
        if ctrl.event.key != Key::ControlLeft || a.event != want || a.output != Output::Char('\u{1}') {
            return Err("Ctrl-A did not come out as Ctrl, then A with CONTROL");
        }
        if modifiers() != Modifiers::NUM_LOCK {
            return Err("a modifier stayed down after its release");
        }
        Ok(())
    }
}

kernel_test! {
    fn irq_delivers_controller_bytes() -> TestResult {
        if !is_present() {
            return Ok(());
        }
        drain_events();
        let before = stats();
        // The controller puts these in its output buffer as if the keyboard
        // had sent them, which raises IRQ 1 (scancodes for B down and up).
        // One at a time: the controller holds one byte, and the interrupt
        // that takes it out may run on another CPU (the boot CPU) a little
        // later than this one writes.
        let deadline = time::now_ns() + time::NANOS_PER_SEC;
        for b in [0x30, 0xb0] {
            let decoded = BYTES.load(SeqCst);
            {
                let _c = CONTROLLER.lock();
                i8042::write_keyboard_output(&I8042Ports, b).map_err(|_| "the controller did not take the command")?;
            }
            while BYTES.load(SeqCst) == decoded && time::now_ns() < deadline {
                crate::thread::yield_now();
            }
        }
        let down = next_event_until(deadline).ok_or("no key event from the controller")?;
        let up = next_event_until(deadline).ok_or("no release event from the controller")?;
        let after = stats();
        crate::kprintln!("keyboard: {} interrupt(s) for 2 bytes", after.interrupts - before.interrupts);
        if (down.event.key, down.event.pressed, up.event.key, up.event.pressed) != (Key::B, true, Key::B, false) {
            return Err("the controller's bytes decoded to the wrong events");
        }
        if after.interrupts == before.interrupts {
            return Err("the bytes arrived without an interrupt");
        }
        Ok(())
    }
}

kernel_test! {
    fn caps_lock_lights_its_led() -> TestResult {
        if !is_present() {
            return Ok(());
        }
        drain_events();
        let before = stats();
        // Caps Lock on, then off again: two LED updates, each acknowledged
        // through the interrupt handler.
        for _ in 0..2 {
            inject(&[0x3a, 0xba]);
            drain_events();
        }
        let after = stats();
        if after.led_failures != before.led_failures {
            return Err("the keyboard did not acknowledge an LED update");
        }
        if after.led_updates != before.led_updates + 2 || modifiers().caps_lock() {
            return Err("Caps Lock did not update the LEDs twice");
        }
        // The event layer knows the LEDs too: Num Lock on, Caps Lock off.
        let leds = device().map(input::leds).ok_or("no keyboard device")?;
        if leds != Leds::NUM_LOCK.to_evdev() {
            return Err("the event layer's LED state is not Num Lock alone");
        }
        Ok(())
    }
}

kernel_test! {
    /// With `poly.test.keyboard=<text>`: wait (30 s at most) for keys
    /// from the host and check they type `<text>` and Enter.
    fn typed_text_matches_the_host_keys(env: &TestEnv) -> TestResult {
        let Some(expected) = env.boot.arg(TEST_KEYBOARD_FLAG) else { return Ok(()) };
        if !is_present() {
            return Err("no keyboard to type on");
        }
        drain_events();
        crate::kprintln!("{TEST_READY} (layout {}, expecting {expected:?} and Enter)", layout().name);
        let deadline = time::now_ns() + 30 * time::NANOS_PER_SEC;
        let mut text = String::new();
        loop {
            let Some(input) = next_event_until(deadline) else {
                crate::kprintln!("keyboard: typed {text:?} without Enter");
                return Err("timed out waiting for Enter");
            };
            match input.output {
                Output::Special(Special::Enter) => break,
                o => text.extend(o.chars()),
            }
        }
        crate::kprintln!("keyboard: typed {text:?}");
        if text != expected {
            return Err("the keys typed something else");
        }
        Ok(())
    }
}
