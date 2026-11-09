//! The boot test: a session typed into the disk's bash.

use super::*;

// ---- the boot test -------------------------------------------------------

/// How long one step of the session may take: a C compile or Python's
/// start-up is slow under emulation, and the host may be loaded.
const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(400);
/// How long the whole session may take.
const SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2400);

/// The boot test of the milestone (see the module documentation): bash,
/// coreutils, Python and gcc from the root disk, typed into the disk's
/// bash on the serial console; then `e2fsck -fn` on the image and the
/// compiled program read back from it on the host.
pub fn boot_test(kernel: &Path, syms: &std::sync::Arc<crate::symbols::Symbols>) -> crate::Result {
    eprintln!("==> boot test: bash, coreutils, Python and gcc run from the persistent ext4 root disk");
    let (initrd, origin) = match write_initramfs()? {
        Ok(found) => found,
        Err(why) => {
            eprintln!("==> skipped: the root disk boots through BusyBox's init: {why}");
            return Ok(());
        }
    };
    let (image, contents) = make(TEST_FILE_NAME)?;
    if !contents.have.contains("bash") {
        eprintln!("==> skipped: the root disk boot test: the host has no bash");
        return Ok(());
    }
    eprintln!("==> BusyBox from {origin}");
    let mut s = spawn(kernel, syms, &image, &initrd)?;
    let mut ran = BTreeSet::new();
    let result = session(&mut s, &contents.have, &mut ran);
    if result.is_err() {
        let log = crate::root().join("target/poly-rootdisk-boot.log");
        let _ = std::fs::write(&log, s.text());
        eprintln!("==> the session's output is in {}", log.display());
        return result;
    }
    drop(s);
    match crate::ext2disk::e2fsck(&image)? {
        Some(_) => eprintln!("==> e2fsck -fn passes on the root disk after the session"),
        None => eprintln!("==> e2fsck not installed; skipped checking the image"),
    }
    if ran.contains(GCC_STEP) {
        let elf = read_back(&image, "root/hello")?;
        // The format string is in the file whatever the optimiser did with
        // the copied text.
        if !elf.starts_with(b"\x7fELF") || !elf.windows(10).any(|w| w == b"%s, argc=%") {
            return Err("the program gcc wrote is not on the root disk after the boot".into());
        }
        eprintln!("==> /root/hello, compiled on Poly, is on the image ({} bytes)", elf.len());
    }
    // The second boot from the same image: what the first one wrote is
    // there, and the program it compiled runs.
    if !ran.contains(PERSIST_STEP) {
        return Ok(());
    }
    eprintln!("==> boot test: a second boot from the root disk finds what the first one wrote");
    let mut s = spawn(kernel, syms, &image, &initrd)?;
    let result = second_boot(&mut s, ran.contains(GCC_STEP));
    if result.is_err() {
        let log = crate::root().join("target/poly-rootdisk-boot2.log");
        let _ = std::fs::write(&log, s.text());
        eprintln!("==> the session's output is in {}", log.display());
    }
    result
}

/// QEMU booting the root disk `image` with the initramfs `initrd`, as a
/// session typed into.
fn spawn(
    kernel: &Path,
    syms: &std::sync::Arc<crate::symbols::Symbols>,
    image: &Path,
    initrd: &Path,
) -> crate::Result<crate::busybox::Session> {
    // POLY_ROOTDISK_APPEND: more kernel arguments (`poly.strace=python3.13`, ...).
    let append = format!("{} {}", boot_args(), std::env::var("POLY_ROOTDISK_APPEND").unwrap_or_default());
    let mut cmd = crate::qemu(kernel, MEMORY, append.trim_end(), crate::Devices::None, "stdio");
    cmd.args(["-display", "none"]).args(drive_args(image)).arg("-initrd").arg(initrd);
    let mut s = crate::busybox::Session::spawn(cmd, syms)?;
    s.set_timeouts(STEP_TIMEOUT, SESSION_TIMEOUT);
    Ok(s)
}

