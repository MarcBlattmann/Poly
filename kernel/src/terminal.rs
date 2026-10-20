//! The screen terminal: a status bar and a text grid on the display, a
//! kernel shell on the keyboard, the kernel log mirrored into the grid
//! ([ADR 0035](../../docs/adr/0035-screen-terminal.md)).
//!
//! ```text
//!  kprintln ─▶ log::_print ─▶ serial (COM1)
//!                    └──────▶ mirror(): LOG ring (64 KiB, spinlock, never renders)
//!                                              │ drained by whoever holds the terminal
//!  keyboard ─▶ ScreenConsole::read ─▶ poly_term::keys (bytes) ─▶ TermDecoder ─▶ Edit ─▶ shell
//!                     shell output ─▶ ScreenConsole::write_str ─▶ Terminal (grid) ─┐
//!                                                                                  ▼ poke
//!  render thread "terminal": drain LOG, status bar, TextGrid::render ─▶ back buffer ─▶ flush in bands
//! ```
//!
//! The logic (layout, status line, key encoding, the log ring, prompt
//! handling, rendering) is the host-tested `poly-term` crate; this module
//! gives it memory, locks, threads and the devices.
//!
//! - **One lock for the grid:** `SCREEN.term`, a sleeping mutex, held by the
//!   shell while it writes and by the render thread while it draws (which
//!   also takes the display's spinlock inside). Never taken by the log.
//! - **The log never touches the grid.** [`mirror`] (called by
//!   `log::_print`, with the console lock held, in any context, interrupt
//!   handlers included) only appends to [`LOG`], a ring behind its own
//!   spinlock, which readers hold just to copy bytes out. Whoever holds the
//!   terminal drains the ring into the grid: the shell before its own
//!   output (so the two stay in order), the render thread otherwise, at
//!   least every [`LOG_POLL_NS`]. The ring keeps the boot log too, so the
//!   terminal starts with the last 64 KiB of it.
//! - **The prompt survives log output** as on the serial console: the
//!   console marks it shown while it waits for keys; log output erases its
//!   row and the console hands the shell an `Edit::Redraw` within
//!   [`REDRAW_CHECK_NS`] (`poly_term::Terminal` documents the rules).
//! - **Rendering is batched.** Writers only change cells and poke the render
//!   thread, which draws at most once per [`MIN_FRAME_NS`] (a full redraw
//!   takes about 40 ms under TCG, a full flush 20 ms) and flushes in bands
//!   of [`FLUSH_BAND`] rows, so interrupts are not held off for a whole
//!   flush. The status bar is refreshed every second.
//! - **Keys** are encoded as a terminal would send them
//!   (`poly_term::keys`) and decoded by the same `TermDecoder` as the
//!   serial line; Shift+Page Up / Page Down scroll the view instead, and
//!   any key that types snaps it back. The keyboard is read with
//!   `keyboard::next_shared_event_until`, which leaves keys alone while
//!   kernel tests run (they read the keyboard themselves), and the render
//!   thread pauses during the display tests, which draw on the screen.
//!
//! - **The terminal device** (`/dev/tty1`, `crate::tty`, ADR 0046): what
//!   programs write to it goes into [`TTY_OUT`] ([`tty_output`], which
//!   never draws or sleeps: terminals call it with their lock held) and
//!   into the grid with the log; while a program has tty1 open, the
//!   keyboard is the terminal's and the screen's shell waits.
//!
//! Without a display (`poly.display=off`, no VGA) or with
//! `poly.terminal=off`, nothing starts and the mirror turns itself off.

use crate::boot_info::BootInfo;
use crate::frames::FRAMES;
use crate::sync::{Mutex, SpinLock};
use crate::thread::{self, WaitQueue};
use crate::{acpi, display, keyboard, kprintln, ktest, shell, time};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use poly_gfx::{Cell, Rect};
use poly_line::{Edit, TermDecoder};
use poly_mem::PAGE_SIZE;
use poly_term::keys::{self, KeyAction};
use poly_term::status::StatusInfo;
use poly_term::{LogRing, Screen, Terminal};

