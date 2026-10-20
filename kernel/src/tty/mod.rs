//! Terminals in the kernel (ADR 0046): the host-tested `poly-tty` crate
//! (line discipline, terminals with job control, pseudo-terminals,
//! devpts) given the machine: who is calling, signals, blocking, user
//! memory, the serial port and the screen.
//!
//! ```text
//!   COM1 IRQ 4 ─► serial ring ─┬─► kernel shell "shell"        (while no program has ttyS0 open)
//!                              └─► thread "tty-serial" ─► ttyS0 ─► programs   (while one has)
//!   keyboard ─┬─► kernel shell "shell-screen"                  (while no program has tty1 open)
//!             └─► thread "tty-keys" ─► tty1 ─► programs           (while one has)
//!   ttyS0 output ─► log::write_tty (COM1, raw)      tty1 output ─► terminal::tty_output (the grid)
//!   /dev/console ─► the console terminal (`console=`, default ttyS0)
//!   /dev/tty ─► the caller's controlling terminal   /dev/ptmx, /dev/pts/<n> ─► pseudo-terminals
//! ```
//!
//! - **Devices**: `/dev/ttyS0` (4, 64) on COM1 (115200 baud, `CLOCAL`),
//!   `/dev/tty1` (4, 1) on the screen terminal and the keyboard (its window
//!   size is the terminal's grid), `/dev/console` (5, 1), `/dev/tty`
//!   (5, 0), `/dev/ptmx` (5, 2) and devpts on `/dev/pts`. They replace the
//!   console device `fs::devices` registers for (5, 0) and (5, 1), whose
//!   writes went to the kernel log and whose reads were end of file.
//! - **Sharing the console with the kernel shell.** A device's input goes
//!   to its terminal while any program has the terminal open, else to the
//!   kernel shell on that device. So the shells work as before when no
//!   program runs; `run <program>` from a shell hands the device to the
//!   program until it (and whatever it started that keeps the terminal
//!   open) ends; `poly.init=` programs get `/dev/console`, the terminal
//!   `console=ttyS0` or `console=tty1` names. Output from a program goes to
//!   the device whenever it writes; on the serial port it erases a shown
//!   shell prompt first, like kernel log lines.
//! - **Traced programs** (`poly.test.capture`, the diff runner's test runs)
//!   keep the old console on fds 0-2: writes to the kernel log, reads end
//!   of file, no terminal ([`log_console_files`]), so what they see does
//!   not depend on the kernel's console.
//! - **Signals** from terminals (`INTR`, `SIGWINCH`, `SIGTTIN`, hangups)
//!   are sent by the kernel ([`signal`]); the end of a process tells the
//!   terminals ([`before_exit`], [`after_exit`]): a controlling process
//!   hangs its session up, and process groups its end orphans that have a
//!   stopped member get `SIGHUP` and `SIGCONT`.

mod host;
pub mod signal;
mod tests;

use crate::boot_info::BootInfo;
use crate::fs::{self, FileType, Files, OpenFile, OpenFlags};
use crate::linux::Thread;
use crate::sync::SetOnce;
use crate::thread::WaitQueue;
use crate::{keyboard, kprintln, terminal, thread, time};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_tty::device::{Console, ControllingTty, TtyDevice};
use poly_tty::devpts::DevPts;
use poly_tty::hw;
use poly_tty::pty::{PTMX, Ptys};
use poly_tty::termios::{Termios, Winsize, cflag};
use poly_tty::{Driver, Tty, Ttys};
use poly_vfs::{At, Cred, DeviceId, MountFlags};

/// Kernel argument naming the console terminal: `ttyS0` (the default) or
/// `tty1` (bootparam(7) `console=`; options after a comma are ignored).
pub const CONSOLE_ARG: &str = "console";

/// The serial port's speed.
const SERIAL_SPEED: u32 = 115_200;
/// How often an input thread looks whether its terminal is still open.
const INPUT_POLL_NS: u64 = 50 * time::NANOS_PER_MILLI;

/// Everything this module set up.
struct Terminals {
    ttys: Arc<Ttys>,
    serial: Arc<Tty>,
    screen: Arc<Tty>,
    console: Arc<Console>,
    ptys: Arc<Ptys>,
}

static TERMINALS: SetOnce<Terminals> = SetOnce::new();