/// The step that writes the file the second boot reads.
const PERSIST_STEP: &str = "a file for the next boot, sync";

/// The file the first boot writes and the second reads.
const PERSIST_TEXT: &str = "written in the first boot";

fn second_boot(s: &mut crate::busybox::Session, compiled: bool) -> crate::Result {
    s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    s.prompt()?;
    let out = s.run("cat /root/persist.txt")?;
    if out != format!("{PERSIST_TEXT}\n") {
        return Err(format!("root disk on Poly: the second boot reads /root/persist.txt as {out:?}"));
    }
    if compiled {
        let out = s.run("/root/hello again; echo \"status=$?\"")?;
        if out != "hello from gcc on Poly, argc=2\nstatus=42\n" {
            return Err(format!("root disk on Poly: the program compiled in the first boot printed {out:?}"));
        }
    }
    s.send("poweroff\r")?;
    s.expect("reboot: Power down.")?;
    match s.finish()? {
        Some(0) => Ok(()),
        status => Err(format!("root disk on Poly: the second boot's poweroff ended QEMU with {status:?}")),
    }
}

/// The file at `path` on the ext4 image, read with `poly-ext2` (from a
/// copy in memory; the image is not changed).
pub(crate) fn read_back(image: &Path, path: &str) -> crate::Result<Vec<u8>> {
    use poly_ext2::{DeviceIo, Ext2, MountOptions};
    use poly_vfs::FileSystem;
    let mut mem = std::fs::read(image).map_err(|e| format!("{}: {e}", image.display()))?;
    let io = DeviceIo::new(poly_block::RamDisk::new(&mut mem, 512).ok_or("bad image size")?);
    let fs = Ext2::mount(&io, std::sync::Arc::new(FixedClock), MountOptions { read_only: true, ordered: true })
        .map_err(|e| format!("{}: {e}", image.display()))?;
    let mut ino = fs.root();
    for name in path.split('/') {
        ino = fs.lookup(ino, name.as_bytes()).map_err(|e| format!("{path} on the image: {}", e.name()))?;
    }
    let size = fs.getattr(ino).map_err(|e| e.name().to_string())?.size;
    let mut buf = vec![0u8; size as usize];
    let n = fs.read(ino, 0, &mut buf).map_err(|e| e.name().to_string())?;
    buf.truncate(n);
    Ok(buf)
}

/// One command line of the session: what it checks, the programs it needs
/// (skipped without them), the line typed into bash, and its output
/// check.
struct Step {
    what: &'static str,
    needs: &'static [&'static str],
    line: String,
    ok: Box<dyn Fn(&str) -> bool>,
}