/// Kernel argument: `poly.terminal=off` leaves the display alone (the boot
/// screen stays) and starts no shell on it.
pub const TERMINAL_ARG: &str = "poly.terminal";

/// Bytes of kernel log the ring keeps.
const LOG_RING_BYTES: usize = 64 * 1024;
/// How often the render thread looks for new log output at most.
const LOG_POLL_NS: u64 = 25 * time::NANOS_PER_MILLI;
/// How often a waiting console checks whether log output erased its prompt.
const REDRAW_CHECK_NS: u64 = 50 * time::NANOS_PER_MILLI;
/// The shortest time between two renders.
const MIN_FRAME_NS: u64 = 15 * time::NANOS_PER_MILLI;
/// Rows of pixels flushed per display lock.
const FLUSH_BAND: u32 = 64;
/// How often the render thread looks whether a display test is over.
const PAUSE_POLL_NS: u64 = 50 * time::NANOS_PER_MILLI;

/// The kernel log's last bytes, for the screen.
static LOG: SpinLock<LogRing<LOG_RING_BYTES>> = SpinLock::new(LogRing::new());
/// `LOG.written()`, readable without the lock.
static LOG_WRITTEN: AtomicU64 = AtomicU64::new(0);
/// No terminal will ever read the ring.
static MIRROR_OFF: AtomicBool = AtomicBool::new(false);

/// Append log output to the ring the screen terminal shows. Called by
/// `log::_print` with the console lock held, from any context: it only
/// copies bytes, under a spinlock that nothing holds for longer.
pub fn mirror(args: fmt::Arguments) {
    if MIRROR_OFF.load(SeqCst) {
        return;
    }
    let mut ring = LOG.lock();
    let _ = ring.write_fmt(args);
    LOG_WRITTEN.store(ring.written(), SeqCst);
}

/// Program output for the grid from `/dev/tty1`, not drawn yet.
static TTY_OUT: SpinLock<TtyOut> = SpinLock::new(TtyOut { bytes: VecDeque::new(), lost: 0 });
/// Most bytes of program output waiting for the grid; more push the
/// oldest out (the terminal reports how many).
const TTY_OUT_BYTES: usize = 64 * 1024;

struct TtyOut {
    bytes: VecDeque<u8>,
    lost: u64,
}

/// Bytes a program wrote to the screen's terminal (`/dev/tty1`), already
/// through its line discipline: queued for the grid and the render thread
/// poked. Never draws or sleeps.
pub fn tty_output(bytes: &[u8]) {
    if MIRROR_OFF.load(SeqCst) {
        return;
    }
    {
        let mut out = TTY_OUT.lock();
        out.bytes.extend(bytes);
        let over = out.bytes.len().saturating_sub(TTY_OUT_BYTES);
        if over > 0 {
            out.bytes.drain(..over);
            out.lost += over as u64;
        }
    }
    SCREEN.poke();
}

/// Scroll the screen's view `pages` pages back (negative: forward), as
/// Shift+Page Up / Down do.
pub fn scroll_pages(pages: i32) {
    SCREEN.with(|t| t.scroll_pages(pages));
    SCREEN.poke();
}

/// Show the screen again if the view is scrolled back (a key that types).
pub fn scroll_to_bottom() {
    if SCREEN.with(|t| t.grid().view_offset() > 0).unwrap_or(false) {
        SCREEN.with(Terminal::scroll_to_bottom);
        SCREEN.poke();
    }
}

/// The terminal and what goes with it.
struct Term<'a> {
    t: Terminal<'a>,
    /// Position in [`LOG`] up to which the log is in the grid.
    log_pos: u64,
    /// A UTF-8 character of program output split between two writes.
    carry: crate::fs::devices::Utf8Carry,
}

