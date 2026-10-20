//! `poly.strace`: one line on the serial port per system call of every
//! Linux process, for debugging programs on Poly (docs/DEBUGGING.md).
//! Unlike `poly.test.capture` (the test-run protocol, ADR 0027) it changes
//! nothing about the processes: they keep their terminals.
//!
//! ```text
//! strace: pid 7 openat(0xffffff9c, "/etc/inittab", 0x80000, 0x0, ..) = 3
//! strace: pid 7 ioctl(0x0, 0x5401, 0x7fffffffe9c0, ..) = -25 ENOTTY
//! ```
//!
//! The arguments are the six registers, in hex; those that the call takes
//! as a path or name are shown as strings, read before the call runs.
//! `poly.strace=<name>` limits the lines to processes whose `comm` (the
//! file name of the program, as `/proc/<pid>/comm` shows it) is `<name>`.

use crate::process::Process;
use crate::sync::SpinLock;
use crate::{log, user};
use alloc::string::String;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_personality_linux::errno::Errno;

/// Kernel argument: trace system calls (`poly.strace` or `poly.strace=<comm>`).
pub const STRACE_ARG: &str = "poly.strace";

static ON: AtomicBool = AtomicBool::new(false);
/// The program name to trace, empty for every process.
static ONLY: SpinLock<String> = SpinLock::new(String::new());

/// Most bytes of a string argument shown.
const STRING_MAX: usize = 96;

/// Whether `poly.strace` is on: then the kernel also logs every stop,
/// continue and kill of a Linux process, which it otherwise logs only for
/// signals whose default action is a core dump.
pub fn verbose() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Turn tracing on if the command line asks for it.
pub fn configure(boot: &crate::boot_info::BootInfo) {
    let name = match boot.arg(STRACE_ARG) {
        Some(name) => name,
        None if boot.has_flag(STRACE_ARG) => "",
        None => return,
    };
    *ONLY.lock() = String::from(name);
    ON.store(true, Ordering::Release);
    crate::kprintln!(
        "strace: tracing the system calls of {}",
        if name.is_empty() { "every Linux process" } else { name }
    );
}

/// The start of a trace line for call `number` with `args` of `process`'s
/// running thread, or `None` if it is not traced. Called before the call
/// runs, so its string arguments are read as the caller passed them.
pub fn enter(process: &Process, number: u64, args: &[u64; 6]) -> Option<String> {
    if !ON.load(Ordering::Acquire) {
        return None;
    }
    {
        let only = ONLY.lock();
        if !only.is_empty() && crate::fs::proc::comm(process.pid).as_deref() != Some(only.as_bytes()) {
            return None;
        }
    }
    let mut line = String::with_capacity(128);
    let name = poly_personality_linux::table::name(number);
    let _ = write!(line, "strace: pid {} ", process.pid);
    match name {
        Some(n) => line.push_str(n),
        None => {
            let _ = write!(line, "syscall_{number}");
        }
    }
    line.push('(');
    let strings = name.map_or(0, string_args);
    for (i, &a) in args.iter().enumerate().take(4) {
        if i > 0 {
            line.push_str(", ");
        }
        if strings & (1 << i) != 0 {
            push_string(&mut line, a);
        } else {
            let _ = write!(line, "{a:#x}");
        }
    }
    line.push_str(", ..)");
    Some(line)
}

/// Finish and write `line` with the call's result (`None`: it does not return).
pub fn exit(mut line: String, ret: Option<u64>) {
    match ret {
        Some(r) => match Errno::from_rax(r) {
            Some(e) => {
                let _ = writeln!(line, " = -{} {}", e.get(), e.name().unwrap_or("?"));
            }
            None if r > 0xffff_ffff => {
                let _ = writeln!(line, " = {r:#x}");
            }
            None => {
                let _ = writeln!(line, " = {r}");
            }
        },
        None => line.push_str(" = ?\n"),
    }
    log::write_console(&line);
}

/// Which of the first four arguments of call `name` are strings (bits).
fn string_args(name: &str) -> u8 {
    match name {
        "open" | "stat" | "lstat" | "access" | "execve" | "truncate" | "chdir" | "mkdir" | "rmdir" | "creat"
        | "unlink" | "readlink" | "chmod" | "chown" | "lchown" | "mknod" | "statfs" | "umount2" | "chroot" | "acct"
        | "swapon" | "swapoff" | "utimes" | "utime" => 0b1,
        "openat" | "newfstatat" | "faccessat" | "faccessat2" | "readlinkat" | "mkdirat" | "unlinkat" | "fchmodat"
        | "fchownat" | "mknodat" | "utimensat" | "execveat" | "statx" | "futimesat" => 0b10,
        "rename" | "link" | "symlink" | "pivot_root" => 0b11,
        "symlinkat" => 0b101,
        "renameat" | "renameat2" | "linkat" => 0b1010,
        "mount" => 0b111,
        _ => 0,
    }
}

/// The string at user address `addr`, quoted and cut at [`STRING_MAX`].
fn push_string(line: &mut String, addr: u64) {
    if addr == 0 {
        line.push_str("NULL");
        return;
    }
    let mut buf = [0u8; STRING_MAX];
    let mut len = 0;
    let mut ended = false;
    while len < STRING_MAX {
        // Never across a page boundary at once: the next page may be unmapped.
        let at = addr.wrapping_add(len as u64);
        let room = (4096 - (at % 4096)) as usize;
        let n = room.min(STRING_MAX - len);
        if user::copy_from_user(&mut buf[len..len + n], at).is_err() {
            break;
        }
        if let Some(nul) = buf[len..len + n].iter().position(|&b| b == 0) {
            len += nul;
            ended = true;
            break;
        }
        len += n;
    }
    if len == 0 && !ended {
        let _ = write!(line, "{addr:#x}");
        return;
    }
    line.push('"');
    for &b in &buf[..len] {
        match b {
            b'"' | b'\\' => {
                line.push('\\');
                line.push(b as char);
            }
            0x20..=0x7e => line.push(b as char),
            b'\n' => line.push_str("\\n"),
            _ => {
                let _ = write!(line, "\\x{b:02x}");
            }
        }
    }
    line.push('"');
    if !ended {
        line.push_str("...");
    }
}