fn step(what: &'static str, needs: &'static [&'static str], line: &str, want: &str) -> Step {
    let want = want.to_string();
    Step { what, needs, line: line.to_string(), ok: Box::new(move |out| out == want) }
}

impl Step {
    /// The same step with another output check.
    fn then(self, ok: impl Fn(&str) -> bool + 'static) -> Step {
        Step { ok: Box::new(ok), ..self }
    }
}

/// The step that compiles `/root/hello` onto the disk, which the test then
/// reads back from the image.
const GCC_STEP: &str = "gcc compiles hello.c onto the disk and runs it";

/// The session's command lines, in order.
fn steps() -> Vec<Step> {
    let mut steps = vec![
        // Long command lines on one line: readline scrolls them at the
        // terminal's width, which the serial line has not set.
        step("stty cols 250 rows 50", &["coreutils"], "stty cols 250 rows 50 && stty size", "50 250\n"),
        Step {
            what: "bash as the login shell",
            needs: &[],
            line: "echo \"bash $BASH_VERSINFO $0\"; shopt -q login_shell && echo login shell".into(),
            ok: Box::new(|out| out.starts_with("bash ") && out.ends_with(" /bin/bash\nlogin shell\n")),
        },
        step(
            "bash -c 'ls / | sort | head -3'",
            &["coreutils"],
            "bash -c 'ls / | sort | head -3'; echo \"status=$?\"",
            "bin\ndev\netc\nstatus=0\n",
        ),
        step(
            "bash -c 'seq 1 1000 | grep 7 | wc -l'",
            &["coreutils", "grep"],
            "bash -c 'seq 1 1000 | grep 7 | wc -l'; echo \"status=$?\"",
            "271\nstatus=0\n",
        ),
        step(
            "seq 10 | sed -n 3,5p | tr",
            &["coreutils", "sed"],
            "bash -c 'seq 10 | sed -n \"3,5p\" | tr \"\\n\" \" \"; echo'",
            "3 4 5 \n",
        ),
        Step {
            what: "mkdir, cp, mv, cat, wc, rm and ls on the disk",
            needs: &["coreutils"],
            line: "bash -c 'cd /root && mkdir -p d && echo hi > d/a && cp d/a d/b && mv d/b d/c && cat d/a d/c | wc -c && rm -r d && ls d'; echo \"status=$?\"".into(),
            ok: Box::new(|out| out.starts_with("6\nls: ") && out.ends_with("\nstatus=2\n")),
        },
        step(
            "env, date, tail, head",
            &["coreutils"],
            "env -i FOO=bar env; date +%Y | wc -c; tail -n 1 /etc/passwd | head -c 6; echo",
            "FOO=bar\n5\nnobody\n",
        ),
        step("bash -c 'exit 7'", &[], "bash -c 'exit 7'; echo \"status=$?\"", "status=7\n"),
        // The terminal's size (TIOCGWINSZ) as bash's checkwinsize sees it,
        // command substitution, here-strings, arithmetic.
        step(
            "bash: window size, substitution, arithmetic",
            &[],
            "echo \"$COLUMNS $LINES $(echo sub) $((6 * 7))\"; read -r x <<< 'here'; echo $x",
            "250 50 sub 42\nhere\n",
        ),
        step(
            "stat, ls -l, readlink /proc/self/exe, ulimit",
            &["coreutils"],
            "stat -c '%s %a %U %F' /etc/hostname; ls -l /etc/hostname | cut -d' ' -f1,3,5; readlink /proc/self/exe; ulimit -n",
            "5 644 root regular file\n-rw-r--r-- root 5\n/usr/bin/readlink\n1024\n",
        ),
        step(
            "dd 32 MiB through the page cache, wc, sha256sum, rm",
            &["coreutils"],
            "dd if=/dev/zero of=/root/big bs=1M count=32 status=none && wc -c < /root/big && cat /root/big | sha256sum | cut -c1-8 && head -c 33554432 /dev/zero | sha256sum | cut -c1-8 && rm /root/big",
            "33554432\n",
        )
        .then(|out| {
            // The two digests of 32 MiB of zeros agree.
            let lines: Vec<&str> = out.lines().collect();
            lines.len() == 3 && lines[0] == "33554432" && lines[1] == lines[2] && lines[1].len() == 8
        }),
        step("a background job", &["coreutils"], "sleep 30 &", "").then(|out| out.starts_with("[1] ")),
        step("jobs", &["coreutils"], "jobs", "").then(|out| out.starts_with("[1]+  Running") && out.contains("sleep 30 &")),
        step("kill %1 and wait", &["coreutils"], "kill %1; wait %1; echo \"status=$?\"", "")
            .then(|out| out.ends_with("status=143\n")),
        step(
            "python3 /root/poly-test.py",
            &["python3", "coreutils"],
            "python3 /root/poly-test.py; echo \"status=$?\"",
            &format!("{PYTHON_EXPECTED}status=0\n"),
        ),
        step(
            "python3 /root/poly-test-sys.py",
            &["python3", "coreutils"],
            "python3 /root/poly-test-sys.py; echo \"status=$?\"",
            &format!("{}status=0\n", python_sys_expected(crate::QEMU_CPUS)),
        ),
        step(
            GCC_STEP,
            &["gcc"],
            "gcc -O1 -Wall -o /root/hello /root/hello.c && /root/hello a b; echo \"status=$?\"",
            "hello from gcc on Poly, argc=3\nstatus=42\n",
        ),
        step(
            "gcc: two files compiled apart, linked with libm, run",
            &["gcc", "coreutils"],
            "cd /root/cc && gcc -O2 -Wall -c util.c && gcc -O2 -Wall -o prog main.c util.o -lm && ./prog; echo \"status=$?\"; cd",
            "from the child; sum 500500; sqrt 1.414; child 0\nstatus=0\n",
        ),
    ];
    steps.push(step(
        PERSIST_STEP,
        &[],
        &format!("echo '{PERSIST_TEXT}' > /root/persist.txt && sync && echo synced"),
        "synced\n",
    ));
    steps
}

/// Lines the kernel logs on the console that are not the programs'
/// output: taken out of each command's output (and checked separately).
fn is_kernel_line(line: &str) -> bool {
    ["syscall: pid ", "linux: pid ", "fs: ", "strace: "].iter().any(|p| line.starts_with(p))
}

/// Run the steps; `ran` gets the names of those that ran.
fn session(
    s: &mut crate::busybox::Session,
    have: &BTreeSet<&'static str>,
    ran: &mut BTreeSet<&'static str>,
) -> crate::Result {
    s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    s.prompt()?;
    let mut failed = Vec::new();
    let mut unimplemented = BTreeSet::new();
    // POLY_ROOTDISK_STEPS=<text>: only the steps whose name contains it
    // (and the terminal's size), for debugging one program.
    let only = std::env::var("POLY_ROOTDISK_STEPS").ok().filter(|s| !s.is_empty());
    for step in steps() {
        if let Some(only) = &only
            && !step.what.contains(only.as_str())
            && !step.what.starts_with("stty")
        {
            continue;
        }
        if let Some(missing) = step.needs.iter().find(|n| !have.contains(**n)) {
            eprintln!("==> skipped: {} (no {missing} on this host)", step.what);
            continue;
        }
        ran.insert(step.what);
        let raw = s.run(&step.line)?;
        let mut out = String::new();
        for line in raw.split_inclusive('\n') {
            if is_kernel_line(line) {
                // No call may be missing (`rseq`, the last one a program
                // here made, is implemented since ADR 0111).
                if line.contains(" not implemented") {
                    unimplemented.insert(line.trim_end().to_string());
                }
            } else {
                out.push_str(line);
            }
        }
        if !(step.ok)(&out) {
            eprintln!("==> FAILED: {}: {out:?}", step.what);
            failed.push(format!("{} (printed {out:?})", step.what));
        }
    }
    // Ctrl-C ends bash's foreground job (exit status 128 + SIGINT).
    if have.contains("coreutils") && only.is_none() {
        s.send("echo started; sleep 20\r")?;
        s.expect("started\n")?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        s.send("\x03")?;
        s.prompt()?;
        let out = s.run("echo \"status=$?\"")?;
        if out != "status=130\n" {
            failed.push(format!("Ctrl-C in bash (printed {out:?})"));
        }
    }
    // poweroff (BusyBox's init powers off on SIGUSR2), which unmounts the
    // disk and ends QEMU with status 0.
    s.send("poweroff\r")?;
    s.expect("reboot: Power down.")?;
    let status = s.finish()?;
    if status != Some(0) {
        failed.push(format!("poweroff (QEMU status {status:?})"));
    }
    if !unimplemented.is_empty() {
        failed.push(format!("system calls Poly does not implement: {:?}", unimplemented));
    }
    if failed.is_empty() { Ok(()) } else { Err(format!("root disk on Poly: failed: {}", failed.join("; "))) }
}
