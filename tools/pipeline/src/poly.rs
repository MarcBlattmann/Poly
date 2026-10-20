//! The Poly side of the diff runner: boot the kernel in QEMU with an
//! initramfs that holds the test program, and turn its serial log into a
//! trace.
//!
//! The kernel interface is specified in ADR 0027 (the "test-run protocol").
//! Kernel command line: `poly.qemu_exit poly.init=/test-binary poly.test.capture`.
//! With `poly.test.capture` the kernel writes, each on a line of its own:
//!
//! ```text
//! POLY-INIT: exec /test-binary              it is starting the program
//! POLY-INIT: unsupported <reason>           it knows the flag but cannot run it
//! POLY-SYSCALL: pid=1 nr=1 args=0x1,0x401000,0xd,0x0,0x0,0x0 ret=13 t=81234 data1=68656c6c6f
//! POLY-OUT: fd=1 hex=68656c6c6f0a           bytes the program wrote to fd 1 or 2
//! POLY-SIGNAL: pid=1 signal=11              a signal delivered to the program
//! POLY-EXIT: pid=1 code=0 t=91234           the program ended (or signal=N)
//! ```
//!
//! `ret=?` (or no `ret`) means the call did not return; `t` is nanoseconds
//! since the program started and is optional; `data<i>=<hex>` optionally
//! gives the bytes behind pointer argument `i` (the string or buffer the
//! call read, or for `read` what it returned). Without data, those
//! arguments match anything in a comparison. After `POLY-EXIT` the kernel
//! ends QEMU with its success status; the program's own status is the one
//! in the `POLY-EXIT` line.
//!
//! A kernel that does not know `poly.init` boots as usual and never prints
//! `POLY-INIT`: the test is then reported as unsupported.

use crate::cpio;
use crate::runner;
use crate::syscalls;
use crate::trace::{self, Arg, Event, Exit, Memory, Syscall, Trace};
use crate::util::{self, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Where the test program sits in the initramfs.
pub const INIT_PATH: &str = "/test-binary";
/// Kernel arguments for a test run.
pub const KERNEL_ARGS: &str = "poly.qemu_exit poly.init=/test-binary poly.test.capture";
/// QEMU's exit status when the kernel reports success (`qemu::ExitCode::Success`).
pub const QEMU_SUCCESS: i32 = 33;

/// What a boot produced.
#[derive(Debug)]
pub enum Outcome {
    /// The kernel ran the program; here is its trace.
    Ran(Trace),
    /// The kernel cannot run programs (yet); the reason.
    Unsupported(String),
}

/// Bytes behind pointer arguments, from `data<i>` fields.
struct Data(BTreeMap<u64, Vec<u8>>);

impl Memory for Data {
    fn read(&self, addr: u64, len: usize) -> Vec<u8> {
        self.0.get(&addr).map(|b| b[..len.min(b.len())].to_vec()).unwrap_or_default()
    }
}

fn hex_bytes(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(format!("odd-length hex {s:?}"));
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("{s:?}: {e}"))).collect()
}

fn number(s: &str) -> Result<u64> {
    let r = match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16),
        None => s.parse::<i64>().map(|v| v as u64),
    };
    r.map_err(|e| format!("bad number {s:?}: {e}"))
}

/// `key=value` fields of a protocol line.
fn fields(rest: &str) -> BTreeMap<&str, &str> {
    rest.split_whitespace().filter_map(|w| w.split_once('=')).collect()
}

fn field<'a>(f: &BTreeMap<&str, &'a str>, key: &str, line: &str) -> Result<&'a str> {
    f.get(key).copied().ok_or_else(|| format!("missing {key}= in {line:?}"))
}