/// A terminal shared by a console, a render thread and the log.
struct Shared<'a> {
    term: Mutex<Option<Term<'a>>>,
    /// Whether the kernel log is drained into it (the screen's terminal;
    /// not the kernel tests' private ones).
    follows_log: bool,
    /// `log_pos` of the terminal, readable without its lock.
    log_seen: AtomicU64,
    /// Cells changed since the last render.
    dirty: AtomicBool,
    /// The render thread waits here.
    wake: WaitQueue,
}

/// The terminal on the display.
static SCREEN: Shared<'static> = Shared::new(true);

impl<'a> Shared<'a> {
    const fn new(follows_log: bool) -> Shared<'a> {
        Shared {
            term: Mutex::new(None),
            follows_log,
            log_seen: AtomicU64::new(0),
            dirty: AtomicBool::new(false),
            wake: WaitQueue::new(),
        }
    }

    /// Run `f` on the terminal, with the log drained into it first. `None`
    /// if there is no terminal.
    fn with<R>(&self, f: impl FnOnce(&mut Terminal<'a>) -> R) -> Option<R> {
        let mut guard = self.term.lock();
        let term = guard.as_mut()?;
        self.drain_log(term);
        self.drain_tty(term);
        Some(f(&mut term.t))
    }

    /// Move log output the terminal has not shown yet into its grid.
    fn drain_log(&self, term: &mut Term<'a>) {
        if !self.follows_log {
            return;
        }
        let mut buf = [0u8; 512];
        loop {
            let got = LOG.lock().read_from(term.log_pos, &mut buf);
            if got.len == 0 && got.lost == 0 {
                break;
            }
            term.t.log_lost(got.lost);
            term.t.write_log(&buf[..got.len]);
            term.log_pos = got.next;
        }
        self.log_seen.store(term.log_pos, SeqCst);
    }

    /// Move program output from `/dev/tty1` into the grid.
    fn drain_tty(&self, term: &mut Term<'a>) {
        if !self.follows_log {
            return;
        }
        let mut buf = [0u8; 512];
        loop {
            let (n, lost) = {
                let mut out = TTY_OUT.lock();
                let n = out.bytes.len().min(buf.len());
                for (dst, src) in buf.iter_mut().zip(out.bytes.drain(..n)) {
                    *dst = src;
                }
                (n, core::mem::take(&mut out.lost))
            };
            if lost > 0 {
                term.t.log_lost(lost);
            }
            if n == 0 {
                break;
            }
            let t = &mut term.t;
            term.carry.feed(&buf[..n], &mut |s| t.write_shell(s));
        }
    }

    /// Log output the terminal has not shown yet.
    fn log_pending(&self) -> bool {
        self.follows_log && (LOG_WRITTEN.load(SeqCst) != self.log_seen.load(SeqCst) || !TTY_OUT.lock().bytes.is_empty())
    }

    /// Something changed: get the render thread going.
    fn poke(&self) {
        self.dirty.store(true, SeqCst);
        self.wake.wake_all();
    }
}

// --- Start ------------------------------------------------------------------

/// The screen terminal is up ([`bring_up`] made it).
static UP: AtomicBool = AtomicBool::new(false);
/// [`bring_up`] ran (whatever it found).
static TRIED: AtomicBool = AtomicBool::new(false);

/// Put the terminal on the display and start a shell on it with the
/// keyboard. Call once, after the kernel tests and before the serial shell
/// starts (it logs, which would disturb that shell's first prompt).
pub fn start(boot: &BootInfo) {
    if !bring_up(boot) {
        return;
    }
    let console = ScreenConsole::new(&SCREEN, KeyboardKeys);
    if let Err(e) = shell::spawn("shell-screen", Box::new(console)) {
        kprintln!("terminal: the shell could not start: {e:?}");
    }
    if let Some(screen) = SCREEN.with(|t| *t.screen()) {
        kprintln!(
            "terminal: {}x{} cells in {} on the display, status bar on top, {} lines of scrollback; a shell on the keyboard{}",
            screen.cols,
            screen.rows,
            screen.font.name(),
            screen.scrollback,
            if keyboard::is_present() { "" } else { " (no keyboard found)" },
        );
    }
}