/// A program has ttyS0 / tty1 open: input goes to the terminal.
static SERIAL_OPEN: AtomicBool = AtomicBool::new(false);
static SCREEN_OPEN: AtomicBool = AtomicBool::new(false);
/// The input threads wait here for their terminal to be opened.
static OPENED: WaitQueue = WaitQueue::new();

fn terminals() -> Option<&'static Terminals> {
    TERMINALS.get()
}

/// Whether a program has the serial terminal open (the serial shell then
/// leaves the input alone).
pub fn serial_in_use() -> bool {
    SERIAL_OPEN.load(Ordering::SeqCst)
}

/// Whether a program has the screen's terminal open (the screen's shell
/// then leaves the keyboard alone).
pub fn screen_in_use() -> bool {
    SCREEN_OPEN.load(Ordering::SeqCst)
}

/// Whether the console terminal is the screen's (`console=tty1`), which
/// then has to be up before the first program runs.
pub fn console_is_screen(boot: &BootInfo) -> bool {
    console_name(boot) == "tty1"
}

fn console_name(boot: &BootInfo) -> &str {
    boot.arg(CONSOLE_ARG).map_or("ttyS0", |a| a.split(',').next().unwrap_or(a))
}

/// COM1 as a terminal's device: output straight to the port.
struct SerialDriver;

impl Driver for SerialDriver {
    fn write(&self, bytes: &[u8]) {
        crate::log::write_tty(bytes);
    }

    fn device_ioctl(&self, request: u32) -> Option<alloc::vec::Vec<u8>> {
        // COM1: a 16550A at port 0x3f8, ISA IRQ 4, 1.8432 MHz / 16.
        let info = hw::SerialInfo {
            kind: hw::PORT_16550A,
            line: 0,
            port: 0x3f8,
            irq: 4,
            xmit_fifo_size: 16,
            baud_base: 115_200,
        };
        hw::serial_ioctl(&info, request)
    }

    fn opened(&self, open: bool) {
        SERIAL_OPEN.store(open, Ordering::SeqCst);
        OPENED.wake_all();
    }
}

/// The screen terminal as a terminal's device.
struct ScreenDriver;

impl Driver for ScreenDriver {
    fn write(&self, bytes: &[u8]) {
        terminal::tty_output(bytes);
    }

    fn device_ioctl(&self, request: u32) -> Option<alloc::vec::Vec<u8>> {
        // tty1, the one virtual console.
        hw::vt_ioctl(1, request)
    }

    fn opened(&self, open: bool) {
        SCREEN_OPEN.store(open, Ordering::SeqCst);
        OPENED.wake_all();
    }
}

/// Set up the terminals and their devices (after `fs::init`, before the
/// first program).
pub fn init(boot: &BootInfo) {
    let ttys = Ttys::new(Arc::new(host::KernelHost));
    let mut serial_termios = Termios::sane(SERIAL_SPEED);
    serial_termios.cflag |= cflag::CLOCAL;
    let serial = Tty::new(&ttys, "ttyS0", Arc::new(SerialDriver), serial_termios, Winsize::default());
    let screen = Tty::new(&ttys, "tty1", Arc::new(ScreenDriver), Termios::sane(38400), Winsize::default());
    let name = console_name(boot);
    let console_tty = match name {
        "tty1" => screen.clone(),
        "ttyS0" => serial.clone(),
        other => {
            kprintln!("tty: no terminal {other} for {CONSOLE_ARG}=; the console is ttyS0");
            serial.clone()
        }
    };
    let console = Arc::new(Console::new(Some(console_tty.clone())));
    let vfs = fs::vfs();
    let ptys = Ptys::new(&ttys, vfs);
    // The console device fs::devices registered makes way for terminals.
    for id in [poly_tty::device::CONSOLE, poly_tty::device::TTY] {
        let _ = vfs.unregister_char_device(id);
    }
    let devices: [(DeviceId, Arc<dyn poly_vfs::CharDevice>); 5] = [
        (poly_tty::device::CONSOLE, console.clone()),
        (poly_tty::device::TTY, Arc::new(ControllingTty::new(ttys.clone()))),
        (PTMX, ptys.ptmx()),
        (poly_tty::device::serial(0), Arc::new(TtyDevice::new(serial.clone()))),
        (poly_tty::device::virtual_console(1), Arc::new(TtyDevice::new(screen.clone()))),
    ];
    for (id, dev) in devices {
        if let Err(e) = vfs.register_char_device(id, dev) {
            kprintln!("tty: device {}:{} not registered: {}", id.major, id.minor, e.name());
        }
    }
    make_nodes(&ptys);
    let _ = TERMINALS.set(Terminals { ttys, serial, screen, console, ptys });
    crate::shell::serial::start_receiving();
    for (name, f) in [("tty-serial", serial_input as thread::ThreadFn), ("tty-keys", key_input)] {
        if let Err(e) = thread::spawn_named(name, f, 0) {
            kprintln!("tty: the {name} thread could not start: {e:?}");
        }
    }
    kprintln!(
        "tty: ttyS0 (4,64) on COM1, tty1 (4,1) on the screen, /dev/console is {}, ptys on /dev/pts",
        console_tty.name()
    );
}

