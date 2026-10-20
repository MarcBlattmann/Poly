//! The shell's commands. Each one reads kernel state through the modules'
//! public APIs and prints it; none changes anything except `test` (which
//! runs the kernel tests), `mount` and `umount` with arguments, `write`,
//! `sync`, `run` (which starts a program), `clear` and `reboot`.

use super::{Console, TEST_CONTEXT, sgr};
use crate::arch::{self, cpu, irq};
use crate::block::{self, Waiting};
use crate::frames::FRAMES;
use crate::fs::{self, FileType};
use crate::ktest::{self, RunError, TestEnv, When};
use crate::pci::{self, Size};
use crate::thread::{self, State};
use crate::{heap, power, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;
use poly_mem::PAGE_SIZE;
use poly_pci::BarKind;

/// What a command returns: `Err` is printed in red after the command's name.
pub type CommandResult = Result<(), String>;

/// One shell command.
#[derive(Debug)]
pub struct Command {
    pub name: &'static str,
    /// Its arguments, for `help`: `"[filter]"`, `""` for none.
    pub usage: &'static str,
    /// One line for `help`.
    pub help: &'static str,
    pub run: fn(&mut dyn Console, &[String]) -> CommandResult,
}

/// Every command, in the order `help` lists them.
pub static COMMANDS: &[Command] = &[
    Command { name: "help", usage: "[command]", help: "list the commands, or describe one", run: help },
    Command { name: "mem", usage: "", help: "physical memory and kernel heap usage", run: mem },
    Command { name: "ps", usage: "", help: "kernel threads and their states", run: ps },
    Command { name: "lspci", usage: "", help: "PCI functions found at boot", run: lspci },
    Command { name: "disks", usage: "", help: "block devices with their cache and queue counters", run: disks },
    Command { name: "ls", usage: "[path]", help: "list a directory (default /) with modes and sizes", run: ls },
    Command { name: "cat", usage: "<file>...", help: "print files", run: cat },
    Command {
        name: "mount",
        usage: "[<disk> <path> [ro]]",
        help: "mounted file systems, or mount a disk's ext2/3/4 volume on path",
        run: mount,
    },
    Command { name: "umount", usage: "<path>", help: "unmount the disk mounted on path", run: umount },
    Command { name: "write", usage: "<file> [words...]", help: "write the words and a newline to a file", run: write },
    Command { name: "sync", usage: "", help: "write back cached file data and disk caches", run: sync },
    Command {
        name: "run",
        usage: "[NAME=value...] <program> [args]",
        help: "run a program until it ends, with the variables as its environment",
        run: run_program,
    },
    Command { name: "uptime", usage: "", help: "time since boot and timer ticks", run: uptime },
    Command { name: "irq", usage: "", help: "interrupt counts per vector", run: irqs },
    Command { name: "cpu", usage: "", help: "processor vendor, model and features (CPUID)", run: cpu_info },
    Command {
        name: "ifconfig",
        usage: "",
        help: "network interface: addresses, link, counters",
        run: super::net::ifconfig,
    },
    Command { name: "ip", usage: "", help: "the same as ifconfig", run: super::net::ifconfig },
    Command { name: "ping", usage: "[-c count] <ipv4>", help: "send ICMP echo requests", run: super::net::ping },
    Command {
        name: "nc",
        usage: "<ipv4> <port> [text...]",
        help: "send text over TCP, print the answer",
        run: super::net::nc,
    },
    Command { name: "test", usage: "[filter]", help: "run the kernel tests whose name contains filter", run: test },
    Command { name: "echo", usage: "[words...]", help: "print the words", run: echo },
    Command { name: "clear", usage: "", help: "clear the screen", run: clear },
    Command { name: "reboot", usage: "", help: "write back the disk caches and reset the machine", run: reboot },
];

/// Ignore console write errors: a console cannot refuse text.
macro_rules! out {
    ($con:expr, $($arg:tt)*) => {{
        let _ = writeln!($con, $($arg)*);
    }};
}

fn no_args(args: &[String]) -> CommandResult {
    if args.is_empty() { Ok(()) } else { Err("takes no arguments".into()) }
}

fn help(con: &mut dyn Console, args: &[String]) -> CommandResult {
    match args {
        [] => {
            let width = COMMANDS.iter().map(|c| c.name.len() + 1 + c.usage.len()).max().unwrap_or(0);
            for c in COMMANDS {
                let call = format!("{} {}", c.name, c.usage);
                out!(con, "  {}{call:width$}{}  {}", sgr::BOLD, sgr::RESET, c.help);
            }
            out!(con, "Keys: Tab completes, Up/Down recall lines, Ctrl-A/E/K/U/W edit, Ctrl-C cancels, Ctrl-L clears.");
            Ok(())
        }
        [name] => {
            let c = COMMANDS.iter().find(|c| c.name == name).ok_or_else(|| format!("no command '{name}'"))?;
            out!(con, "{}{} {}{}: {}", sgr::BOLD, c.name, c.usage, sgr::RESET, c.help);
            Ok(())
        }
        _ => Err("takes at most one command name".into()),
    }
}

/// `bytes` in the largest binary unit that keeps it at 1 or more, with one
/// decimal: "245.3 MiB".
fn human(bytes: u64) -> String {
    let units = [(40, "TiB"), (30, "GiB"), (20, "MiB"), (10, "KiB")];
    match units.iter().find(|&&(shift, _)| bytes >= 1 << shift) {
        Some(&(shift, unit)) => {
            let tenths = (bytes * 10 + (1 << shift) / 2) >> shift;
            format!("{}.{} {unit}", tenths / 10, tenths % 10)
        }
        None => format!("{bytes} B"),
    }
}

fn mem(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    let (total, free) = {
        let frames = FRAMES.lock();
        let f = frames.as_ref().ok_or("the frame allocator is not set up")?;
        (f.total_frames() * PAGE_SIZE, f.free_frames() * PAGE_SIZE)
    };
    out!(
        con,
        "RAM   {} total, {} free, {} used ({} frames of 4 KiB)",
        human(total),
        human(free),
        human(total - free),
        total / PAGE_SIZE
    );
    let h = heap::stats();
    out!(
        con,
        "heap  {} in use, {} held from the frame allocator ({} pages, {} in {} large runs), {} failed allocations",
        human(h.bytes_in_use as u64),
        human((h.pages_held * PAGE_SIZE as usize) as u64),
        h.pages_held,
        h.large_pages,
        h.large_allocations,
        h.failed_allocations
    );
    let c = fs::cache::cache().stats();
    out!(
        con,
        "cache {} pages ({}, {} frames), {} dirty, limit {}; {} hits, {} misses, {} read, {} written, {} evicted",
        c.pages,
        human((c.pages * PAGE_SIZE as usize) as u64),
        fs::cache::frames(),
        c.dirty,
        c.limit,
        c.hits,
        c.misses,
        c.filled,
        c.written,
        c.evicted
    );
    Ok(())
}

fn ps(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    let threads = thread::list();
    out!(con, "{}{:>6}  {:<9} NAME{}", sgr::BOLD, "ID", "STATE", sgr::RESET);
    for t in &threads {
        let state = match t.state {
            State::Runnable => "runnable",
            State::Running => "running",
            State::Blocked => "blocked",
            State::Exited => "exited",
        };
        let name = if t.name.is_empty() { "-" } else { t.name };
        out!(con, "{:>6}  {state:<9} {name}", format!("{}", t.id));
    }
    out!(con, "{} threads (of {})", threads.len(), thread::MAX_THREADS);
    Ok(())
}

fn lspci(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    let devices = pci::devices();
    for d in devices {
        let h = &d.header;
        let mut line = format!(
            "{} {:04x}:{:04x} {:<17} {:02x}.{:02x}.{:02x}",
            h.bdf,
            h.vendor_id,
            h.device_id,
            h.class.name(),
            h.class.class,
            h.class.subclass,
            h.class.prog_if
        );
        for (i, bar) in d.bars.iter().enumerate() {
            let Some(bar) = bar else { continue };
            let kind = if bar.kind == BarKind::Io { "io" } else { "mem" };
            let _ = write!(line, "  BAR{i} {kind} {:#x} {}", bar.address, Size(bar.size));
        }
        out!(con, "{line}");
    }
    out!(con, "{} functions", devices.len());
    Ok(())
}

fn disks(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    let disks = block::disks();
    for d in &disks {
        let i = d.info();
        let s = d.stats();
        let mode = match d.waiting() {
            Waiting::Polled => String::from("polled"),
            Waiting::Interrupt(v) => format!("interrupt-driven (vector {v:#x})"),
        };
        let flags = match (i.read_only, i.write_cache) {
            (true, _) => ", read-only",
            (false, true) => ", write cache",
            (false, false) => "",
        };
        out!(
            con,
            "{}{}{}  {}, {} ({} blocks of {} bytes), {mode}{flags}",
            sgr::BOLD,
            d.name(),
            sgr::RESET,
            d.description(),
            Size(i.bytes()),
            i.blocks,
            i.block_size
        );
        let c = s.cache;
        out!(
            con,
            "  cache {} buffers, {} dirty; {} hits, {} misses, {} read ahead, {} written back, {} evicted",
            s.buffers,
            s.dirty,
            c.hits,
            c.misses,
            c.read_ahead,
            c.write_backs,
            c.evictions
        );
        let q = s.queue;
        out!(
            con,
            "  queue {} requests ({} to the device, {} merged, {} split); {} interrupts, {} sleeps",
            q.requests,
            q.device_requests,
            q.merged,
            q.split,
            s.interrupts,
            s.sleeps
        );
    }
    if disks.is_empty() {
        out!(con, "no disks");
    }
    Ok(())
}

/// `ls -l`'s mode column: the type, then `rwx` per class with the
/// set-user-ID, set-group-ID and sticky bits folded in (`s`/`S`, `t`/`T`).
fn mode_string(st: &fs::Stat) -> String {
    let kind = match st.kind() {
        Some(FileType::Directory) => 'd',
        Some(FileType::Symlink) => 'l',
        Some(FileType::CharDevice) => 'c',
        Some(FileType::BlockDevice) => 'b',
        Some(FileType::Fifo) => 'p',
        Some(FileType::Socket) => 's',
        Some(FileType::Regular) | None => '-',
    };
    let p = st.perm();
    let mut s = String::from(kind);
    for (shift, special, mark) in [(6, 0o4000, 's'), (3, 0o2000, 's'), (0, 0o1000, 't')] {
        let bits = (p >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(match (bits & 1 != 0, p & special != 0) {
            (true, true) => mark,
            (false, true) => mark.to_ascii_uppercase(),
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    s
}

/// One `ls` line: mode, size (or device number), name, link target.
fn ls_line(files: &fs::Files, path: &str, name: &str) -> String {
    let Ok(st) = files.lstat(path.as_bytes()) else { return format!("?????????? {:>9} {name}", "?") };
    let size = match st.kind() {
        Some(FileType::CharDevice | FileType::BlockDevice) => format!("{}, {}", st.rdev.major, st.rdev.minor),
        _ => format!("{}", st.size),
    };
    let mut line = format!("{} {size:>9} {name}", mode_string(&st));
    if st.kind() == Some(FileType::Symlink)
        && let Ok(target) = files.readlinkat(fs::AT_FDCWD, path.as_bytes())
    {
        let _ = write!(line, " -> {}", String::from_utf8_lossy(&target));
    }
    line
}

fn ls(con: &mut dyn Console, args: &[String]) -> CommandResult {
    let path = match args {
        [] => "/",
        [p] => p.as_str(),
        _ => return Err("takes at most one path".into()),
    };
    let files = fs::Files::new();
    let st = files.stat(path.as_bytes()).or_else(|_| files.lstat(path.as_bytes()));
    let st = st.map_err(|e| format!("{path}: {}", e.name()))?;
    if st.kind() != Some(FileType::Directory) {
        out!(con, "{}", ls_line(&files, path, path));
        return Ok(());
    }
    let mut entries = files.read_dir(path.as_bytes()).map_err(|e| format!("{path}: {}", e.name()))?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let dir = path.trim_end_matches('/');
    for (name, _, _) in &entries {
        let name = String::from_utf8_lossy(name);
        out!(con, "{}", ls_line(&files, &format!("{dir}/{name}"), &name));
    }
    out!(con, "{} entries", entries.len());
    Ok(())
}

/// The most `cat` prints of one file (`/dev/zero` never ends).
const CAT_LIMIT: usize = 64 * 1024;

fn cat(con: &mut dyn Console, args: &[String]) -> CommandResult {
    if args.is_empty() {
        return Err("needs a file".into());
    }
    let files = fs::Files::new();
    for path in args {
        let (data, more) =
            files.read_to_end(path.as_bytes(), CAT_LIMIT).map_err(|e| format!("{path}: {}", e.name()))?;
        let text = String::from_utf8_lossy(&data);
        let _ = con.write_str(&text);
        if !text.is_empty() && !text.ends_with('\n') {
            let _ = con.write_str("\n");
        }
        if more {
            out!(con, "{}(stopped after {} KiB){}", sgr::BOLD, CAT_LIMIT / 1024, sgr::RESET);
        }
    }
    Ok(())
}

fn mount(con: &mut dyn Console, args: &[String]) -> CommandResult {
    let read_only = match args {
        [] => false,
        [_, _] => false,
        [_, _, ro] if ro == "ro" => true,
        _ => return Err("takes no arguments, or <disk> <path> [ro]".into()),
    };
    if let [disk, path, ..] = args {
        fs::disk::mount(disk, path, read_only).map_err(|e| format!("{disk} on {path}: {e} ({})", e.errno().name()))?;
        return Ok(());
    }
    let vfs = fs::vfs();
    let ctx = vfs.root_context().map_err(|e| format!("/: {}", e.name()))?;
    for m in vfs.mounts() {
        let at = vfs
            .mount_path(&ctx, m.id)
            .map_or_else(|_| String::from("(hidden)"), |p| String::from_utf8_lossy(&p).into_owned());
        let mut opts = String::from(if m.flags.read_only { "ro" } else { "rw" });
        for (on, name) in [(m.flags.no_suid, "nosuid"), (m.flags.no_dev, "nodev"), (m.flags.no_exec, "noexec")] {
            if on {
                opts.push(',');
                opts.push_str(name);
            }
        }
        out!(con, "{} on {at} type {} ({opts})", fs::disk::source_of(m.id, m.fs_name), m.fs_name);
    }
    Ok(())
}

fn umount(_con: &mut dyn Console, args: &[String]) -> CommandResult {
    let [path] = args else { return Err("needs one path".into()) };
    fs::disk::unmount(path).map_err(|e| format!("{path}: {}", e.name()))
}

fn write(_con: &mut dyn Console, args: &[String]) -> CommandResult {
    let [path, words @ ..] = args else { return Err("needs a file".into()) };
    let mut text = words.join(" ");
    text.push('\n');
    let files = fs::Files::new();
    let flags = fs::OpenFlags::WRONLY | fs::OpenFlags::CREAT | fs::OpenFlags::TRUNC;
    let fd = files.open(path.as_bytes(), flags, 0o644).map_err(|e| format!("{path}: {}", e.name()))?;
    let written = files.write(fd, text.as_bytes());
    let _ = files.close(fd);
    match written {
        Ok(n) if n == text.len() => Ok(()),
        Ok(n) => Err(format!("{path}: wrote {n} of {} bytes", text.len())),
        Err(e) => Err(format!("{path}: {}", e.name())),
    }
}

fn sync(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    let before = fs::cache::cache().stats();
    fs::disk::sync_all().map_err(|e| format!("sync: {}", e.name()))?;
    let after = fs::cache::cache().stats();
    out!(con, "{} cached pages written back; every disk synced", after.written - before.written);
    Ok(())
}

fn uptime(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    let ns = time::now_ns();
    let ms = ns / time::NANOS_PER_MILLI;
    let (h, m, s) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60);
    out!(
        con,
        "up {h}:{m:02}:{s:02}.{:03} ({} ns), {} ticks at {} Hz, {} threads",
        ms % 1000,
        ns,
        time::ticks(),
        time::TICK_HZ,
        thread::list().len()
    );
    Ok(())
}

/// Who a device vector belongs to, as far as the shell can tell.
fn vector_owner(v: u8) -> String {
    if super::serial::vector() == Some(v) {
        return "serial (COM1)".into();
    }
    for d in block::disks() {
        if d.waiting() == Waiting::Interrupt(v) {
            return format!("disk {}", d.name());
        }
    }
    if irq::is_allocated(v) { "device".into() } else { "device (no handler now)".into() }
}

fn irqs(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    out!(con, "{}VECTOR  {:>12}  WHAT{}", sgr::BOLD, "COUNT", sgr::RESET);
    for v in irq::PIC_BASE..=u8::MAX {
        let n = irq::count(v);
        if n == 0 && !irq::is_allocated(v) {
            continue;
        }
        let what = match v {
            irq::TIMER_VECTOR => "timer (scheduler tick)".into(),
            irq::ERROR_VECTOR => "local APIC error".into(),
            irq::SPURIOUS_VECTOR => "local APIC spurious".into(),
            irq::PIC_BASE..irq::FIRST_DEVICE_VECTOR => "8259 (spurious)".into(),
            _ => vector_owner(v),
        };
        out!(con, "{v:#6x}  {n:>12}  {what}");
    }
    let s = irq::stats();
    out!(
        con,
        "spurious {}, without a handler {}, 8259 {}, APIC errors {}",
        s.spurious,
        s.unhandled,
        s.legacy,
        s.errors
    );
    Ok(())
}

fn cpu_info(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    let id = cpu::identify();
    let brand = if id.brand().is_empty() { "(no brand string)" } else { id.brand() };
    out!(con, "vendor    {}", id.vendor());
    out!(con, "model     {brand}");
    out!(con, "family    {} model {} stepping {}", id.family, id.model, id.stepping);
    if let Some(t) = arch::time::info() {
        match t.tsc_hz {
            Some(hz) => out!(con, "TSC       {}.{:03} MHz", hz / 1_000_000, hz / 1000 % 1000),
            None => out!(con, "TSC       not calibrated"),
        }
    }
    if let Some(apic) = arch::apic::interrupt_destination() {
        out!(con, "APIC ID   {apic}");
    }
    // Wrap the feature list at the console's width.
    let width = con.width().max(20);
    let mut line = String::from("features ");
    for f in id.features() {
        if line.len() + 1 + f.len() > width {
            out!(con, "{line}");
            line = String::from("         ");
        }
        line.push(' ');
        line.push_str(f);
    }
    out!(con, "{line}");
    Ok(())
}

fn test(con: &mut dyn Console, args: &[String]) -> CommandResult {
    let filter = match args {
        [] => None,
        [f] => Some(f.as_str()),
        _ => return Err("takes at most one filter".into()),
    };
    let ctx = TEST_CONTEXT.get().ok_or("the test environment is not set up")?;
    let env = TestEnv { boot: &ctx.boot, memory: ctx.memory };
    let start = time::now_ns();
    let summary = ktest::run(&env, filter, When::Runtime, con).map_err(|e| match e {
        RunError::Busy => String::from("tests are already running"),
    })?;
    let ms = (time::now_ns() - start) / time::NANOS_PER_MILLI;
    let color = if summary.failed == 0 { sgr::GREEN } else { sgr::RED };
    out!(con, "{color}{} passed, {} failed{} in {ms} ms", summary.passed, summary.failed, sgr::RESET);
    Ok(())
}

/// `run [NAME=value...] <program> [args...]`: start a program from the
/// file system and wait for it; its output goes to the console like the
/// kernel log's. Leading `NAME=value` words (a name without `/`) are its
/// environment, as with env(1): `run LD_DEBUG=libs /bin/prog`.
fn run_program(con: &mut dyn Console, args: &[String]) -> CommandResult {
    let is_variable = |a: &String| a.split_once('=').is_some_and(|(name, _)| !name.is_empty() && !name.contains('/'));
    let vars = args.iter().take_while(|a| is_variable(a)).count();
    let (envp, args) = args.split_at(vars);
    let [path, rest @ ..] = args else { return Err("needs a program".into()) };
    let status = crate::program::run_on(path, rest, envp, con.terminal()).map_err(|e| format!("{path}: {e}"))?;
    out!(con, "{path}: {status}");
    Ok(())
}

fn echo(con: &mut dyn Console, args: &[String]) -> CommandResult {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    out!(con, "{}", words.join(" "));
    Ok(())
}

fn clear(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    let _ = con.write_str("\x1b[2J\x1b[H");
    Ok(())
}

fn reboot(con: &mut dyn Console, args: &[String]) -> CommandResult {
    no_args(args)?;
    power::reboot(con)
}

// --- Tests ------------------------------------------------------------------

crate::kernel_test! {
    fn sizes_print_with_one_decimal() -> TestResult {
        let cases: [(u64, &str); 5] =
            [(512, "512 B"), (1024, "1.0 KiB"), (1536, "1.5 KiB"), (256 << 20, "256.0 MiB"), ((2 << 30) - 1, "2.0 GiB")];
        for (bytes, want) in cases {
            if human(bytes) != want {
                return Err("human() formats a size wrong");
            }
        }
        if COMMANDS.iter().enumerate().any(|(i, c)| COMMANDS[..i].iter().any(|d| d.name == c.name)) {
            return Err("two commands share a name");
        }
        Ok(())
    }
}

crate::kernel_test! {
    fn ls_cat_and_mount_show_the_file_systems() -> TestResult {
        use crate::fs::OpenFlags;
        let files = fs::Files::new();
        let fd = files
            .open(b"/tmp/shell-test.txt", OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::TRUNC, 0o640)
            .map_err(|e| e.name())?;
        files.write(fd, "line one\nzwei, gr\u{fc}n".as_bytes()).map_err(|e| e.name())?;
        files.close(fd).map_err(|e| e.name())?;
        files.symlinkat(b"shell-test.txt", fs::AT_FDCWD, b"/tmp/shell-link").map_err(|e| e.name())?;
        let out = super::session(
            "ls\rls /dev\rls /tmp\rcat /tmp/shell-link /tmp/shell-test.txt\rcat /nope\rmount\rls /dev/null\r",
        );
        let _ = files.unlinkat(fs::AT_FDCWD, b"/tmp/shell-link", 0);
        let _ = files.unlinkat(fs::AT_FDCWD, b"/tmp/shell-test.txt", 0);
        let check = |n: usize, what: &str, needles: &[&str]| {
            let o = super::output_of(&out, n);
            if needles.iter().all(|s| o.contains(s)) {
                Ok(())
            } else {
                crate::kprintln!("shell: {what} printed:\n{o}");
                Err("ls, cat or mount printed something unexpected")
            }
        };
        check(0, "ls", &["drwxr-xr-x", " dev\n", "drwxrwxrwt", " tmp\n", " proc\n", " entries\n"])?;
        check(1, "ls /dev", &["crw------- ", "5, 1 console\n", "crw-rw-rw- ", "1, 3 null\n", "1, 5 zero\n"])?;
        check(2, "ls /tmp", &["-rw-r----- ", " 20 shell-test.txt\n", "lrwxrwxrwx ", " shell-link -> shell-test.txt\n"])?;
        check(3, "cat", &["line one\nzwei, gr\u{fc}n\nline one\nzwei, gr\u{fc}n\n"])?;
        check(4, "cat /nope", &["cat: /nope: ENOENT"])?;
        check(5, "mount", &["tmpfs on / type tmpfs (rw)\n"])?;
        check(6, "ls /dev/null", &["crw-rw-rw- ", "1, 3 /dev/null\n"])?;
        mode_check()
    }
}

/// Mode strings for the special bits, as `ls -l` shows them.
fn mode_check() -> Result<(), &'static str> {
    let stat = |mode: u32| fs::Stat {
        dev: fs::DeviceId::default(),
        ino: 1,
        mode,
        nlink: 1,
        uid: 0,
        gid: 0,
        rdev: fs::DeviceId::default(),
        size: 0,
        blksize: 4096,
        blocks: 0,
        atime: Default::default(),
        mtime: Default::default(),
        ctime: Default::default(),
    };
    let cases =
        [(0o104755, "-rwsr-xr-x"), (0o102644, "-rw-r-Sr--"), (0o041777, "drwxrwxrwt"), (0o041776, "drwxrwxrwT")];
    if cases.iter().all(|&(mode, want)| mode_string(&stat(mode)) == want) { Ok(()) } else { Err("mode string") }
}