/// Put the terminal on the display, with its render thread, without a
/// shell: early, when the first program's console is the screen's
/// terminal (`console=tty1`), and from [`start`]. Returns whether the
/// terminal is up; later calls do nothing again.
pub fn bring_up(boot: &BootInfo) -> bool {
    if TRIED.swap(true, SeqCst) {
        return UP.load(SeqCst);
    }
    let Some(info) = display::info() else {
        MIRROR_OFF.store(true, SeqCst);
        return false;
    };
    if boot.arg(TERMINAL_ARG) == Some("off") {
        MIRROR_OFF.store(true, SeqCst);
        kprintln!("terminal: off ({TERMINAL_ARG}=off); the display keeps the boot screen");
        return false;
    }
    let Some(screen) = Screen::new(info.width, info.height) else {
        MIRROR_OFF.store(true, SeqCst);
        kprintln!("terminal: a {}x{} display is too small for a terminal", info.width, info.height);
        return false;
    };
    let mut cells = Vec::new();
    if cells.try_reserve_exact(screen.cells()).is_err() {
        MIRROR_OFF.store(true, SeqCst);
        kprintln!("terminal: no memory for {} cells", screen.cells());
        return false;
    }
    cells.resize(screen.cells(), Cell::BLANK);
    // The terminal lives as long as the kernel.
    let cells: &'static mut [Cell] = Box::leak(cells.into_boxed_slice());
    let t = Terminal::new(screen, cells).expect("the screen layout fits a text grid");
    // Start with what the ring still has of the boot log.
    let log_pos = LOG.lock().oldest();
    *SCREEN.term.lock() = Some(Term { t, log_pos, carry: crate::fs::devices::Utf8Carry::default() });
    if let Err(e) = thread::spawn_named("terminal", render_main, 0) {
        kprintln!("terminal: the render thread could not start: {e:?}");
        return false;
    }
    UP.store(true, SeqCst);
    crate::tty::screen_size(screen.cols, screen.rows);
    true
}

// --- Rendering ----------------------------------------------------------------

/// The display tests draw on the screen and check it: no rendering then.
fn render_paused() -> bool {
    ktest::current().is_some_and(|name| name.starts_with("display::"))
}

fn render_main(_: usize) -> usize {
    let s = &SCREEN;
    let (mut last_frame, mut next_status) = (0u64, 0u64);
    loop {
        let now = time::now_ns();
        let deadline = next_status.min(now + LOG_POLL_NS);
        s.wake.wait_until_deadline(|| s.dirty.load(SeqCst) || s.log_pending(), deadline);
        if render_paused() {
            thread::sleep_until(time::now_ns() + PAUSE_POLL_NS);
            continue;
        }
        let now = time::now_ns();
        let status_due = now >= next_status;
        if !s.dirty.load(SeqCst) && !s.log_pending() && !status_due {
            continue;
        }
        // Let more output gather if the last frame was just now.
        if now < last_frame + MIN_FRAME_NS {
            thread::sleep_until(last_frame + MIN_FRAME_NS);
        }
        s.dirty.store(false, SeqCst);
        if let Some(r) = render(s, status_info()) {
            flush(r);
        }
        last_frame = time::now_ns();
        if status_due {
            // On the second, so the uptime ticks when it should.
            next_status = (now / time::NANOS_PER_SEC + 1) * time::NANOS_PER_SEC;
        }
    }
}

/// Draw what changed into the back buffer; the rectangle to flush.
fn render(s: &Shared<'_>, info: StatusInfo) -> Option<Rect> {
    s.with(|t| {
        t.set_status(info);
        if !t.needs_render() {
            return None;
        }
        display::with_canvas(|c| t.render(c)).flatten()
    })
    .flatten()
}

