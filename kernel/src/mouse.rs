//! The PS/2 mouse on the i8042's second port (ADR 0103).
//!
//! `keyboard::init` brings the controller up and asks
//! `poly_input::i8042::init_aux` for the mouse (reset, the IntelliMouse
//! wheel and five-button detection, 100 reports per second, reporting on);
//! [`attach`] then routes ISA IRQ 12 to the same handler as IRQ 1 (both
//! drain the one controller, which says whose byte it holds) and adds the
//! mouse to the event layer. Bytes from the second port come to
//! [`on_byte`], which assembles packets (`poly_input::ps2mouse`) and
//! reports their motion, wheel and buttons as evdev events.

use crate::arch::i8042::I8042Ports;
use crate::sync::SpinLock;
use crate::{arch, input, kprintln, time};
use core::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering::SeqCst};
use poly_input::codes::bus;
use poly_input::hub::{DeviceInfo, InputId};
use poly_input::i8042::{self, AuxError, AuxSetup, CONFIG_AUX_CLOCK_OFF, CONFIG_AUX_IRQ, Setup};
use poly_input::ps2mouse::{Kind, MouseDecoder, MouseState};

/// The ISA IRQ of the controller's second port.
const MOUSE_IRQ: u8 = 12;

/// The mouse's device in the event layer (`usize::MAX`: none).
static DEVICE: AtomicUsize = AtomicUsize::new(usize::MAX);
static DECODER: SpinLock<Option<(MouseDecoder, MouseState)>> = SpinLock::new(None);
static VECTOR: AtomicU8 = AtomicU8::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static PACKETS: AtomicU64 = AtomicU64::new(0);

/// A mouse [`attach`] brought up, for the boot log.
#[derive(Clone, Copy, Debug)]
pub struct Attached {
    aux: AuxSetup,
    vector: u8,
    gsi: u32,
    dev: usize,
}

impl Attached {
    pub fn log(&self) {
        kprintln!(
            "mouse: {} (id {}), ISA IRQ {MOUSE_IRQ} -> GSI {} -> vector {:#x}; event{}",
            self.aux.kind.name(),
            self.aux.id,
            self.gsi,
            self.vector,
            self.dev
        );
    }
}

/// After `i8042::init_aux` (whose result is `aux`): route IRQ 12 and add
/// the mouse to the event layer. Without a mouse, or if IRQ 12 cannot be
/// routed, the second port's interrupt stays off in `setup`.
pub fn attach(aux: Result<AuxSetup, AuxError>, setup: &mut Setup) -> Option<Attached> {
    let aux = match aux {
        Ok(a) => a,
        Err(e) => {
            kprintln!("mouse: i8042 second port: {e:?}; no PS/2 mouse");
            return None;
        }
    };
    let vector = match arch::irq::route_isa_irq(MOUSE_IRQ, crate::keyboard::on_irq, 0) {
        Ok(v) => v,
        Err(e) => {
            kprintln!("mouse: cannot route ISA IRQ {MOUSE_IRQ}: {e:?}; no PS/2 mouse");
            setup.config = (setup.config & !CONFIG_AUX_IRQ) | CONFIG_AUX_CLOCK_OFF;
            return None;
        }
    };
    let gsi = arch::apic::isa_route(MOUSE_IRQ).map_or(u32::from(MOUSE_IRQ), |r| r.gsi);
    *DECODER.lock() = Some((MouseDecoder::new(aux.kind), MouseState::new()));
    let id = InputId { bustype: bus::I8042, vendor: 0, product: 2, version: u16::from(aux.id) };
    let mut info = DeviceInfo::mouse(aux.kind.name(), id, aux.kind.buttons(), aux.kind.has_wheel(), false);
    info.phys = "i8042/serio1".into();
    let dev = input::register(info, None);
    DEVICE.store(dev, SeqCst);
    VECTOR.store(vector, SeqCst);
    Some(Attached { aux, vector, gsi, dev })
}

/// The mouse's device in the event layer.
pub fn device() -> Option<usize> {
    let d = DEVICE.load(SeqCst);
    (d != usize::MAX).then_some(d)
}