/// Parse a serial log. `Ok(None)` if the kernel never acknowledged
/// `poly.init` (it does not support running programs).
pub fn parse_serial(log: &str, test: &str) -> Result<Option<Outcome>> {
    let mut t = Trace::new(test, "poly", "poly-serial");
    let mut pids: BTreeMap<u64, u32> = BTreeMap::new();
    let mut index = |pid: u64| -> u32 {
        let next = pids.len() as u32;
        *pids.entry(pid).or_insert(next)
    };
    let (mut acknowledged, mut stdout, mut stderr) = (false, Vec::new(), Vec::new());
    for line in log.lines() {
        // Lines may carry a carriage return, or kernel log text before the
        // marker if the console interleaved them; the marker starts the record.
        let line = line.trim_end_matches('\r');
        let Some(at) = line.find("POLY-") else { continue };
        let record = &line[at..];
        let Some((tag, rest)) = record.split_once(": ") else { continue };
        match tag {
            "POLY-INIT" => {
                if let Some(reason) = rest.strip_prefix("unsupported") {
                    return Ok(Some(Outcome::Unsupported(format!("kernel: {}", reason.trim()))));
                }
                acknowledged = true;
            }
            _ if !acknowledged => {}
            "POLY-SYSCALL" => {
                let f = fields(rest);
                let pid = index(number(field(&f, "pid", line)?)?);
                let nr = number(field(&f, "nr", line)?)?;
                let mut raw = [0u64; 6];
                let list = field(&f, "args", line)?;
                for (slot, a) in raw.iter_mut().zip(list.split(',').filter(|a| !a.is_empty())) {
                    *slot = number(a)?;
                }
                let mut data = BTreeMap::new();
                for (i, &addr) in raw.iter().enumerate() {
                    if let Some(hex) = f.get(format!("data{i}").as_str()) {
                        // Strings come without their NUL; buffers are read
                        // by length, so the extra byte does not matter.
                        let mut bytes = hex_bytes(hex)?;
                        bytes.push(0);
                        data.insert(addr, bytes);
                    }
                }
                let ret = match f.get("ret") {
                    None | Some(&"?") => None,
                    Some(r) => Some(number(r)? as i64),
                };
                let name = syscalls::name(nr);
                let mem = Data(data);
                let mut args = trace::decode_entry(&name, &raw, &mem);
                if let Some(r) = ret {
                    trace::decode_exit(&name, &raw, r, &mut args, &mem);
                }
                // Pointers whose contents the kernel did not send match anything.
                for a in &mut args {
                    if let Arg::Unreadable(p) = *a
                        && !mem.0.contains_key(&p)
                    {
                        *a = Arg::Opaque(p);
                    }
                }
                let t_ns = f.get("t").map(|v| number(v)).transpose()?.unwrap_or(0);
                t.events.push(Event::Syscall(Syscall { pid, nr, name, args, ret, t_ns }));
            }
            "POLY-OUT" => {
                let f = fields(rest);
                let bytes = hex_bytes(field(&f, "hex", line)?)?;
                match field(&f, "fd", line)? {
                    "1" => stdout.extend_from_slice(&bytes),
                    "2" => stderr.extend_from_slice(&bytes),
                    fd => return Err(format!("output on fd {fd} in {line:?}")),
                }
            }
            "POLY-SIGNAL" => {
                let f = fields(rest);
                let pid = index(number(field(&f, "pid", line)?)?);
                let signal = number(field(&f, "signal", line)?)? as i32;
                t.events.push(Event::Signal { pid, signal });
            }
            "POLY-EXIT" => {
                let f = fields(rest);
                if index(number(field(&f, "pid", line)?)?) != 0 {
                    continue;
                }
                t.exit = match (f.get("code"), f.get("signal")) {
                    (Some(c), _) => Exit::Code(number(c)? as i32),
                    (None, Some(s)) => Exit::Signal(number(s)? as i32),
                    _ => return Err(format!("POLY-EXIT without code= or signal=: {line:?}")),
                };
                t.duration_ns = f.get("t").map(|v| number(v)).transpose()?.unwrap_or(0);
            }
            _ => {}
        }
    }
    if !acknowledged {
        return Ok(None);
    }
    t.stdout = String::from_utf8_lossy(&stdout).into_owned();
    t.stderr = String::from_utf8_lossy(&stderr).into_owned();
    Ok(Some(Outcome::Ran(t)))
}