/// Copy `r` to the screen a band of rows at a time, so the display lock
/// (interrupts off) is never held for a whole-screen copy.
fn flush(r: Rect) {
    let bottom = r.bottom();
    let mut y = r.y as i64;
    while y < bottom {
        let h = (bottom - y).min(i64::from(FLUSH_BAND)) as u32;
        display::flush(Rect::new(r.x, y as i32, r.width, h));
        y += i64::from(h);
    }
}

/// What the status bar shows now. Takes the frame allocator's and the
/// keyboard's locks, so call it without the terminal's.
fn status_info() -> StatusInfo {
    let (free, total) =
        FRAMES.lock().as_ref().map_or((0, 0), |f| (f.free_frames() * PAGE_SIZE, f.total_frames() * PAGE_SIZE));
    StatusInfo {
        version: env!("CARGO_PKG_VERSION"),
        uptime_ns: time::now_ns(),
        free_bytes: free,
        total_bytes: total,
        layout: keyboard::layout().name,
        // One CPU runs the kernel until SMP (Phase 4); ACPI says how many
        // the machine has.
        cpus_online: 1,
        cpus: acpi::summary().map_or(1, |s| s.cpus.max(1)),
        scrolled: None,
    }
}

// --- The console ----------------------------------------------------------------

/// What a key source returned.
enum KeyWait {
    Key(keyboard::Input),
    /// Nothing before the deadline.
    Timeout,
    /// No more keys, ever: ends the shell (scripted sources in tests).
    End,
}

/// Where the screen console's keys come from.
trait Keys {
    /// The next key event, waiting until `deadline_ns` at most.
    fn next(&mut self, deadline_ns: u64) -> KeyWait;
}

/// The PS/2 keyboard.
struct KeyboardKeys;

impl Keys for KeyboardKeys {
    fn next(&mut self, deadline_ns: u64) -> KeyWait {
        // While a program has /dev/tty1 open, the keys are its.
        if crate::tty::screen_in_use() {
            thread::sleep_until(deadline_ns);
            return KeyWait::Timeout;
        }
        keyboard::next_shared_event_until(deadline_ns).map_or(KeyWait::Timeout, KeyWait::Key)
    }
}

/// A shell's [`Console`](shell::Console) on a terminal: keys in, grid out.
struct ScreenConsole<'s, 'a, K> {
    shared: &'s Shared<'a>,
    keys: K,
    decoder: TermDecoder,
    /// Edits decoded but not handed out yet.
    pending: VecDeque<Edit>,
    cols: usize,
}

impl<'s, 'a, K: Keys> ScreenConsole<'s, 'a, K> {
    fn new(shared: &'s Shared<'a>, keys: K) -> ScreenConsole<'s, 'a, K> {
        let cols = shared.with(|t| t.screen().cols).unwrap_or(poly_line::DEFAULT_WIDTH);
        ScreenConsole { shared, keys, decoder: TermDecoder::new(), pending: VecDeque::new(), cols }
    }

    /// Act on one key event: bytes for the line editor, or scrolling.
    fn key(&mut self, input: &keyboard::Input) {
        match keys::translate(input) {
            KeyAction::None => {}
            KeyAction::ScrollPages(pages) => {
                self.shared.with(|t| t.scroll_pages(pages));
                self.shared.poke();
            }
            KeyAction::Send(bytes) => {
                // Typing shows the screen again.
                if self.shared.with(|t| t.grid().view_offset() > 0).unwrap_or(false) {
                    self.shared.with(Terminal::scroll_to_bottom);
                    self.shared.poke();
                }
                for &b in bytes.as_bytes() {
                    if let Some(e) = self.decoder.feed(b) {
                        self.pending.push_back(e);
                    }
                }
            }
        }
    }
}

impl<K> fmt::Write for ScreenConsole<'_, '_, K> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.shared.with(|t| t.write_shell(s));
        self.shared.poke();
        Ok(())
    }
}

