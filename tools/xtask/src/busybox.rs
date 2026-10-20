//! BusyBox as init and as an interactive shell (roadmap Phase 4,
//! "BusyBox shell boots as init and runs interactively"; ADR 0051).
//!
//! BusyBox is GPL-2.0 and is never committed or copied into the
//! repository: it is a test program, taken at test time from one of
//!
//! 1. `POLY_BUSYBOX=<file>`: that statically linked binary;
//! 2. the host's statically linked BusyBox (`/bin/busybox` or
//!    `/usr/bin/busybox`, e.g. Debian's and Ubuntu's `busybox-static`,
//!    which the CI workflow installs);
//! 3. a build of the pinned release ([`VERSION`], its SHA-256 checked)
//!    cached under `target/busybox/`: downloaded with `curl`, configured
//!    with `make defconfig` plus a static build, built with the host's C
//!    compiler. `POLY_BUSYBOX=build` asks for this even when the host has
//!    one.
//!
//! Without any of them the boot test is skipped with the reason. Nothing
//! of BusyBox is read to learn how Linux behaves (CLEAN_ROOM.md): it is run
//! on Poly only, and what it needs is found from Poly's own logs.
//!
//! The image (`initramfs::busybox_image`) has `/bin/busybox`, a link per
//! applet and BusyBox's configuration files; init is `/sbin/init`
//! (`poly.init=/sbin/init`), which runs `/etc/init.d/rcS` and then a login
//! shell on the console. `cargo xtask run --busybox` boots into it.

use crate::console::strip_ansi;
use crate::symbols::Symbols;
use poly_elf::Elf;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The BusyBox release built when the host has none.
pub const VERSION: &str = "1.36.1";
/// Where it is downloaded from.
const URL: &str = "https://busybox.net/downloads/busybox-1.36.1.tar.bz2";
/// Its SHA-256, as published next to it (`.sha256`).
const SHA256: &str = "b8cc24c9574d809e7279c3be349795c5d5ceb6fdf19ca709f80cde50e47de314";

/// The kernel arguments that boot into BusyBox's init.
pub const INIT_ARGS: &str = "poly.init=/sbin/init";

/// A BusyBox binary and where it came from.
#[derive(Debug)]
pub struct Binary {
    pub elf: Vec<u8>,
    pub origin: String,
}

/// Whether `elf` is a statically linked x86-64 executable (no `PT_INTERP`).
fn is_static(elf: &[u8]) -> bool {
    Elf::parse(elf).is_ok_and(|e| e.interp().is_none())
}

/// The BusyBox to test with, or why there is none.
pub fn obtain() -> super::Result<std::result::Result<Binary, String>> {
    let wanted = std::env::var("POLY_BUSYBOX").ok().filter(|v| !v.is_empty());
    if let Some(path) = wanted.as_deref().filter(|&p| p != "build") {
        let elf = std::fs::read(path).map_err(|e| format!("POLY_BUSYBOX={path}: {e}"))?;
        if !is_static(&elf) {
            return Err(format!("POLY_BUSYBOX={path} is not a statically linked ELF executable"));
        }
        return Ok(Ok(Binary { elf, origin: format!("{path} (POLY_BUSYBOX)") }));
    }
    if wanted.is_none() {
        for path in ["/bin/busybox", "/usr/bin/busybox"] {
            if let Ok(elf) = std::fs::read(path)
                && is_static(&elf)
            {
                return Ok(Ok(Binary { elf, origin: format!("{path} (the host's, statically linked)") }));
            }
        }
    }
    build()
}

fn cache_dir() -> PathBuf {
    super::root().join("target/busybox")
}

/// Whether `program` can be started (it exists on `PATH`).
fn have(program: &str) -> bool {
    Command::new(program).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

fn run_step(what: &str, cmd: &mut Command, log: &Path) -> super::Result {
    let out = std::fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let status = cmd.stdout(out).stderr(err).status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what} failed ({status}); see {}", log.display())) }
}

