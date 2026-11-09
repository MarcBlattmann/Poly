//! The shell on COM1: output through the kernel's serial console, input
//! from the UART's receive interrupt.
//!
//! ISA IRQ 4 is routed through the I/O APIC to a vector of its own
//! (`arch::irq::route_isa_irq`, ADR 0016). The handler drains the UART into
//! a ring buffer (`poly_tty::rx::RxRing`; single producer: the handler,
//! which never nests, or a reader holding [`RECEIVER`]; single consumer:
//! the one [`SerialConsole`]) and wakes the reader. A full ring is not
//! overrun: the handler leaves the rest in the UART and switches its
//! receive interrupt off, so QEMU holds the input back, and the reader
//! drains the UART again and switches the interrupt on once it has taken
//! the ring down to half (flow control, ADR 0123). The reader
//! decodes the terminal's byte stream into edits with
//! `poly_line::TermDecoder`. If the IRQ cannot be routed, the reader polls
//! the UART every 10 ms instead.
//!
//! Kernel log lines that arrive while the prompt is on the screen erase it
//! first (see `log`); the reader notices within [`REDRAW_CHECK_NS`] and
//! hands the editor an [`Edit::Redraw`], which draws the prompt and the line
//! again below the log output.
//!
//! The terminal on COM1 (`/dev/ttyS0`, `crate::tty`) reads the same ring
//! while a program has it open ([`take_input`], ADR 0046); the shell then
//! leaves the input alone. Either reader takes bytes under [`CONSUMER`].

use super::Console;
use crate::arch::irq;
use crate::arch::serial::{COM1, COM1_IRQ, SerialReceiver};
use crate::sync::SpinLock;
use crate::thread::WaitQueue;
use crate::{kprintln, log, thread, time};
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use core::time::Duration;
use poly_line::{Edit, TermDecoder};
use poly_tty::rx::RxRing;

// SAFETY: COM1's receive side is only driven through this lock.
static RECEIVER: SpinLock<SerialReceiver> = SpinLock::new(unsafe { SerialReceiver::new(COM1) });

/// Received bytes not read yet.
const RX_SIZE: usize = 256;
static RX: RxRing<RX_SIZE> = RxRing::new();
/// How often the UART was held back because the ring was full.
static RX_THROTTLED: AtomicU64 = AtomicU64::new(0);
/// The reader waits here for bytes.
static RX_WAIT: WaitQueue = WaitQueue::new();
/// Taken by whoever takes bytes out of the ring: the shell's console or
/// the terminal's input thread.
static CONSUMER: SpinLock<()> = SpinLock::new(());

/// The receive interrupt's vector, 0 while polled.
static VECTOR: AtomicU8 = AtomicU8::new(0);
/// The one [`SerialConsole`] has been handed out.
static TAKEN: AtomicBool = AtomicBool::new(false);
/// [`start_receiving`] ran.
static RECEIVING: AtomicBool = AtomicBool::new(false);

/// How often a waiting reader checks whether log output erased the prompt.
const REDRAW_CHECK_NS: u64 = 50 * time::NANOS_PER_MILLI;
/// Bytes the handler moves per interrupt at most, so a flood cannot keep it
/// running.
const DRAIN_LIMIT: usize = 64;

/// The vector COM1's receive interrupt arrives on, once the console is set up.
pub fn vector() -> Option<u8> {
    match VECTOR.load(Ordering::Relaxed) {
        0 => None,
        v => Some(v),
    }
}

/// Consumer side: the [`SerialConsole`] or the terminal, holding
/// [`CONSUMER`]. Taking bytes may let the UART go again.
fn pop() -> Option<u8> {
    let byte = {
        let _consumer = CONSUMER.lock();
        RX.pop()
    };
    resume_if_due();
    byte
}

fn available() -> bool {
    !RX.is_empty()
}

/// Move what the UART holds into the ring while there is room; a full ring
/// holds the UART back (its receive interrupt off). The caller holds
/// [`RECEIVER`], so it is the only producer.
fn drain(rx: &mut SerialReceiver) -> bool {
    let f = RX.fill(|| rx.read(), DRAIN_LIMIT);
    if f.full {
        rx.disable_interrupt();
        RX_THROTTLED.fetch_add(1, Ordering::Relaxed);
    }
    f.moved > 0
}

/// The ring was full and is down to half: drain the UART again and, if the
/// ring took all of it, switch the receive interrupt back on. The readers
/// call this after taking bytes and before waiting.
fn resume_if_due() {
    if vector().is_none() || !RX.take_resume() {
        return;
    }
    let mut rx = RECEIVER.lock();
    let f = RX.fill(|| rx.read(), RX_SIZE);
    if !f.full {
        rx.enable_interrupt();
    }
    // Full again: still throttled, the interrupt stays off.
    drop(rx);
    if f.moved > 0 {
        RX_WAIT.wake_all();
    }
}

/// COM1's receive interrupt.
fn on_receive(_: usize) {
    let any = drain(&mut RECEIVER.lock());
    if any {
        RX_WAIT.wake_all();
    }
}