impl<K: Keys> shell::Console for ScreenConsole<'_, '_, K> {
    fn read(&mut self) -> Option<Edit> {
        // The editor has drawn the prompt (and maybe part of a line) by now.
        self.shared.with(|t| t.prompt_shown(true));
        loop {
            if let Some(e) = self.pending.pop_front() {
                if matches!(e, Edit::Enter | Edit::Cancel) {
                    // The line is finished; log output may go anywhere again.
                    self.shared.with(|t| t.prompt_shown(false));
                }
                return Some(e);
            }
            if self.shared.with(Terminal::take_redraw).unwrap_or(false) {
                return Some(Edit::Redraw);
            }
            match self.keys.next(time::now_ns() + REDRAW_CHECK_NS) {
                KeyWait::Key(input) => self.key(&input),
                KeyWait::Timeout => {}
                KeyWait::End => return None,
            }
        }
    }

    fn width(&self) -> usize {
        self.cols
    }

    fn terminal(&self) -> Option<&'static str> {
        Some("/dev/tty1")
    }
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use alloc::string::String;
use alloc::vec;
use poly_input::layout;
use poly_input::set1::encode;
use poly_input::{Key, Keyboard, Modifiers, Output};

/// One step of a scripted key source.
enum Step {
    Key(keyboard::Input),
    /// Kernel log output arrives (written into the terminal directly).
    Log(&'static str),
}

/// Keys from a script, for a terminal of the tests' own.
struct Script<'s, 'a> {
    shared: &'s Shared<'a>,
    steps: VecDeque<Step>,
}

impl Keys for Script<'_, '_> {
    fn next(&mut self, _deadline_ns: u64) -> KeyWait {
        match self.steps.pop_front() {
            None => KeyWait::End,
            Some(Step::Key(input)) => KeyWait::Key(input),
            Some(Step::Log(text)) => {
                self.shared.with(|t| t.write_log(text.as_bytes()));
                KeyWait::Timeout
            }
        }
    }
}

/// The key events of pressing and releasing `key` with `held` down, on a
/// US keyboard.
fn tap(kb: &mut Keyboard, held: &[Key], key: Key) -> Vec<Step> {
    let mut bytes = Vec::new();
    held.iter().for_each(|&h| bytes.extend_from_slice(&encode(h, true)));
    bytes.extend_from_slice(&encode(key, true));
    bytes.extend_from_slice(&encode(key, false));
    held.iter().rev().for_each(|&h| bytes.extend_from_slice(&encode(h, false)));
    bytes.into_iter().filter_map(|b| kb.feed(b)).map(Step::Key).collect()
}

/// The key events that type `text` (characters on the US layout, without
/// or with Shift).
fn typing(kb: &mut Keyboard, text: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    for c in text.chars() {
        let found = Key::ALL.iter().find_map(|&k| {
            [false, true].into_iter().find_map(|shift| {
                let mut probe = Keyboard::new(&layout::US);
                let mut bytes = Vec::new();
                if shift {
                    bytes.extend_from_slice(&encode(Key::ShiftLeft, true));
                }
                bytes.extend_from_slice(&encode(k, true));
                let out = bytes.into_iter().filter_map(|b| probe.feed(b)).last()?.output;
                (out == Output::Char(c)).then_some((k, shift))
            })
        });
        let (key, shift) = found.expect("a US key types it");
        steps.extend(tap(kb, if shift { &[Key::ShiftLeft] } else { &[] }, key));
    }
    steps
}

/// A small terminal (80 x 6 cells and 100 lines of scrollback) in `cells`.
fn small_terminal(cells: &mut [Cell]) -> Terminal<'_> {
    let screen = Screen { scrollback: 100, ..Screen::new(640, 112).expect("a layout") };
    Terminal::new(screen, cells).expect("cells for the small screen")
}

fn row_text(t: &Terminal<'_>, row: usize) -> String {
    let line = t.grid().screen_line(row).unwrap_or(&[]);
    let s: String = line.iter().map(|c| c.ch).collect();
    String::from(s.trim_end())
}

