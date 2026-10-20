//! The kernel shell: a prompt, a line editor and a few commands that show
//! what the kernel knows (memory, threads, devices, disks, interrupts) and
//! run its tests, until there are user-mode programs.
//!
//! The shell never touches a device. It talks to a [`Console`]: text and
//! ANSI escape sequences out, line-editing events ([`Edit`]) in. Today the
//! console is the serial port ([`serial::SerialConsole`]: COM1 receive
//! interrupts, terminal byte stream decoded by `poly_line::TermDecoder`);
//! the screen and keyboard become another `Console` (keys mapped to `Edit`s,
//! text drawn by a text grid) without changes here. The line editor is the
//! host-tested `poly-line` crate.
//!
//! ```text
//!  Console::read ──Edit──► poly_line::Editor ──line──► words::split ──► COMMANDS[name](console, args)
//!        ▲                        │                                          │
//!        └── serial / keyboard    └──── redraw (text + escapes) ──► Console ◄┘ output
//! ```
//!
//! [`run`] is the loop: prompt, read a line, run it, forever (or until a
//! scripted console runs out of input, which is how the kernel tests drive
//! it). [`spawn`] runs it in a kernel thread of its own; [`start_serial`]
//! does that on COM1, which `kernel_entry` calls unless the boot is an
//! automated test run (`poly.qemu_exit` without `poly.shell`).

mod commands;
mod net;
pub mod serial;

use crate::boot_info::BootInfo;
use crate::ktest;
use crate::sync::SetOnce;
use crate::thread::{self, SpawnError, ThreadId};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
pub use commands::COMMANDS;
use core::fmt;
use poly_line::words::{self, SplitError};
use poly_line::{Action, Completion, Edit, Editor};
use poly_mem::MemoryMap;

/// Kernel flag: start the shell even when `poly.qemu_exit` asks for an
/// automated test run (after the tests pass), so a boot test can type into
/// it. Without `poly.qemu_exit` the shell always starts.
pub const SHELL_FLAG: &str = "poly.shell";

/// The prompt: "poly> " in bold green.
pub const PROMPT: &str = "\x1b[1;32mpoly>\x1b[0m ";

/// Lines the history keeps.
pub const HISTORY: usize = 64;

/// Where a shell reads from and writes to.
///
/// Output is UTF-8 text with the escape sequences `poly-line` documents
/// (cursor left / right, erase to end of line, erase screen, cursor home)
/// plus SGR colors (`ESC [ 0 m`, `1`, `30`-`37`, `90`-`97`). `"\n"` means
/// "start of the next line"; a console that needs CR LF adds the CR.
pub trait Console: fmt::Write {
    /// The next input event, waiting for one if there is none yet. `None`
    /// ends the shell: a scripted console ran out of input. Consoles on
    /// devices never return `None`.
    fn read(&mut self) -> Option<Edit>;

    /// Columns per line, for listings.
    fn width(&self) -> usize {
        poly_line::DEFAULT_WIDTH
    }

    /// The terminal device on the same device as this console
    /// (`/dev/ttyS0`, `/dev/tty1`): what `run` gives the program it starts
    /// (ADR 0046). `None`: the console terminal.
    fn terminal(&self) -> Option<&'static str> {
        None
    }
}

/// SGR sequences the commands use.
pub mod sgr {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
}

/// Run the shell on `con`: prompt, read a line, run it, until `con.read()`
/// returns `None`.
pub fn run(con: &mut dyn Console) {
    let mut editor = Editor::new(HISTORY);
    editor.set_width(con.width());
    loop {
        editor.start(PROMPT, con);
        let line = loop {
            let Some(edit) = con.read() else { return };
            match editor.handle(edit, con, &mut complete) {
                Action::Pending => {}
                Action::Done(line) => break Some(line),
                Action::Cancelled => break None,
            }
        };
        if let Some(line) = line {
            execute(con, &line);
        }
    }
}

/// Run one command line on `con`.
pub fn execute(con: &mut dyn Console, line: &str) {
    let args = match words::split(line) {
        Ok(args) => args,
        Err(e @ (SplitError::UnclosedQuote | SplitError::TrailingBackslash)) => {
            error(con, format_args!("{}", e.describe()));
            return;
        }
    };
    let Some(name) = args.first() else { return };
    let Some(cmd) = COMMANDS.iter().find(|c| c.name == name) else {
        unknown(con, name);
        return;
    };
    if let Err(e) = (cmd.run)(con, &args[1..]) {
        error(con, format_args!("{}: {e}", cmd.name));
    }
}