/// A devpts over the machine's pseudo-terminals, for a mount(2) of
/// `devpts` (ADR 0051): every instance shows the same ptys.
pub fn new_devpts() -> Option<Arc<dyn poly_vfs::FileSystem>> {
    Some(Arc::new(DevPts::new(terminals()?.ptys.clone())))
}

/// The nodes in `/dev` (those the root does not have yet) and devpts.
fn make_nodes(ptys: &Arc<Ptys>) {
    let vfs = fs::vfs();
    let Ok(mut ctx) = vfs.root_context() else { return };
    ctx.set_umask(0);
    let root = Cred::root();
    let nodes: [(&[u8], DeviceId, u16); 3] = [
        (b"/dev/ttyS0", poly_tty::device::serial(0), 0o660),
        (b"/dev/tty1", poly_tty::device::virtual_console(1), 0o620),
        (b"/dev/ptmx", PTMX, 0o666),
    ];
    for (path, id, perm) in nodes {
        match vfs.mknod(&ctx, &root, At::Cwd, path, FileType::CharDevice, perm, id) {
            Ok(()) | Err(poly_vfs::Errno::EEXIST) => {}
            Err(e) => kprintln!("tty: {}: {}", alloc::string::String::from_utf8_lossy(path), e.name()),
        }
    }
    match vfs.mkdir(&ctx, &root, At::Cwd, b"/dev/pts", 0o755) {
        Ok(()) | Err(poly_vfs::Errno::EEXIST) => {}
        Err(e) => kprintln!("tty: /dev/pts: {}", e.name()),
    }
    let devpts = Arc::new(DevPts::new(ptys.clone()));
    if let Err(e) = vfs.mount(&ctx, &root, At::Cwd, b"/dev/pts", devpts, MountFlags::default()) {
        kprintln!("tty: devpts not mounted on /dev/pts: {}", e.name());
    }
}

/// The terminal of the console, if the terminals are up.
pub fn console() -> Option<Arc<Tty>> {
    terminals().and_then(|t| t.console.tty())
}

/// The serial terminal (ttyS0).
pub fn serial() -> Option<&'static Arc<Tty>> {
    terminals().map(|t| &t.serial)
}

/// The screen's terminal (tty1).
pub fn screen() -> Option<&'static Arc<Tty>> {
    terminals().map(|t| &t.screen)
}

/// The pseudo-terminals.
pub fn ptys() -> Option<&'static Arc<Ptys>> {
    terminals().map(|t| &t.ptys)
}

/// The screen terminal came up with `cols` x `rows` cells: tty1's window
/// size.
pub fn screen_size(cols: usize, rows: usize) {
    if let Some(t) = screen() {
        let clamp = |v: usize| u16::try_from(v).unwrap_or(u16::MAX);
        t.set_winsize(Winsize::new(clamp(rows), clamp(cols)));
    }
}

// ---- input ---------------------------------------------------------------------

/// Typed bytes from COM1 to ttyS0 while a program has it open.
fn serial_input(_: usize) -> usize {
    let mut buf = [0u8; 64];
    loop {
        OPENED.wait_until(serial_in_use);
        crate::shell::serial::wait_for_input(time::now_ns() + INPUT_POLL_NS);
        if !serial_in_use() {
            continue;
        }
        let n = crate::shell::serial::take_input(&mut buf);
        if n > 0
            && let Some(t) = serial()
        {
            t.receive(&buf[..n]);
        }
    }
}