fn rows(t: &Terminal<'_>) -> Vec<String> {
    (0..t.screen().rows).map(|r| row_text(t, r)).collect()
}

kernel_test! {
    /// The real shell on a screen console: typed keys (through the key
    /// encoding and the line editor's decoder) run commands, Tab completes,
    /// Backspace and the arrows edit, and the output lands in the grid.
    fn shell_runs_on_a_screen_console() -> TestResult {
        let mut cells = vec![Cell::BLANK; 80 * 106];
        let shared = Shared::new(false);
        *shared.term.lock() = Some(Term { t: small_terminal(&mut cells), log_pos: 0, carry: Default::default() });
        let mut kb = Keyboard::new(&layout::US);
        kb.set_locks(Modifiers::NUM_LOCK);
        let mut steps: Vec<Step> = typing(&mut kb, "ech");
        steps.extend(tap(&mut kb, &[], Key::Tab));
        steps.extend(typing(&mut kb, "Screan"));
        steps.extend(tap(&mut kb, &[], Key::ArrowLeft));
        steps.extend(tap(&mut kb, &[], Key::Backspace));
        steps.extend(typing(&mut kb, "e"));
        steps.extend(tap(&mut kb, &[], Key::End));
        steps.extend(typing(&mut kb, "!"));
        steps.extend(tap(&mut kb, &[], Key::Enter));
        // Ctrl-C drops a line; Up recalls the last one.
        steps.extend(typing(&mut kb, "reboot"));
        steps.extend(tap(&mut kb, &[Key::ControlLeft], Key::C));
        steps.extend(tap(&mut kb, &[], Key::ArrowUp));
        steps.extend(tap(&mut kb, &[], Key::NumpadEnter));
        let script = Script { shared: &shared, steps: steps.into() };
        let mut con = ScreenConsole::new(&shared, script);
        if shell::Console::width(&con) != 80 {
            return Err("the console does not have the grid's width");
        }
        shell::run(&mut con);
        let got = shared.with(|t| rows(t)).ok_or("the terminal went away")?;
        let want = ["poly> echo Screen!", "Screen!", "poly> reboot^C", "poly> echo Screen!", "Screen!", "poly>"];
        if got != want {
            kprintln!("terminal: the screen shows {got:?}, expected {want:?}");
            return Err("the screen does not show the typed commands and their output");
        }
        Ok(())
    }
}

kernel_test! {
    /// Log output while the prompt is shown goes above it, and the shell
    /// draws its prompt and line again below; Shift+Page Up scrolls back
    /// and typing scrolls forward again.
    fn log_output_moves_the_screen_prompt_down() -> TestResult {
        let mut cells = vec![Cell::BLANK; 80 * 106];
        let shared = Shared::new(false);
        *shared.term.lock() = Some(Term { t: small_terminal(&mut cells), log_pos: 0, carry: Default::default() });
        let mut kb = Keyboard::new(&layout::US);
        let mut steps = typing(&mut kb, "echo a");
        steps.push(Step::Log("disk: a log line\n"));
        steps.extend(typing(&mut kb, "b"));
        steps.extend(tap(&mut kb, &[], Key::Enter));
        for _ in 0..8 {
            steps.extend(tap(&mut kb, &[], Key::Enter));
        }
        steps.extend(tap(&mut kb, &[Key::ShiftLeft], Key::PageUp));
        let script = Script { shared: &shared, steps: steps.into() };
        let mut con = ScreenConsole::new(&shared, script);
        shell::run(&mut con);
        let (view, history) = shared.with(|t| (t.grid().view_offset(), t.grid().history())).ok_or("no terminal")?;
        if view != 5 {
            kprintln!("terminal: view {view} of {history} after Shift+PgUp");
            return Err("Shift+Page Up did not scroll the view back a page");
        }
        let lines: Vec<String> = shared
            .with(|t| (0..t.grid().history()).rev().filter_map(|n| t.grid().history_line(n)).map(|l| l.iter().map(|c| c.ch).collect::<String>()).collect())
            .ok_or("no terminal")?;
        let first: Vec<&str> = lines.iter().map(|l| l.trim_end()).take(3).collect();
        if first != ["disk: a log line", "poly> echo ab", "ab"] {
            kprintln!("terminal: the screen begins {first:?}");
            return Err("the log line did not go above a redrawn prompt");
        }
        // Typing snaps the view back.
        let mut steps = typing(&mut kb, "x");
        steps.extend(tap(&mut kb, &[Key::ControlLeft], Key::C));
        let script = Script { shared: &shared, steps: steps.into() };
        shell::run(&mut ScreenConsole::new(&shared, script));
        if shared.with(|t| t.grid().view_offset()) != Some(0) {
            return Err("typing did not scroll the view back to the screen");
        }
        Ok(())
    }
}

