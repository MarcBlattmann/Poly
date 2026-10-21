//! The kernel's side of the console host ([ADR 0080]): which terminal a
//! console shows on, feeding it the terminal's input, giving a new process
//! its console and standard handles, and running control handlers.
//!
//! The console itself — modes, input, line editing, the screen buffer and
//! the VT bytes that mirror it — is `poly_personality_nt::console` and
//! `poly_console`, host-tested. Here:
//!
//! - [`setup`] (from [`super::start_with`]): a console program started on
//!   one of Poly's terminals (its descriptors 0–2, a Linux-side file table)
//!   attaches to that terminal's console, made on first use; descriptors
//!   that are not terminals (files, pipes) become ordinary file handles
//!   (Learn: "Console Handles": redirected standard handles are not console
//!   handles). A program without a terminal gets a console that shows
//!   nowhere (as Windows gives a detached process a window it never shows),
//!   so its `CONOUT$` still works. A GUI program gets no console.
//! - [`drive`] (after every NT system call and process exit, and after
//!   input): sends each console's VT bytes to its terminal and acts on the
//!   console events.
//! - Input: once a console needs it (a read, or a program that takes
//!   Ctrl+C), its terminal is put in raw mode (the console does its own
//!   echo and line editing, per the Windows modes) and a reader thread
//!   feeds it; when the console closes, the terminal's settings come back.
//! - A control event (Ctrl+C) starts a thread at the routine `kernelbase`
//!   registered, in each attached process.
//!
//! [ADR 0080]: ../../../docs/adr/0080-console-host.md

use super::{io, sched, with};
use crate::process::Process;
use crate::sync::Mutex;
use crate::thread;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_ntobj::{Caller, Handle, NtStatus, ProcessId, ThreadRef};
use poly_personality_nt::backend::Backend;
use poly_personality_nt::console::ConsoleEvent;
use poly_personality_nt::io::consts::{FILE_GENERIC_READ, FILE_GENERIC_WRITE};
use poly_personality_nt::system::NtSystem;
use poly_tty::Tty;
use poly_tty::termios::{Termios, cc, iflag, lflag, oflag};
use poly_vfs::OpenFile;

/// A console's terminal.
struct Link {
    tty: Option<Arc<Tty>>,
    /// The terminal's settings before the console put it in raw mode.
    saved: Option<Termios>,
    /// The reader thread runs while this is set.
    reading: Option<Arc<AtomicBool>>,
}

/// Each console's terminal, by console id. Taken after the NT lock, never
/// while calling into it.
static LINKS: Mutex<BTreeMap<u32, Link>> = Mutex::new(BTreeMap::new());

/// What a new process's standard handle comes from.
#[derive(Clone)]
pub enum StdSource {
    /// One of Poly's terminals: the console's handle.
    Terminal(Arc<Tty>),
    /// Anything else: an ordinary file handle on it.
    File(Arc<OpenFile>),
    None,
}

/// The terminal an open file is, by its device number (`tty(4)`,
/// `console(4)`, `pts(4)`).
fn tty_of(open: &OpenFile) -> Option<Arc<Tty>> {
    if open.kind() != poly_vfs::FileType::CharDevice {
        return None;
    }
    let rdev = open.stat().ok()?.rdev;
    if rdev == poly_tty::device::CONSOLE {
        return crate::tty::console();
    }
    if rdev == poly_tty::device::serial(0) {
        return crate::tty::serial().cloned();
    }
    if rdev == poly_tty::device::virtual_console(1) {
        return crate::tty::screen().cloned();
    }
    if rdev.major == poly_tty::pty::PTY_SLAVE_MAJOR {
        return crate::tty::ptys()?.get(rdev.minor).map(|p| Arc::clone(p.slave()));
    }
    None
}

/// The sources of `process`'s standard handles: its descriptors 0, 1, 2.
pub fn std_sources(process: &Process) -> [StdSource; 3] {
    core::array::from_fn(|fd| match process.files.file(fd as i32) {
        Ok(open) => match tty_of(&open) {
            Some(tty) => StdSource::Terminal(tty),
            None => StdSource::File(open),
        },
        Err(_) => StdSource::None,
    })
}