fn unknown(con: &mut dyn Console, name: &str) {
    let names = COMMANDS.iter().map(|c| c.name);
    match words::closest(name, names) {
        Some(guess) => error(con, format_args!("unknown command '{name}'; did you mean '{guess}'?")),
        None => error(con, format_args!("unknown command '{name}'; type 'help' for a list")),
    }
}

/// Print an error line in red.
fn error(con: &mut dyn Console, msg: fmt::Arguments) {
    let _ = writeln!(con, "{}{msg}{}", sgr::RED, sgr::RESET);
}

/// Tab completion: command names for the first word, `help`'s argument, and
/// kernel test names for `test`.
fn complete(before: &str) -> Completion {
    let start = before.rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let first = before.split_whitespace().next().filter(|_| start > 0);
    let candidates: Vec<String> = match first {
        None | Some("help") => COMMANDS.iter().map(|c| c.name.into()).collect(),
        Some("test") => ktest::all().iter().map(|t| t.name().into()).collect(),
        Some(_) => Vec::new(),
    };
    Completion { start, candidates }
}

/// What the `test` command runs the kernel tests with.
struct TestContext {
    boot: BootInfo,
    memory: &'static MemoryMap,
}

static TEST_CONTEXT: SetOnce<TestContext> = SetOnce::new();

/// Give the `test` command the boot information and memory map the tests
/// need. The boot's `poly.selftest.*` flags (which make tests fail or panic
/// on purpose) and its `poly.test` filter are dropped. Call once.
pub fn set_test_env(boot: &BootInfo, memory: &'static MemoryMap) {
    let boot = boot.filtered(|w| !w.starts_with("poly.selftest.") && !w.starts_with("poly.test="));
    if TEST_CONTEXT.set(TestContext { boot, memory }).is_err() {
        panic!("shell::set_test_env called twice");
    }
}

/// Run the shell on `con` in a new kernel thread named `name`.
pub fn spawn(name: &'static str, con: Box<dyn Console + Send>) -> Result<ThreadId, SpawnError> {
    // A thin pointer to the fat one, through the thread's argument.
    let arg = Box::into_raw(Box::new(con));
    thread::spawn_named(name, shell_main, arg as usize).inspect_err(|_| {
        // SAFETY: the thread was not created, so the box is still ours.
        drop(unsafe { Box::from_raw(arg) });
    })
}

fn shell_main(arg: usize) -> usize {
    // SAFETY: `spawn` leaked this box for this thread, which takes it over.
    let mut con = unsafe { Box::from_raw(arg as *mut Box<dyn Console + Send>) };
    run(&mut **con);
    0
}

/// Start the shell on the serial console (COM1) in a thread named "shell".
/// Call once, after the kernel tests.
pub fn start_serial() -> Result<ThreadId, SpawnError> {
    let con = serial::SerialConsole::take().expect("the serial console is taken once");
    spawn("shell", Box::new(con))
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use poly_line::TermDecoder;

/// A console that types `input` (terminal bytes, as a serial line would
/// carry them) and records the output.
struct Script {
    input: &'static [u8],
    decoder: TermDecoder,
    output: String,
}

impl fmt::Write for Script {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.output.push_str(s);
        Ok(())
    }
}

impl Console for Script {
    fn read(&mut self) -> Option<Edit> {
        while let Some((&b, rest)) = self.input.split_first() {
            self.input = rest;
            if let Some(e) = self.decoder.feed(b) {
                return Some(e);
            }
        }
        None
    }
}

/// Run the shell on `input` and return its output without escape
/// sequences (cursor movements included, so only plain typing reads well).
fn session(input: &'static str) -> String {
    let mut s = Script { input: input.as_bytes(), decoder: TermDecoder::new(), output: String::new() };
    run(&mut s);
    let mut plain = String::new();
    let mut chars = s.output.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ESC [ parameters final
            chars.by_ref().skip(1).find(|c| c.is_ascii_alphabetic());
        } else {
            plain.push(c);
        }
    }
    plain
}

/// The output of the command typed on line `n` (from 0) of a session: what
/// follows that line's prompt row up to the next prompt.
fn output_of(session: &str, n: usize) -> &str {
    let mut parts = session.split("poly> ").skip(n + 1);
    let part = parts.next().unwrap_or("");
    part.split_once('\n').map_or("", |(_, out)| out)
}

