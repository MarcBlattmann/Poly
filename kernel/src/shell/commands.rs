//! The shell's commands. Each one reads kernel state through the modules'
//! public APIs and prints it; none changes anything except `test` (which
//! runs the kernel tests), `clear` and `reboot`.

use super::{Console, TEST_CONTEXT, sgr};
use crate::arch::{self, cpu, irq};
use crate::block::{self, Waiting};
use crate::frames::FRAMES;
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
    Command { name: "uptime", usage: "", help: "time since boot and timer ticks", run: uptime },
    Command { name: "irq", usage: "", help: "interrupt counts per vector", run: irqs },
    Command { name: "cpu", usage: "", help: "processor vendor, model and features (CPUID)", run: cpu_info },
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