/// The initramfs for one test: `/test-binary` plus the directories a
/// program expects to exist.
pub fn initramfs(binary: &[u8]) -> Result<Vec<u8>> {
    cpio::write(&[
        cpio::Entry::dir("dev"),
        cpio::Entry::dir("tmp"),
        cpio::Entry::file(INIT_PATH.trim_start_matches('/'), 0o755, binary.to_vec()),
    ])
}

/// The default kernel ELF (`cargo xtask build`).
pub fn default_kernel(root: &Path) -> PathBuf {
    root.join("kernel/target/x86_64-unknown-none/debug/poly-kernel")
}

/// Build the kernel (debug) the way `cargo xtask build` does.
pub fn build_kernel(root: &Path) -> Result {
    eprintln!("==> build kernel");
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root.join("kernel"))
        .arg("build")
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("kernel build failed ({status})")) }
}

/// How often a boot is retried when QEMU itself crashes. QEMU 8.2 under
/// TCG on a loaded host occasionally dies with SIGSEGV during boot; a
/// guest cannot make the emulator take a signal (a triple fault or a
/// kernel panic ends QEMU with an exit status), and a timeout is ours
/// ([`Exit::Timeout`]), so a signal is the host's problem, not the test's.
pub const QEMU_CRASH_RETRIES: usize = 2;

/// QEMU ended by a signal it was not sent by us.
pub fn qemu_crashed(exit: &Exit) -> bool {
    matches!(exit, Exit::Signal(_))
}

#[derive(Debug)]
pub struct Target {
    pub kernel: PathBuf,
    pub timeout: Duration,
}

impl Target {
    /// Boot `binary` as `/test-binary`. Files go into `work`: the
    /// initramfs and the serial log.
    pub fn run(&self, test: &str, binary: &Path, work: &Path) -> Result<(Outcome, PathBuf)> {
        let bytes = util::read(binary)?;
        self.run_image(test, &initramfs(&bytes)?, &crate::sha256::hex(&bytes), work)
    }