/// Keys from the keyboard to tty1 while a program has it open: the bytes
/// a terminal sends (`poly_term::keys`); Shift+Page Up / Down scroll the
/// screen's view as in the shell.
fn key_input(_: usize) -> usize {
    use poly_term::keys::{KeyAction, translate};
    loop {
        OPENED.wait_until(screen_in_use);
        if !keyboard::is_present() {
            thread::sleep_until(time::now_ns() + 10 * INPUT_POLL_NS);
            continue;
        }
        let Some(input) = keyboard::next_shared_event_until(time::now_ns() + INPUT_POLL_NS) else { continue };
        match translate(&input) {
            KeyAction::None => {}
            KeyAction::ScrollPages(pages) => terminal::scroll_pages(pages),
            KeyAction::Send(bytes) => {
                terminal::scroll_to_bottom();
                if let Some(t) = screen() {
                    t.receive(bytes.as_bytes());
                }
            }
        }
    }
}

// ---- processes -------------------------------------------------------------------

/// What [`after_exit`] needs from before a process's end.
#[derive(Debug, Default)]
pub struct BeforeExit {
    sid: Option<i32>,
    groups: Vec<(i32, bool)>,
}

/// Process `pid` is about to end: its session, and the groups its end
/// may orphan.
pub fn before_exit(pid: u32) -> BeforeExit {
    crate::linux::task::with_tree(|t| BeforeExit {
        sid: t.sid(pid as i32),
        groups: poly_personality_linux::task::jobs::before_exit(t, pid as i32),
    })
}

/// Process `pid` (the running thread's, `k`) ended: a controlling process
/// hangs its session up; groups that its end orphaned get `SIGHUP` and
/// `SIGCONT` if a member is stopped (POSIX `_exit`).
pub fn after_exit(k: &mut Thread<'_>, pid: u32, before: BeforeExit) {
    let Some(t) = terminals() else { return };
    if let Some(sid) = before.sid {
        t.ttys.process_exited(pid as i32, sid);
    }
    use poly_personality_linux::task::jobs;
    let orphaned = crate::linux::task::with_tree(|tree| jobs::newly_orphaned(tree, &before.groups));
    for pgid in orphaned {
        let members = crate::linux::task::with_tree(|tree| jobs::members(tree, pgid));
        let stopped =
            members.iter().any(|&m| crate::process::find(m as u32).is_some_and(|p| p.is_stopped() && !p.has_ended()));
        if stopped {
            for &m in &members {
                signal::send(m as u32, poly_tty::sig::SIGHUP, Some(&mut *k));
                signal::send(m as u32, poly_tty::sig::SIGCONT, Some(&mut *k));
            }
        }
    }
}

// ---- traced programs ------------------------------------------------------------------

/// The console the kernel log is: writes are log lines, reads end of file,
/// no terminal (`ENOTTY` for terminal requests).
struct LogConsole(fs::devices::Console);

impl poly_vfs::FileObject for LogConsole {
    fn read(&self, _: &mut [u8]) -> poly_vfs::Result<usize> {
        Ok(0)
    }

    fn write(&self, buf: &[u8]) -> poly_vfs::Result<usize> {
        poly_vfs::CharDevice::write(&self.0, 0, buf, false)
    }

    fn poll(&self) -> poly_vfs::PollEvents {
        poly_vfs::PollEvents::READABLE | poly_vfs::PollEvents::WRITABLE
    }

    fn wait_queue(&self) -> Option<&Arc<poly_vfs::WaitQueue>> {
        None
    }
}

/// The files of a traced process (`poly.test.capture`): the kernel log's
/// console on fds 0, 1 and 2 (one open file description), as every
/// process had before terminals, so a test run does not depend on the
/// console terminal.
pub fn log_console_files() -> Files {
    let files = Files::new();
    let object = Arc::new(LogConsole(fs::devices::Console::new()));
    let Ok(node) = fs::vfs().anon_node(FileType::CharDevice, 0o600, &files.cred()) else { return files };
    let file = Arc::new(OpenFile::on_object(node, object, OpenFlags::RDWR));
    if files.install(file, false).is_ok() {
        let _ = files.dup(0).and_then(|_| files.dup(0));
    }
    files
}