/// Route COM1's receive interrupt and start filling the ring (once; later
/// calls do nothing). Without a route, readers poll the UART.
pub fn start_receiving() {
    if RECEIVING.swap(true, Ordering::SeqCst) {
        return;
    }
    match irq::route_isa_irq(COM1_IRQ, on_receive, 0) {
        Ok(v) => {
            VECTOR.store(v, Ordering::Relaxed);
            let mut rx = RECEIVER.lock();
            rx.enable_interrupt();
            // Whatever arrived before the interrupt was on; with
            // interrupts off (the lock), the handler cannot run meanwhile.
            drain(&mut rx);
            kprintln!("shell: serial input on COM1, ISA IRQ {COM1_IRQ} -> vector {v:#x}");
        }
        Err(e) => kprintln!("shell: COM1 receive interrupt not routed ({e:?}); polling"),
    }
}

/// Take up to `buf.len()` received bytes (the terminal on COM1).
pub fn take_input(buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        let b = if vector().is_none() { RECEIVER.lock().read() } else { pop() };
        let Some(b) = b else { break };
        buf[n] = b;
        n += 1;
    }
    n
}

/// Wait until bytes were received or the monotonic clock reaches
/// `deadline_ns`.
pub fn wait_for_input(deadline_ns: u64) {
    if vector().is_none() {
        thread::sleep(Duration::from_millis(10));
    } else {
        resume_if_due();
        RX_WAIT.wait_until_deadline(available, deadline_ns);
    }
}

/// The shell's console on COM1. There is one; [`take`](Self::take) hands it
/// out once.
#[derive(Debug)]
pub struct SerialConsole {
    decoder: TermDecoder,
}

impl SerialConsole {
    /// Set up the receive interrupt and return the console, the first time;
    /// `None` after that.
    pub fn take() -> Option<SerialConsole> {
        if TAKEN.swap(true, Ordering::Relaxed) {
            return None;
        }
        start_receiving();
        Some(SerialConsole { decoder: TermDecoder::new() })
    }

    /// The next byte, waiting for one. `None`: the wait ended because log
    /// output erased the prompt. While a program has the terminal on COM1
    /// open, its bytes are the terminal's.
    fn next_byte(&mut self) -> Option<u8> {
        loop {
            if log::prompt_disturbed() {
                return None;
            }
            if crate::tty::serial_in_use() {
                thread::sleep(Duration::from_millis(REDRAW_CHECK_NS / time::NANOS_PER_MILLI));
                continue;
            }
            if vector().is_none() {
                if let Some(b) = RECEIVER.lock().read() {
                    return Some(b);
                }
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            if let Some(b) = pop() {
                return Some(b);
            }
            resume_if_due();
            let deadline = time::now_ns() + REDRAW_CHECK_NS;
            RX_WAIT.wait_until_deadline(|| available() || log::prompt_disturbed(), deadline);
        }
    }
}

impl fmt::Write for SerialConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        log::write_console(s);
        Ok(())
    }
}

impl Console for SerialConsole {
    fn terminal(&self) -> Option<&'static str> {
        Some("/dev/ttyS0")
    }

    fn read(&mut self) -> Option<Edit> {
        // The editor has drawn the prompt (and maybe part of a line) by now.
        log::prompt_shown(true);
        loop {
            if log::take_prompt_disturbed() {
                return Some(Edit::Redraw);
            }
            let Some(b) = self.next_byte() else { continue };
            if let Some(e) = self.decoder.feed(b) {
                if matches!(e, Edit::Enter | Edit::Cancel) {
                    // The line is finished; log output may go anywhere again.
                    log::prompt_shown(false);
                }
                return Some(e);
            }
        }
    }
}

// --- Tests ------------------------------------------------------------------

crate::kernel_test! {
    fn ring_buffer_keeps_order_and_holds_back_when_full() -> TestResult {
        // Only before the console exists, and while no program has the
        // terminal open: they are the ring's consumers (and the UART's
        // interrupt is not on yet).
        if TAKEN.load(Ordering::Relaxed) || crate::tty::serial_in_use() || vector().is_some() {
            return Ok(());
        }
        let mut next = 0usize;
        let f = {
            // As the handler would: interrupts off while producing.
            let _rx = RECEIVER.lock();
            RX.fill(
                || {
                    next += 1;
                    Some((next - 1) as u8)
                },
                RX_SIZE + 3,
            )
        };
        // Nothing taken past the room: the source keeps the rest.
        if !f.full || f.moved != RX_SIZE || next != RX_SIZE || !RX.is_throttled() {
            return Err("a full ring did not stop taking bytes and hold the source back");
        }
        let all_kept = (0..RX_SIZE).all(|i| pop() == Some(i as u8));
        if !all_kept || pop().is_some() || available() {
            return Err("the ring lost or reordered bytes");
        }
        // Without the interrupt the readers poll the UART: the ring only
        // forgets that it was throttled.
        if !RX.take_resume() || RX.is_throttled() {
            return Err("an emptied ring was not resumed");
        }
        Ok(())
    }
}

crate::kernel_test! {
    fn log_output_erases_a_shown_prompt_once() -> TestResult {
        log::prompt_shown(true);
        let quiet = !log::take_prompt_disturbed();
        kprintln!("shell: (a log line while the prompt is shown)");
        kprintln!("shell: (and another)");
        let first = log::take_prompt_disturbed();
        let second = log::take_prompt_disturbed();
        log::prompt_shown(false);
        kprintln!("shell: (a log line without a prompt)");
        let without = log::take_prompt_disturbed();
        if !quiet || !first || second || without {
            return Err("the prompt was not marked disturbed exactly when log output erased it");
        }
        Ok(())
    }
}