/// The console showing on `tty`, made if there is none.
fn console_for(sys: &mut NtSystem, tty: Option<&Arc<Tty>>) -> u32 {
    let mut links = LINKS.lock();
    if let Some(tty) = tty
        && let Some((&id, _)) = links
            .iter()
            .find(|(id, l)| l.tty.as_ref().is_some_and(|t| Arc::ptr_eq(t, tty)) && sys.io.console(**id).is_some())
    {
        return id;
    }
    let (cols, rows) = tty.map_or((0, 0), |t| {
        let w = t.winsize();
        (w.cols, w.rows)
    });
    let id = sys.io.create_console(cols, rows);
    links.insert(id, Link { tty: tty.cloned(), saved: None, reading: None });
    id
}

/// Give NT process `pid` (its first thread `thread`) its console and
/// standard handles: `(console connection, [input, output, error])`, the
/// values for its process parameters. `console_app`: the image's subsystem
/// is `IMAGE_SUBSYSTEM_WINDOWS_CUI`.
pub(super) fn setup(
    sys: &mut NtSystem,
    b: &mut dyn Backend,
    pid: ProcessId,
    thread: ThreadRef,
    console_app: bool,
    sources: &[StdSource; 3],
) -> Result<(u64, [u64; 3]), NtStatus> {
    let caller = Caller::user(pid, thread);
    let mut std = [0u64; 3];
    let mut connection = 0u64;
    if console_app {
        let tty = sources.iter().find_map(|s| match s {
            StdSource::Terminal(t) => Some(t),
            _ => None,
        });
        let id = console_for(sys, tty);
        let handles = poly_personality_nt::console::open_for_process(sys, b, &caller, id)?;
        connection = handles[0].0;
        for (i, source) in sources.iter().enumerate() {
            if !matches!(source, StdSource::File(_)) {
                std[i] = handles[i + 1].0;
            } else {
                // Redirected: the console's handle is not needed.
                let _ = sys.objects.close(&caller, handles[i + 1]);
            }
        }
    }
    for (i, source) in sources.iter().enumerate() {
        if let StdSource::File(open) = source {
            let access = if i == 0 { FILE_GENERIC_READ } else { FILE_GENERIC_WRITE };
            let h: Handle = sys.io.open_host_file(&mut sys.objects, &caller, Arc::clone(open), access)?;
            std[i] = h.0;
        }
    }
    sys.reap(b);
    Ok((connection, std))
}

/// Raw mode for a console's terminal: no line editing, echo or signals,
/// no input or output translation; reads return each byte as it comes.
fn raw(t: &Termios) -> Termios {
    let mut r = *t;
    r.lflag &= !(lflag::ICANON | lflag::ECHO | lflag::ISIG | lflag::IEXTEN);
    r.iflag &= !(iflag::ICRNL | iflag::INLCR | iflag::IGNCR | iflag::IXON);
    r.oflag &= !oflag::OPOST;
    r.cc[cc::VMIN] = 1;
    r.cc[cc::VTIME] = 0;
    r
}

/// Send `bytes` to `tty` without blocking (output the terminal has no
/// room for is dropped).
fn send(tty: &Tty, bytes: &[u8]) {
    let mut done = 0;
    let mut tries = 0;
    while done < bytes.len() && tries < 64 {
        match tty.write(&bytes[done..], true) {
            Ok(n) if n > 0 => done += n,
            _ => {
                tries += 1;
                thread::yield_now();
            }
        }
    }
}