kernel_test! {
    /// Kernel log output reaches the ring the screen shows, and draining
    /// it puts it in a terminal's grid, in the log's colour.
    fn kernel_log_is_mirrored_for_the_screen() -> TestResult {
        if MIRROR_OFF.load(SeqCst) {
            return Ok(()); // no display: nothing reads the ring
        }
        let mut cells = vec![Cell::BLANK; 80 * 106];
        let shared = Shared::new(true);
        let start = LOG_WRITTEN.load(SeqCst);
        *shared.term.lock() = Some(Term { t: small_terminal(&mut cells), log_pos: start, carry: Default::default() });
        let marker = time::now_ns();
        kprintln!("terminal: mirror check {marker}");
        let want = alloc::format!("terminal: mirror check {marker}");
        let found = shared
            .with(|t| {
                let row = rows(t).iter().position(|r| *r == want);
                row.map(|r| t.grid().cell(0, r).map(|c| c.style))
            })
            .ok_or("no terminal")?;
        match found {
            Some(Some(style)) if style == poly_term::LOG_STYLE => {}
            Some(_) => return Err("the log line is not in the log's colour"),
            None => return Err("a kprintln did not reach the terminal's grid"),
        }
        Ok(())
    }
}

kernel_test! {
    /// A terminal renders onto a canvas the display's size: status bar on
    /// top, the shell's text below, nothing outside the screen.
    fn renders_a_display_sized_canvas() -> TestResult {
        let Some(info) = display::info() else { return Ok(()) };
        let screen = Screen { scrollback: 0, ..Screen::new(info.width, info.height).ok_or("no layout")? };
        let mut cells = vec![Cell::BLANK; screen.cells()];
        let mut t = Terminal::new(screen, &mut cells).map_err(|_| "no grid")?;
        t.set_status(status_info());
        t.write_shell("poly> help\n");
        // The top of the display (the status bar and the first text row):
        // drawing is clipped, and a whole screen's pixels may not fit one
        // heap allocation.
        let stride = info.width as usize * 4;
        let height = 2 * screen.font.height();
        let mut pixels = vec![0u8; stride * height as usize];
        let mut c =
            poly_gfx::Canvas::new(&mut pixels, info.width, height, stride, info.format).map_err(|_| "no canvas")?;
        if t.render(&mut c).is_none() {
            return Err("the first render drew nothing");
        }
        let bar = poly_term::status::BACKGROUND;
        let last = info.width as i32 - 1;
        if c.pixel(0, 0) != Some(bar) || c.pixel(last, screen.status.height as i32 - 1) != Some(bar) {
            return Err("the status bar does not span the top row");
        }
        let fg = poly_gfx::Palette::DEFAULT.foreground;
        let cell = screen.cell_rect(0, 0);
        let lit = (cell.y..cell.y + cell.height as i32).any(|y| (0..8).any(|x| c.pixel(x, y) == Some(fg)));
        if !lit {
            return Err("the shell's text is not below the status bar");
        }
        Ok(())
    }
}