/// What kind of mouse answered.
pub fn kind() -> Option<Kind> {
    DECODER.lock().as_ref().map(|(d, _)| d.kind())
}

/// A byte from the second port (from the interrupt handler, under the
/// keyboard's producer lock), which arrived at `now_ns`.
pub fn on_byte(byte: u8, now_ns: u64) {
    if let Some(dev) = device() {
        let mut d = DECODER.lock();
        if let Some((decoder, state)) = d.as_mut()
            && let Some(p) = decoder.feed(byte, now_ns)
        {
            let (events, n) = state.events(&p);
            input::report_at(dev, now_ns, &events[..n]);
            PACKETS.fetch_add(1, SeqCst);
        }
    }
    // Counted once the byte's events are in the event layer.
    BYTES.fetch_add(1, SeqCst);
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use poly_input::codes::{ev, key, rel, syn};
use poly_input::ps2mouse::{Packet, button, encode};

kernel_test! {
    fn mouse_found_and_irq_routed() -> TestResult {
        let Some(dev) = device() else {
            // QEMU's PC machines always have the PS/2 mouse.
            return if crate::keyboard::is_present() { Err("a keyboard but no PS/2 mouse") } else { Ok(()) };
        };
        let route = arch::apic::isa_route(MOUSE_IRQ).ok_or("ISA IRQ 12 has no route")?;
        let entry = arch::apic::gsi_entry(route.gsi).ok_or("no redirection entry for IRQ 12's GSI")?;
        if entry.vector() != VECTOR.load(SeqCst) || entry.is_masked() {
            return Err("IRQ 12's redirection entry is masked or points elsewhere");
        }
        let info = input::info(dev).ok_or("the mouse is not a device of the event layer")?;
        if !info.caps.has(ev::REL, rel::X) || !info.caps.has(ev::KEY, key::BTN_LEFT) {
            return Err("the mouse device lacks motion or buttons");
        }
        if info.caps.has(ev::REL, rel::WHEEL) != kind().is_some_and(|k| k.has_wheel()) {
            return Err("the wheel was not declared as detected");
        }
        Ok(())
    }
}

kernel_test! {
    fn controller_mouse_bytes_become_events() -> TestResult {
        let (Some(dev), Some(kind)) = (device(), kind()) else { return Ok(()) };
        let files = crate::fs::Files::new();
        let fd = input::open_event_file(&files, dev)?;
        // The controller puts each byte in its output buffer as if the
        // mouse had sent it (0xD3), raising IRQ 12: a packet with motion
        // right and up, the wheel up and the left button down, then one
        // with nothing pressed.
        let wheel = i32::from(kind.has_wheel());
        let packets = [Packet { dx: 7, dy: 3, wheel, buttons: button::LEFT }, Packet::default()];
        let deadline = time::now_ns() + 2 * time::NANOS_PER_SEC;
        for p in &packets {
            let (bytes, n) = encode(kind, p);
            for &b in &bytes[..n] {
                let seen = BYTES.load(SeqCst);
                {
                    let _c = crate::keyboard::CONTROLLER.lock();
                    i8042::write_aux_output(&I8042Ports, b).map_err(|_| "the controller did not take the byte")?;
                }
                while BYTES.load(SeqCst) == seen && time::now_ns() < deadline {
                    crate::thread::yield_now();
                }
            }
        }
        // The input thread wakes readers; the queue is filled already.
        let events = input::read_events(&files, fd);
        let _ = files.close(fd);
        let mut want = alloc::vec![(ev::REL, rel::X, 7), (ev::REL, rel::Y, -3)];
        if wheel != 0 {
            want.push((ev::REL, rel::WHEEL, 1));
        }
        want.extend([(ev::KEY, key::BTN_LEFT, 1), (ev::SYN, syn::REPORT, 0), (ev::KEY, key::BTN_LEFT, 0), (ev::SYN, syn::REPORT, 0)]);
        if events != want {
            kprintln!("mouse test: read {events:?}, {} byte(s), {} packet(s)", BYTES.load(SeqCst), PACKETS.load(SeqCst));
            return Err("the controller's mouse bytes did not read as motion and a click");
        }
        Ok(())
    }
}