    /// Boot the initramfs `image`, whose `/test-binary` has the hash
    /// `binary_sha256` (an app test's tree, `apps::Tree::cpio`).
    pub fn run_image(&self, test: &str, image: &[u8], binary_sha256: &str, work: &Path) -> Result<(Outcome, PathBuf)> {
        let image_path = work.join(format!("{test}.initramfs.cpio"));
        util::write(&image_path, image)?;
        let image = image_path;
        let log_path = work.join(format!("{test}.serial.log"));
        let mut attempt = 0;
        let (done, log) = loop {
            let mut cmd = Command::new("qemu-system-x86_64");
            cmd.args(["-machine", "q35", "-m", "256M", "-smp", "2", "-display", "none", "-serial", "stdio"])
                .args(["-no-reboot", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
                .arg("-kernel")
                .arg(&self.kernel)
                .arg("-initrd")
                .arg(&image)
                .args(["-append", KERNEL_ARGS]);
            let done = runner::run_command(&mut cmd, self.timeout)
                .map_err(|e| format!("{e} (is qemu-system-x86_64 installed?)"))?;
            let log = String::from_utf8_lossy(&done.stdout).into_owned();
            util::write(&log_path, &log)?;
            if !qemu_crashed(&done.exit) {
                break (done, log);
            }
            // The emulator itself died: that says nothing about the kernel
            // or the test. Keep the log, boot again.
            attempt += 1;
            util::write(&work.join(format!("{test}.serial.crash-{attempt}.log")), &log)?;
            if attempt > QEMU_CRASH_RETRIES {
                return Err(format!(
                    "QEMU crashed ({}) on {} boots in a row; serial log {}",
                    done.exit,
                    attempt,
                    log_path.display()
                ));
            }
        };
        let outcome = match parse_serial(&log, test)? {
            Some(Outcome::Ran(mut t)) => {
                t.binary_sha256 = Some(binary_sha256.to_string());
                if done.exit == Exit::Timeout {
                    t.exit = Exit::Timeout;
                } else if t.exit == Exit::Unknown {
                    return Err(format!("the kernel ran {test} but did not report its exit (QEMU: {})", done.exit));
                } else if done.exit != Exit::Code(QEMU_SUCCESS) {
                    return Err(format!("the kernel failed after {test} ended (QEMU: {})", done.exit));
                }
                Outcome::Ran(t)
            }
            Some(u) => u,
            None if done.exit == Exit::Timeout => {
                Outcome::Unsupported(format!("no \"POLY-INIT:\" line within {:?}; boot timed out", self.timeout))
            }
            None => Outcome::Unsupported(format!(
                "no \"POLY-INIT:\" line (QEMU {}): the kernel ignores poly.init and cannot run \
                 programs yet",
                done.exit
            )),
        };
        Ok((outcome, log_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{NormOptions, normalize};

    const LOG: &str = "boot: cmdline \"poly.qemu_exit poly.init=/test-binary poly.test.capture\"\n\
        POLY-INIT: exec /test-binary\r\n\
        POLY-SYSCALL: pid=7 nr=12 args=0x0 ret=0x400000 t=10\n\
        kernel log line\n\
        POLY-SYSCALL: pid=7 nr=1 args=0x1,0x401000,0xd ret=13 data1=68656c6c6f2c20776f726c640a\n\
        POLY-OUT: fd=1 hex=68656c6c6f2c20776f726c640a\n\
        POLY-SYSCALL: pid=7 nr=1 args=0x2,0x401000,0x3 ret=-14\n\
        POLY-OUT: fd=2 hex=6f6b\n\
        POLY-SIGNAL: pid=7 signal=11\n\
        POLY-SYSCALL: pid=7 nr=231 args=0x0 ret=?\n\
        POLY-EXIT: pid=7 code=0 t=99\n";

    #[test]
    fn parses_the_protocol() {
        let Some(Outcome::Ran(t)) = parse_serial(LOG, "hello").unwrap() else { panic!("not run") };
        assert_eq!(t.exit, Exit::Code(0));
        assert_eq!(t.stdout, "hello, world\n");
        assert_eq!(t.stderr, "ok");
        assert_eq!(t.duration_ns, 99);
        let lines: Vec<String> = normalize(&t, &NormOptions::default()).iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "brk(NULL) = <addr>",
                "write(1, \"hello, world\\n\", 13) = 13",
                "write(2, *, 3) = -1 EFAULT",
                "--- SIGSEGV ---",
                "exit_group(0) = ?",
            ]
        );
    }

    #[test]
    fn a_kernel_without_poly_init_is_unsupported() {
        assert!(parse_serial("boot: cmdline \"...\"\nPOLY-SELFTEST: PASS\n", "x").unwrap().is_none());
        let Some(Outcome::Unsupported(r)) = parse_serial("POLY-INIT: unsupported no ELF loader\n", "x").unwrap() else {
            panic!()
        };
        assert_eq!(r, "kernel: no ELF loader");
    }

    /// Only a signal is a crash of the emulator; our timeout and the
    /// kernel's own exit statuses are results.
    #[test]
    fn only_a_signal_is_a_qemu_crash() {
        assert!(qemu_crashed(&Exit::Signal(11)));
        assert!(!qemu_crashed(&Exit::Timeout));
        assert!(!qemu_crashed(&Exit::Code(QEMU_SUCCESS)));
        assert!(!qemu_crashed(&Exit::Code(0)));
    }

    #[test]
    fn malformed_lines_are_errors() {
        assert!(parse_serial("POLY-INIT: exec /t\nPOLY-SYSCALL: pid=1 args=0x1\n", "x").is_err());
        assert!(parse_serial("POLY-INIT: exec /t\nPOLY-OUT: fd=1 hex=abc\n", "x").is_err());
        assert!(parse_serial("POLY-INIT: exec /t\nPOLY-EXIT: pid=1\n", "x").is_err());
    }

    #[test]
    fn initramfs_holds_the_binary() {
        let entries = cpio::read(&initramfs(b"\x7fELF").unwrap()).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["dev", "tmp", "test-binary"]);
        assert_eq!(entries[2].mode, cpio::S_IFREG | 0o755);
    }
}