kernel_test! {
    fn help_lists_every_command() -> TestResult {
        let out = session("help\r");
        let listed = output_of(&out, 0);
        for c in COMMANDS {
            if !listed.lines().any(|l| l.trim_start().starts_with(c.name) && l.contains(c.help)) {
                crate::kprintln!("shell: help does not list {}:\n{listed}", c.name);
                return Err("help does not list a command with its help text");
            }
        }
        let one = session("help uptime\r");
        if !output_of(&one, 0).contains(COMMANDS.iter().find(|c| c.name == "uptime").unwrap().help) {
            return Err("help <command> does not describe it");
        }
        Ok(())
    }
}

kernel_test! {
    fn commands_report_the_machine() -> TestResult {
        let out = session("uptime\rmem\rps\rlspci\rdisks\rirq\rcpu\r");
        let check = |n: usize, what: &str, needles: &[&str]| {
            let o = output_of(&out, n);
            if needles.iter().all(|s| o.contains(s)) {
                Ok(())
            } else {
                crate::kprintln!("shell: {what} printed:\n{o}");
                Err("a command's output is missing something")
            }
        };
        check(0, "uptime", &["up ", " ticks"])?;
        check(1, "mem", &["RAM", " total, ", " free, ", "heap", " in use, "])?;
        check(2, "ps", &["idle", "runnable", "running"])?;
        check(3, "lspci", &["00:00.0", "host bridge", " functions"])?;
        check(4, "disks", &["ram0", "RAM disk", "256 KiB", "hits", "polled"])?;
        check(5, "irq", &["0xf0", "timer"])?;
        check(6, "cpu", &["vendor", "family", "features", " lm"])?;
        Ok(())
    }
}

kernel_test! {
    fn echo_splits_words_and_reports_bad_quotes() -> TestResult {
        let out = session("echo  a 'b  c' \"d\\\"e\"\recho 'oops\r");
        if output_of(&out, 0) != "a b  c d\"e\n" {
            return Err("echo printed the wrong words");
        }
        if !output_of(&out, 1).contains("unclosed quote") {
            return Err("an unclosed quote was not reported");
        }
        Ok(())
    }
}

kernel_test! {
    fn unknown_commands_get_a_suggestion() -> TestResult {
        let out = session("hlep\rxyzzy\r\r");
        if !output_of(&out, 0).contains("unknown command 'hlep'; did you mean 'help'?") {
            return Err("no suggestion for a typo");
        }
        if !output_of(&out, 1).contains("unknown command 'xyzzy'; type 'help'") {
            return Err("an unknown command without a close match was not reported");
        }
        if !output_of(&out, 2).is_empty() {
            return Err("an empty line printed something");
        }
        Ok(())
    }
}

kernel_test! {
    fn editing_history_and_completion_reach_the_commands() -> TestResult {
        // Recall "echo one" with Up and change it; complete "ech" with Tab;
        // Ctrl-C a line; fix a typo with Left, Backspace and Ctrl-E.
        let out = session("echo one\r\x1b[A\x7f\x7f\x7ftwo\rech\tthree\rreboot\x03echo frox\x1b[D\x1b[D\x7f\x05 x\r");
        let want = ["one\n", "two\n", "three\n", "", "fox x\n"];
        for (n, w) in want.iter().enumerate() {
            if output_of(&out, n) != *w {
                crate::kprintln!("shell: line {n} printed {:?}, expected {w:?}", output_of(&out, n));
                return Err("an edited line did not run as expected");
            }
        }
        Ok(())
    }
}

kernel_test! {
    fn test_command_does_not_nest() -> TestResult {
        // This test is itself run by ktest, so the command must refuse.
        let out = session("test shell::\r");
        if !output_of(&out, 0).contains("already running") {
            return Err("the test command ran tests inside a test");
        }
        Ok(())
    }
}

kernel_test! {
    fn completion_knows_commands_and_tests() -> TestResult {
        let c = complete("ls");
        if c.start != 0 || !c.candidates.iter().any(|n| n == "lspci") {
            return Err("command names are not completed");
        }
        let c = complete("test shell::he");
        if c.start != 5 || !c.candidates.iter().any(|n| n == "shell::help_lists_every_command") {
            return Err("test names are not completed");
        }
        if !complete("echo x").candidates.is_empty() {
            return Err("echo's arguments were completed");
        }
        Ok(())
    }
}