/// The pinned release, built static and cached: `target/busybox/<version>/busybox`.
fn build() -> super::Result<std::result::Result<Binary, String>> {
    let dir = cache_dir();
    let binary = dir.join(format!("busybox-{VERSION}.bin"));
    if let Ok(elf) = std::fs::read(&binary)
        && is_static(&elf)
    {
        return Ok(Ok(Binary { elf, origin: format!("{} (built from BusyBox {VERSION})", binary.display()) }));
    }
    for tool in ["curl", "sha256sum", "tar", "bzip2", "make", "cc"] {
        if !have(tool) {
            return Ok(Err(format!(
                "no statically linked BusyBox on this host (install busybox-static or set POLY_BUSYBOX) \
                 and no `{tool}` to build BusyBox {VERSION}"
            )));
        }
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tarball = dir.join(format!("busybox-{VERSION}.tar.bz2"));
    if !tarball.exists() {
        eprintln!("==> downloading {URL}");
        let partial = tarball.with_extension("part");
        let fetched = Command::new("curl").args(["-fsSL", "--retry", "3", "-o"]).arg(&partial).arg(URL).status();
        if !fetched.is_ok_and(|s| s.success()) {
            let _ = std::fs::remove_file(&partial);
            return Ok(Err(format!("no statically linked BusyBox on this host and downloading {URL} failed")));
        }
        std::fs::rename(&partial, &tarball).map_err(|e| e.to_string())?;
    }
    let sum = Command::new("sha256sum").arg(&tarball).output().map_err(|e| format!("sha256sum: {e}"))?;
    let sum = String::from_utf8_lossy(&sum.stdout);
    if sum.split_whitespace().next() != Some(SHA256) {
        let _ = std::fs::remove_file(&tarball);
        return Err(format!("{} does not have the pinned SHA-256 {SHA256} (got {sum}); removed", tarball.display()));
    }
    // A fresh, empty directory for the sources each time.
    let src_root = dir.join("src");
    let _ = std::fs::remove_dir_all(&src_root);
    std::fs::create_dir_all(&src_root).map_err(|e| format!("{}: {e}", src_root.display()))?;
    let log = dir.join("build.log");
    eprintln!("==> building BusyBox {VERSION} (static) in {}; log in {}", src_root.display(), log.display());
    run_step("tar", Command::new("tar").arg("-xjf").arg(&tarball).arg("-C").arg(&src_root), &log)?;
    let src = src_root.join(format!("busybox-{VERSION}"));
    run_step("make defconfig", Command::new("make").current_dir(&src).arg("defconfig"), &log)?;
    // A static build. `tc` needs traffic-control definitions that newer
    // kernel headers dropped, so it is left out.
    let config = src.join(".config");
    let text = std::fs::read_to_string(&config).map_err(|e| format!("{}: {e}", config.display()))?;
    let text: String = text
        .lines()
        .map(|l| match l {
            "# CONFIG_STATIC is not set" => "CONFIG_STATIC=y",
            "CONFIG_TC=y" => "# CONFIG_TC is not set",
            l => l,
        })
        .flat_map(|l| [l, "\n"])
        .collect();
    std::fs::write(&config, text).map_err(|e| format!("{}: {e}", config.display()))?;
    run_step("make oldconfig", Command::new("make").current_dir(&src).arg("oldconfig").stdin(Stdio::null()), &log)?;
    let jobs = thread::available_parallelism().map_or(2, |n| n.get());
    run_step("make", Command::new("make").current_dir(&src).arg(format!("-j{jobs}")), &log)?;
    let elf = std::fs::read(src.join("busybox")).map_err(|e| format!("the built busybox: {e}"))?;
    if !is_static(&elf) {
        return Ok(Err(format!("BusyBox {VERSION} did not build statically linked (no static C library?)")));
    }
    std::fs::write(&binary, &elf).map_err(|e| format!("{}: {e}", binary.display()))?;
    let _ = std::fs::remove_dir_all(&src_root);
    Ok(Ok(Binary { elf, origin: format!("{} (built from BusyBox {VERSION})", binary.display()) }))
}

/// Write the BusyBox image to `target/poly-busybox-initramfs.cpio`.
pub fn write_image() -> super::Result<std::result::Result<(PathBuf, Binary), String>> {
    let binary = match obtain()? {
        Ok(b) => b,
        Err(why) => return Ok(Err(why)),
    };
    let image = crate::initramfs::busybox_image(&binary.elf);
    let path = crate::initramfs::write(&super::root(), "poly-busybox-initramfs.cpio", &image)?;
    Ok(Ok((path, binary)))
}

/// `cargo xtask busybox-initramfs`: write the image, print its path.
pub fn command() -> super::Result {
    match write_image()? {
        Ok((path, binary)) => {
            println!("{}", path.display());
            eprintln!("==> {} with BusyBox from {}", path.display(), binary.origin);
            Ok(())
        }
        Err(why) => Err(why),
    }
}

// ---- the boot test -------------------------------------------------------

/// How long one step of the session may take: generous, since the host
/// running the tests may be heavily loaded.
const STEP_TIMEOUT: Duration = Duration::from_secs(90);
/// How long the whole session may take.
const SESSION_TIMEOUT: Duration = Duration::from_secs(600);

/// A boot with the serial port on QEMU's standard input and output, typed
/// into like a terminal: [`Session::expect`] waits for text, [`Session::run`]
/// types a command line and returns what it printed before the next prompt.
struct Session {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    /// Everything QEMU wrote, raw.
    raw: Arc<Mutex<Vec<u8>>>,
    /// Where in the text (escape sequences removed) the last match ended.
    mark: usize,
    /// Lines of the text already echoed to the test's output.
    printed: usize,
    syms: Arc<Symbols>,
    started: Instant,
}

impl Session {
    fn start(kernel: &Path, syms: &Arc<Symbols>, append: &str, disk: &Path, image: &Path) -> super::Result<Session> {
        let mut child = super::qemu(kernel, super::DEFAULT_MEMORY, append, super::Devices::Test(disk), "stdio")
            .args(["-display", "none", "-initrd"])
            .arg(image)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("qemu-system-x86_64: {e} (is QEMU installed?)"))?;
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let raw = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&raw);
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n @ 1..) = stdout.read(&mut buf) {
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        Ok(Session { child, stdin, raw, mark: 0, printed: 0, syms: Arc::clone(syms), started: Instant::now() })
    }

    /// The output so far without escape sequences and carriage returns.
    fn text(&mut self) -> String {
        let raw = self.raw.lock().unwrap().clone();
        let upto = match std::str::from_utf8(&raw) {
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            _ => raw.len(),
        };
        let text = strip_ansi(&String::from_utf8_lossy(&raw[..upto]));
        for line in text.split_inclusive('\n').skip(self.printed).filter(|l| l.ends_with('\n')) {
            println!("    | {}", self.syms.symbolize_line(line.trim_end()));
            self.printed += 1;
        }
        text
    }

    /// Wait until `found` finds something in the text after the mark (it
    /// returns where its match ends); move the mark there and return the
    /// text up to it.
    fn wait(&mut self, what: &str, found: impl Fn(&str) -> Option<usize>) -> super::Result<String> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        loop {
            let text = self.text();
            if let Some(end) = found(&text[self.mark..]) {
                let got = text[self.mark..self.mark + end].to_string();
                self.mark += end;
                return Ok(got);
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(format!("QEMU exited ({status}) while waiting for {what}"));
            }
            if Instant::now() > deadline || self.started.elapsed() > SESSION_TIMEOUT {
                let last = super::ktest::last_line(&text[self.mark..]).to_string();
                return Err(format!("timed out waiting for {what} (last line: {last:?})"));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Wait for `needle`.
    fn expect(&mut self, needle: &str) -> super::Result<String> {
        self.wait(&format!("{needle:?}"), |t| t.find(needle).map(|i| i + needle.len()))
    }

    /// Wait for the shell's prompt (`root@<host>:<dir># ` at the end of
    /// the output so far); returns the text before it.
    fn prompt(&mut self) -> super::Result<String> {
        let got = self.wait("the shell's prompt", |t| {
            let start = t.rfind('\n').map_or(0, |i| i + 1);
            let last = &t[start..];
            (last.starts_with("root@") && last.ends_with("# ")).then_some(t.len())
        })?;
        let start = got.rfind('\n').map_or(0, |i| i + 1);
        Ok(got[..start].to_string())
    }

    fn send(&mut self, bytes: &str) -> super::Result {
        self.stdin.write_all(bytes.as_bytes()).and_then(|()| self.stdin.flush()).map_err(|e| format!("qemu stdin: {e}"))
    }

    /// Type `line` and Enter at the prompt and return what the command
    /// printed (the echoed line left out).
    fn run(&mut self, line: &str) -> super::Result<String> {
        self.send(&format!("{line}\r"))?;
        let out = self.prompt()?;
        // The shell echoes the line, then the command's output follows.
        Ok(out.split_once('\n').map_or(String::new(), |(_, rest)| rest.to_string()))
    }

    /// Wait for QEMU to exit; its status.
    fn finish(&mut self) -> super::Result<Option<i32>> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        loop {
            if let Some(s) = self.child.try_wait().map_err(|e| e.to_string())? {
                thread::sleep(Duration::from_millis(100));
                self.text();
                return Ok(s.code());
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn check(what: &str, ok: bool, output: &str) -> super::Result {
    if ok { Ok(()) } else { Err(format!("BusyBox on Poly: {what} failed; it printed {output:?}")) }
}

/// The boot test of the milestone: BusyBox's init as the first process on
/// the serial console, its shell typed into through QEMU's standard input.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> super::Result {
    eprintln!("==> boot test: BusyBox boots as init and its shell runs interactively on the serial console");
    let (image, binary) = match write_image()? {
        Ok(found) => found,
        Err(why) => {
            eprintln!("==> skipped: {why}");
            return Ok(());
        }
    };
    eprintln!("==> BusyBox from {}", binary.origin);
    let disk = super::test_disk()?;
    let mut s = Session::start(kernel, syms, &format!("{INIT_ARGS} console=ttyS0"), &disk, &image)?;
    let result = session(&mut s);
    if result.is_err() {
        let log = super::root().join("target/poly-busybox-boot.log");
        let _ = std::fs::write(&log, s.text());
        eprintln!("==> the session's output is in {}", log.display());
    }
    result
}

fn session(s: &mut Session) -> super::Result {
    s.expect("rcS: Linux 6.1.0-poly up, file systems mounted")?;
    s.prompt()?;
    // Long command lines on one line: the serial line has no window size
    // until one is set (the shell's line editor wraps at 80 columns).
    let out = s.run("stty cols 250 rows 50 && stty size")?;
    check("stty cols 250 rows 50", out == "50 250\n", &out)?;
    let out = s.run("echo hi")?;
    check("echo hi", out == "hi\n", &out)?;
    let out = s.run("ls /")?;
    let names: Vec<&str> = out.split_whitespace().collect();
    check("ls /", ["bin", "dev", "etc", "proc", "sbin", "sys", "tmp", "usr"].iter().all(|n| names.contains(n)), &out)?;
    let out = s.run("cat /proc/meminfo | head -1")?;
    check("cat /proc/meminfo | head -1", out.starts_with("MemTotal:") && out.ends_with(" kB\n"), &out)?;
    let out = s.run("seq 1 100 | grep 7 | wc -l")?;
    check("a pipeline", out.trim() == "19", &out)?;
    let out = s.run("cd /tmp && echo x > f && cat f && pwd")?;
    check("a file in /tmp", out == "x\n/tmp\n", &out)?;
    let out = s.run("sleep 0.2 && echo done")?;
    check("sleep 0.2", out == "done\n", &out)?;

    // Job control: a background job, listed and killed.
    s.run("sleep 30 &")?;
    let out = s.run("jobs")?;
    check("jobs lists the background job", out.contains("[1]+  Running") && out.contains("sleep 30"), &out)?;
    s.send("kill %1\r")?;
    let out = s.expect("Terminated")?;
    check("kill %1", out.contains("[1]+"), &out)?;
    s.prompt()?;
    let out = s.run("jobs")?;
    check("no jobs left", out.is_empty(), &out)?;

    // Ctrl-C interrupts the foreground job (exit status 128 + SIGINT).
    s.send("echo started; sleep 10\r")?;
    s.expect("started\n")?;
    thread::sleep(Duration::from_millis(300));
    s.send("\x03")?;
    s.prompt()?;
    let out = s.run("echo $?")?;
    check("Ctrl-C interrupts sleep 10", out == "130\n", &out)?;

    // Ctrl-Z stops a job, bg continues it in the background, fg brings it
    // back, Ctrl-C ends it.
    s.send("(echo started; sleep 20)\r")?;
    s.expect("started\n")?;
    thread::sleep(Duration::from_millis(300));
    s.send("\x1a")?;
    let out = s.expect("Stopped")?;
    check("Ctrl-Z stops the job", out.contains("[1]+"), &out)?;
    s.prompt()?;
    let out = s.run("bg")?;
    check("bg", out.contains("[1] (echo started; sleep 20)"), &out)?;
    let out = s.run("jobs")?;
    check("the job runs in the background", out.contains("Running"), &out)?;
    s.send("fg\r")?;
    s.expect("(echo started; sleep 20)\n")?;
    thread::sleep(Duration::from_millis(300));
    s.send("\x03")?;
    s.prompt()?;
    let out = s.run("jobs")?;
    check("fg and Ctrl-C end the job", out.is_empty(), &out)?;

    let out = s.run("ps")?;
    check(
        "ps",
        out.lines().any(|l| l.trim_start().starts_with("1 root") && l.ends_with("init")) && out.contains("/bin/sh -l"),
        &out,
    )?;
    let out = s.run("mount")?;
    check(
        "mount lists the file systems",
        out.contains("proc on /proc type proc (rw,nosuid,nodev,noexec)")
            && out.contains("tmpfs on /tmp type tmpfs (rw,nosuid,nodev)")
            && out.contains("devpts on /dev/pts type devpts"),
        &out,
    )?;
    // mount(2) and umount2(2): a tmpfs, and the ext2 test disk (/dev/vdc).
    let out = s.run("mkdir -p /m && mount -t tmpfs -o size=1m,mode=700 scratch /m && stat -c '%a' /m && grep ' /m ' /proc/mounts && umount /m && echo unmounted")?;
    check("mount -t tmpfs", out == "700\nscratch /m tmpfs rw 0 0\nunmounted\n", &out)?;
    let out = s.run("mount -t ext2 /dev/vdc /mnt && cat /mnt/hello.txt && umount /mnt && echo unmounted")?;
    // The kernel logs the disk coming up and the (un)mount in between.
    let hello = String::from_utf8_lossy(super::ext2disk::HELLO);
    check("mount -t ext2 /dev/vdc", out.lines().any(|l| l == hello.trim_end()) && out.ends_with("unmounted\n"), &out)?;

    let out = s.run("hostname poly-test && uname -a && hostname poly")?;
    check("hostname and uname -a", out.starts_with("Linux poly-test 6.1.0-poly #1 Poly x86_64"), &out)?;
    let out = s.run("id")?;
    check("id", out.starts_with("uid=0(root) gid=0(root)"), &out)?;

    // The shell exits; init starts another (a new process).
    let before = s.run("echo $$")?;
    s.send("exit\r")?;
    s.expect("exit\n")?;
    s.prompt()?;
    let after = s.run("echo $$")?;
    check("a new shell after exit", after.trim().parse::<u32>().is_ok() && after != before, &after)?;

    // poweroff: init ends everything and powers off (ACPI S5), which ends
    // QEMU with status 0.
    s.send("poweroff\r")?;
    s.expect("reboot: Power down.")?;
    let status = s.finish()?;
    check("poweroff ends QEMU cleanly", status == Some(0), &format!("QEMU status {status:?}"))
}