/// Send the consoles' output to their terminals and act on their events.
pub(super) fn drive() {
    let (outputs, events) = with(|s| (s.sys.io.take_console_output(), s.sys.io.take_console_events()));
    if outputs.is_empty() && events.is_empty() {
        return;
    }
    for (id, bytes) in outputs {
        let tty = LINKS.lock().get(&id).and_then(|l| l.tty.clone());
        if let Some(tty) = tty {
            send(&tty, &bytes);
        }
    }
    for event in events {
        match event {
            ConsoleEvent::Created { id } => {
                // AllocConsole: Poly's system console (Poly's choice: there
                // is no window to open).
                let tty = crate::tty::console();
                if let Some(t) = &tty {
                    let w = t.winsize();
                    with(|s| s.sys.io.resize_console(id, w.cols, w.rows));
                }
                LINKS.lock().entry(id).or_insert(Link { tty, saved: None, reading: None });
            }
            ConsoleEvent::InputWanted { id } => start_input(id),
            ConsoleEvent::Closed { id, output } => {
                let link = LINKS.lock().remove(&id);
                if let Some(link) = link {
                    if let Some(flag) = &link.reading {
                        flag.store(false, Ordering::SeqCst);
                    }
                    if let Some(tty) = &link.tty {
                        send(tty, &output);
                        if let Some(saved) = link.saved {
                            tty.set_termios(saved, false);
                        }
                    }
                }
            }
            ConsoleEvent::Ctrl { process, routine, event } => ctrl_thread(process, routine, event),
        }
    }
}

/// Put console `id`'s terminal in raw mode and start its reader.
fn start_input(id: u32) {
    let mut links = LINKS.lock();
    let Some(link) = links.get_mut(&id) else { return };
    let Some(tty) = link.tty.clone() else { return };
    if link.reading.is_some() {
        return;
    }
    let saved = tty.termios();
    tty.set_termios(raw(&saved), false);
    link.saved = Some(saved);
    let flag = Arc::new(AtomicBool::new(true));
    link.reading = Some(Arc::clone(&flag));
    drop(links);
    match thread::spawn_named("nt-console", reader_main, id as usize) {
        Ok(tid) => {
            let _ = thread::detach(tid);
        }
        Err(_) => crate::kprintln!("nt: no reader thread for console {id}"),
    }
}

/// How often a reader looks at its terminal when there was nothing.
const POLL_MS: u64 = 10;

/// A console's reader: the terminal's bytes into the console, until the
/// console closes.
fn reader_main(arg: usize) -> usize {
    let id = arg as u32;
    let Some((tty, flag)) = LINKS.lock().get(&id).and_then(|l| Some((l.tty.clone()?, Arc::clone(l.reading.as_ref()?))))
    else {
        return 0;
    };
    let mut buf = [0u8; 256];
    while flag.load(Ordering::SeqCst) {
        match tty.read(&mut buf, true) {
            Ok(n) if n > 0 => {
                let input = &buf[..n];
                with(|s| s.sys.io.console_input(id, input));
                // Reads it finished, and Ctrl+C.
                io::after_call();
                drive();
            }
            _ => thread::sleep(core::time::Duration::from_millis(POLL_MS)),
        }
    }
    0
}

/// `THREAD_ALL_ACCESS` (winnt.h).
const THREAD_ALL_ACCESS: u32 = 0x001F_FFFF;

/// Run `routine(event)` on a new thread of NT process `process`.
fn ctrl_thread(process: ProcessId, routine: u64, event: u32) {
    let Some(memory) = io::memory_of(process.0) else { return };
    let creator = ThreadRef(u64::from(process.0));
    let req = poly_personality_nt::ThreadCreate {
        start_routine: routine,
        argument: u64::from(event),
        stack_reserve: 0,
        stack_commit: 0,
        suspended: false,
        desired_access: THREAD_ALL_ACCESS,
    };
    match sched::create_thread(process, creator, &memory, req) {
        Ok(created) => {
            with(|s| {
                let _ = s.sys.objects.close(&Caller::user(process, creator), created.handle);
            });
        }
        Err(e) => crate::kprintln!("nt: no control-handler thread in pid {}: {:#x}", process.0, e.0),
    }
}

/// For the kernel tests: the console a terminal shows, if any.
#[allow(dead_code, reason = "read by the kernel tests")]
pub fn console_of(tty: &Arc<Tty>) -> Option<u32> {
    let ids: Vec<u32> = LINKS
        .lock()
        .iter()
        .filter(|(_, l)| l.tty.as_ref().is_some_and(|t| Arc::ptr_eq(t, tty)))
        .map(|(&id, _)| id)
        .collect();
    ids.into_iter().find(|&id| with(|s| s.sys.io.console(id).is_some()))
}
