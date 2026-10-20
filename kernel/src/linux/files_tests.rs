//! In-kernel tests of the file system services and of `/proc`, `/sys` and
//! `/dev` as a Linux process sees them (ADR 0043): a process whose thread
//! runs in ring 3 and makes one call into the test hook, which runs the
//! checks on that thread, in that system call, through
//! `Linux::dispatch_with` with the kernel's [`Thread`] and real user
//! memory, as the system call dispatcher does. The program then exits
//! with the result: 0 when every check passed, else the number of the
//! first that failed (logged).

use super::Thread;
use crate::fs;
use crate::kernel_test;
use crate::process::{self, ExitStatus, Process};
use crate::sync::SpinLock;
use crate::time;
use crate::user::memory::{Layout, Placement, Prot};
use crate::user::{self, Entry, Memory};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_abi::SyscallRegs;
use poly_binfmt::{Arch, Bits, Format};
use poly_personality_linux::{Errno, Linux, nr};

type Check = Result<(), u64>;

/// The hook's own system call: run the test's checks.
const CALLBACK: u64 = 0x7e58;
/// Where the program is mapped (read, execute).
const CODE: u64 = 0x40_0000;
/// 64 KiB of read-write data: strings at `PATHS`, buffers at `BUF`.
const DATA: u64 = 0x60_0000;
const DATA_LEN: u64 = 0x1_0000;
const PATHS: u64 = DATA + 0x1000;
const BUF: u64 = DATA + 0x8000;
const BUF_LEN: u64 = 0x8000;
/// Where the program break starts.
const BRK: u64 = 0x80_0000;
const DEADLINE_NS: u64 = 60 * time::NANOS_PER_SEC;
const AT_FDCWD: u64 = -100i64 as u64;

global_asm!(
    r#"
    .section .rodata.linux_fs_test_program, "a", @progbits
    .balign 16
    .global linux_fs_test_program
linux_fs_test_program:
    mov eax, {callback}
    syscall
    mov rdi, rax
    mov eax, 231
    syscall
    ud2
    .global linux_fs_test_program_end
linux_fs_test_program_end:
    "#,
    callback = const CALLBACK,
);

unsafe extern "C" {
    static linux_fs_test_program: u8;
    static linux_fs_test_program_end: u8;
}

type Callback = fn(&Process) -> Check;

static RUN: SpinLock<Option<Callback>> = SpinLock::new(None);

fn hook(process: &Process, regs: &mut SyscallRegs) {
    if regs.rax == CALLBACK {
        let f = RUN.lock().expect("a callback is set");
        regs.rax = match f(process) {
            Ok(()) => 0,
            Err(n) => n,
        };
    } else {
        user::DEFAULT_HOOK(process, regs);
    }
}

/// A Linux process with the program, data, a break and a stack.
fn setup() -> Result<(Arc<Process>, Arc<Memory>), &'static str> {
    let format = Format::Elf { bits: Bits::B64, arch: Arch::X86_64 };
    let process = Arc::new(Process::new_linux(process::alloc_pid(), 0, format, false));
    let memory = Memory::new(Layout::default()).map_err(|_| "no memory for a memory map")?;
    let start = &raw const linux_fs_test_program;
    let len = (&raw const linux_fs_test_program_end) as usize - start as usize;
    // SAFETY: the assembly above puts the program between the two symbols,
    // in a read-only section of the kernel image.
    let code = unsafe { core::slice::from_raw_parts(start, len) };
    let ok = memory.map_anonymous(Placement::FixedNoReplace(CODE), 4096, Prot::RW).is_ok()
        && memory.write(CODE, code).is_ok()
        && memory.protect(CODE, 4096, Prot::RX).is_ok()
        && memory.map_anonymous(Placement::FixedNoReplace(DATA), DATA_LEN, Prot::RW).is_ok()
        && memory.init_brk(BRK).is_ok()
        && memory.setup_stack(64 << 10, Prot::RW).is_ok();
    if !ok {
        return Err("could not lay out the test memory");
    }
    Ok((process, Arc::new(memory)))
}

/// Run `f` in a thread of `process` in `memory`; its result is the exit
/// status.
fn run(process: &Arc<Process>, memory: &Arc<Memory>, f: Callback) -> Result<(), &'static str> {
    *RUN.lock() = Some(f);
    let rsp = memory.layout().stack_top - 64;
    let entry = Entry { rip: CODE, rsp, args: [0; 3] };
    let tid = user::spawn_in(Arc::clone(process), Arc::clone(memory), entry, hook).map_err(|_| "spawn failed")?;
    let _ = crate::thread::detach(tid);
    let status = process.wait_until(time::now_ns() + DEADLINE_NS).ok_or("the test process did not end in time")?;
    if status == ExitStatus::Exited(0) {
        Ok(())
    } else {
        crate::kprintln!("linux fs test: {status}");
        Err("a check failed (see the log)")
    }
}

/// A system call from the callback, as the dispatcher makes it.
fn sys(p: &Process, number: u64, args: &[u64]) -> Result<u64, Errno> {
    let mut k = Thread::current(p).expect("a thread of a Linux process");
    let mut a = [0u64; 6];
    a[..args.len()].copy_from_slice(args);
    let mut regs =
        SyscallRegs { rax: number, rdi: a[0], rsi: a[1], rdx: a[2], r10: a[3], r8: a[4], r9: a[5], ..Default::default() };
    Linux::dispatch_with(&mut k, &mut regs);
    match Errno::from_rax(regs.rax) {
        Some(e) => Err(e),
        None => Ok(regs.rax),
    }
}

/// Fails check `n`, logging what was expected.
macro_rules! check {
    ($n:expr, $cond:expr) => {
        if !$cond {
            crate::kprintln!("linux fs test: check {} failed: {}", $n, stringify!($cond));
            return Err($n);
        }
    };
}

/// `s` as a NUL-terminated string in user memory, in slot `slot` (256
/// bytes each).
fn cstr(slot: u64, s: &str) -> u64 {
    let at = PATHS + slot * 256;
    let mut b = Vec::from(s.as_bytes());
    b.push(0);
    let _ = user::copy_to_user(at, &b);
    at
}

/// The bytes at `at` in user memory.
fn peek(at: u64, len: usize) -> Vec<u8> {
    let mut b = alloc::vec![0u8; len];
    let _ = user::copy_from_user(&mut b, at);
    b
}

/// The whole file at `path`, read with openat/read/close from ring 3's
/// point of view.
fn read_file(p: &Process, path: &str) -> Result<Vec<u8>, Errno> {
    let fd = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(15, path), 0, 0])?;
    let mut out = Vec::new();
    let result = loop {
        match sys(p, nr::READ, &[fd, BUF, BUF_LEN]) {
            Ok(0) => break Ok(out),
            Ok(n) => out.extend_from_slice(&peek(BUF, n as usize)),
            Err(e) => break Err(e),
        }
    };
    let _ = sys(p, nr::CLOSE, &[fd]);
    result
}

fn text(p: &Process, path: &str) -> String {
    read_file(p, path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

fn readlink(p: &Process, path: &str) -> Result<Vec<u8>, Errno> {
    let n = sys(p, nr::READLINKAT, &[AT_FDCWD, cstr(14, path), BUF, 4096])?;
    Ok(peek(BUF, n as usize))
}

/// The names getdents64 lists for the directory open as `fd`.
fn list(p: &Process, fd: u64) -> Result<Vec<Vec<u8>>, Errno> {
    let mut names = Vec::new();
    loop {
        let n = sys(p, nr::GETDENTS64, &[fd, BUF, 512])? as usize;
        if n == 0 {
            return Ok(names);
        }
        let buf = peek(BUF, n);
        let mut at = 0;
        while at < n {
            let reclen = u16::from_le_bytes([buf[at + 16], buf[at + 17]]) as usize;
            let name = &buf[at + 19..at + reclen];
            names.push(name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())].to_vec());
            at += reclen;
        }
    }
}

kernel_test! {
    fn proc_self_describes_the_calling_process() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| {
            let pid = alloc::format!("{}", p.pid);
            check!(1, readlink(p, "/proc/self") == Ok(pid.clone().into_bytes()));
            // Grow the break so there is a heap to see.
            check!(2, sys(p, nr::BRK, &[BRK + 0x3000]) == Ok(BRK + 0x3000));
            let maps = text(p, "/proc/self/maps");
            let lines: Vec<&str> = maps.lines().collect();
            check!(3, lines.iter().any(|l| l.starts_with("00400000-00401000 r-xp 00000000 00:00 0")));
            check!(4, lines.iter().any(|l| l.starts_with("00600000-00610000 rw-p ")));
            check!(5, lines.iter().any(|l| l.starts_with("00800000-00803000 rw-p ") && l.ends_with("[heap]")));
            check!(6, lines.iter().any(|l| l.contains(" rw-p ") && l.ends_with("[stack]")));
            // A file mapping shows its path, device and inode.
            let f = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(0, "/tmp/fs-test-map"), 0o102, 0o644]).map_err(|_| 7u64)?;
            check!(8, sys(p, nr::WRITE, &[f, cstr(1, "mapped"), 6]) == Ok(6));
            let at = sys(p, nr::MMAP, &[0, 4096, 1, 2, f, 0]).map_err(|_| 9u64)?;
            let maps = text(p, "/proc/self/maps");
            let line = maps.lines().find(|l| l.starts_with(&alloc::format!("{at:08x}-"))).unwrap_or("");
            check!(10, line.contains(" r--p 00000000 00:") && line.ends_with(" /tmp/fs-test-map"));
            check!(11, sys(p, nr::UNLINKAT, &[AT_FDCWD, cstr(0, "/tmp/fs-test-map"), 0]) == Ok(0));
            check!(12, text(p, "/proc/self/maps").contains(" /tmp/fs-test-map (deleted)\n"));
            let status = text(p, "/proc/self/status");
            check!(13, status.contains(&alloc::format!("\nPid:\t{pid}\n")));
            check!(14, status.contains("\nState:\tR (running)\n"));
            check!(15, status.contains("\nUid:\t0\t0\t0\t0\n") && status.contains("\nUmask:\t0022\n"));
            check!(16, status.contains("\nThreads:\t1\n") && status.contains("\nVmStk:"));
            let stat = text(p, "/proc/self/stat");
            check!(17, stat.starts_with(&alloc::format!("{pid} (?) R 0 {pid} {pid} ")));
            check!(18, stat.split_whitespace().count() == 52);
            check!(19, text(p, &alloc::format!("/proc/{pid}/statm")).split_whitespace().count() == 7);
            Ok(())
        })
    }
}

kernel_test! {
    fn proc_shows_the_program_its_arguments_and_descriptors() -> TestResult {
        let (process, memory) = setup()?;
        // A "program" file, and an initial stack as exec lays it out:
        // argv[0], argv[1] and envp[0] pointing at strings in a row.
        let files = &process.files;
        let flags = fs::OpenFlags::CREAT | fs::OpenFlags::WRONLY | fs::OpenFlags::TRUNC;
        let fd = files.open(b"/tmp/fs-test-prog", flags, 0o755).map_err(|e| e.name())?;
        let file = files.file(fd).map_err(|e| e.name())?;
        files.close(fd).map_err(|e| e.name())?;
        let strings = DATA + 0x200;
        let ok = memory.write(strings, b"prog\0arg1\0A=1\0").is_ok()
            && memory.write(DATA + 0x100, &strings.to_le_bytes()).is_ok()
            && memory.write(DATA + 0x110, &(strings + 10).to_le_bytes()).is_ok();
        if !ok {
            return Err("could not write the strings");
        }
        let stack = poly_elf::StackLayout {
            rsp: DATA + 0xf8,
            argc: 2,
            argv: DATA + 0x100,
            envp: DATA + 0x110,
            auxv: 0,
            auxc: 0,
            random: 0,
            execfn: 0,
            phdr: 0,
        };
        fs::proc::record_exec(&process, &file, &memory, &stack, &[b"prog", b"arg1"], &[b"A=1"]);
        drop(file);
        let result = run(&process, &memory, |p| {
            check!(1, readlink(p, "/proc/self/exe") == Ok(b"/tmp/fs-test-prog".to_vec()));
            check!(2, read_file(p, "/proc/self/cmdline") == Ok(b"prog\0arg1\0".to_vec()));
            check!(3, read_file(p, "/proc/self/environ") == Ok(b"A=1\0".to_vec()));
            check!(4, text(p, "/proc/self/comm") == "fs-test-prog\n");
            check!(5, text(p, "/proc/self/stat").contains(" (fs-test-prog) R "));
            // Descriptors: fd 3 on a file, its link and its fdinfo.
            let fd = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(0, "/tmp/fs-test-prog"), 0o2000000, 0]).map_err(|_| 6u64)?;
            check!(7, fd == 3);
            check!(8, readlink(p, "/proc/self/fd/3") == Ok(b"/tmp/fs-test-prog".to_vec()));
            check!(9, readlink(p, "/proc/self/fd/0") == Ok(b"/dev/console".to_vec()));
            check!(10, text(p, "/proc/self/fdinfo/3").starts_with("pos:\t0\nflags:\t02000000\nmnt_id:\t"));
            // Reading a descriptor's own fdinfo: the file being read is
            // the one described, and nothing deadlocks.
            let own = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(1, "/proc/self/fdinfo/4"), 0, 0]).map_err(|_| 11u64)?;
            check!(12, own == 4);
            let n = sys(p, nr::READ, &[own, BUF, 4096]).map_err(|_| 13u64)?;
            check!(14, String::from_utf8_lossy(&peek(BUF, n as usize)).starts_with("pos:\t0\nflags:\t00\n"));
            check!(15, sys(p, nr::CLOSE, &[own]) == Ok(0));
            let dir = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(2, "/proc/self/fd"), 0o200000, 0]).map_err(|_| 16u64)?;
            let names = list(p, dir).map_err(|_| 17u64)?;
            check!(18, names == [&b"."[..], b"..", b"0", b"1", b"2", b"3", b"4"]);
            // The link follows to the file.
            check!(19, sys(p, nr::NEWFSTATAT, &[AT_FDCWD, cstr(3, "/proc/self/fd/3"), BUF, 0]) == Ok(0));
            check!(20, u32::from_le_bytes(peek(BUF + 24, 4).try_into().unwrap_or_default()) == 0o100755);
            // The program is deleted: exe says so.
            check!(21, sys(p, nr::UNLINKAT, &[AT_FDCWD, cstr(0, "/tmp/fs-test-prog"), 0]) == Ok(0));
            check!(22, readlink(p, "/proc/self/exe") == Ok(b"/tmp/fs-test-prog (deleted)".to_vec()));
            Ok(())
        });
        result
    }
}

kernel_test! {
    fn proc_lists_processes_and_system_files() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| {
            let dir = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(0, "/proc"), 0o200000, 0]).map_err(|_| 1u64)?;
            let names = list(p, dir).map_err(|_| 2u64)?;
            check!(3, names.iter().any(|n| n == b"self") && names.iter().any(|n| n == b"cpuinfo"));
            check!(4, names.iter().any(|n| n == alloc::format!("{}", p.pid).as_bytes()));
            let cpu = text(p, "/proc/cpuinfo");
            check!(5, cpu.starts_with("processor\t: 0\nvendor_id\t: ") && cpu.contains("\nflags\t\t: fpu "));
            check!(6, cpu.contains(" lm"));
            let mem = text(p, "/proc/meminfo");
            check!(7, mem.starts_with("MemTotal:") && !mem.starts_with("MemTotal:               0 kB"));
            check!(8, text(p, "/proc/uptime").split_whitespace().count() == 2);
            check!(9, text(p, "/proc/loadavg").split_whitespace().count() == 5);
            check!(10, text(p, "/proc/version").starts_with("Linux version 6.1.0-poly "));
            check!(11, text(p, "/proc/stat").contains("\nbtime "));
            let mounts = text(p, "/proc/mounts");
            check!(12, mounts.contains(" /proc proc rw,nosuid,nodev,noexec 0 0\n"));
            check!(13, mounts.contains(" /sys sysfs ") && mounts.contains(" /dev/shm tmpfs "));
            check!(14, text(p, "/proc/self/mountinfo").contains(" / / rw - tmpfs "));
            check!(15, text(p, "/proc/filesystems").contains("nodev\tproc\n"));
            check!(16, text(p, "/proc/sys/kernel/ostype") == "Linux\n");
            check!(17, text(p, "/proc/sys/kernel/random/uuid").len() == 37);
            check!(18, text(p, "/sys/devices/system/cpu/online") == "0\n");
            check!(19, text(p, "/sys/dev/char/1:9/dev") == "1:9\n");
            check!(20, readlink(p, "/sys/class/mem/null") == Ok(b"../../devices/virtual/mem/null".to_vec()));
            // A working directory inside /proc, and its path.
            check!(21, sys(p, nr::CHDIR, &[cstr(1, "/proc/self/fd")]) == Ok(0));
            let n = sys(p, nr::GETCWD, &[BUF, 256]).map_err(|_| 22u64)?;
            check!(23, peek(BUF, n as usize) == alloc::format!("/proc/{}/fd\0", p.pid).into_bytes());
            check!(24, sys(p, nr::CHDIR, &[cstr(1, "/proc/self/cwd/../..")]) == Ok(0));
            let n = sys(p, nr::GETCWD, &[BUF, 256]).map_err(|_| 25u64)?;
            check!(26, peek(BUF, n as usize) == b"/proc\0");
            // statfs and the read-only mount.
            check!(27, sys(p, nr::STATFS, &[cstr(2, "/proc/self"), BUF]) == Ok(0));
            check!(28, u64::from_le_bytes(peek(BUF, 8).try_into().unwrap_or_default()) == 0x9fa0);
            check!(29, sys(p, nr::OPENAT, &[AT_FDCWD, cstr(3, "/proc/meminfo"), 1, 0]) == Err(Errno::EROFS));
            Ok(())
        })
    }
}

kernel_test! {
    fn dev_random_urandom_and_links() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| {
            let fd = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(0, "/dev/urandom"), 0, 0]).map_err(|_| 1u64)?;
            check!(2, sys(p, nr::READ, &[fd, BUF, 64]) == Ok(64));
            let a = peek(BUF, 64);
            check!(3, sys(p, nr::READ, &[fd, BUF, 64]) == Ok(64));
            let b = peek(BUF, 64);
            check!(4, a != b && a.iter().any(|&x| x != 0));
            // Writes are accepted (and mixed in).
            check!(5, sys(p, nr::WRITE, &[fd, cstr(1, "seed"), 4]) == Ok(4));
            let r = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(2, "/dev/random"), 0, 0]).map_err(|_| 6u64)?;
            check!(7, sys(p, nr::READ, &[r, BUF, 4096]) == Ok(512));
            check!(8, sys(p, nr::GETRANDOM, &[BUF, 32, 0]) == Ok(32));
            check!(9, readlink(p, "/dev/stdin") == Ok(b"/proc/self/fd/0".to_vec()));
            check!(10, readlink(p, "/dev/fd") == Ok(b"/proc/self/fd".to_vec()));
            // /dev/stdout leads to the console: a character device.
            check!(11, sys(p, nr::NEWFSTATAT, &[AT_FDCWD, cstr(3, "/dev/stdout"), BUF, 0]) == Ok(0));
            check!(12, u32::from_le_bytes(peek(BUF + 24, 4).try_into().unwrap_or_default()) & 0o170000 == 0o020000);
            // /dev/shm is a writable tmpfs.
            let shm = sys(p, nr::OPENAT, &[AT_FDCWD, cstr(4, "/dev/shm/x"), 0o102, 0o600]).map_err(|_| 13u64)?;
            check!(14, sys(p, nr::CLOSE, &[shm]) == Ok(0));
            check!(15, sys(p, nr::UNLINKAT, &[AT_FDCWD, cstr(4, "/dev/shm/x"), 0]) == Ok(0));
            Ok(())
        })
    }
}

kernel_test! {
    fn file_calls_and_limits_from_ring_3() -> TestResult {
        let (process, memory) = setup()?;
        run(&process, &memory, |p| {
            // prlimit64: the defaults, then a soft RLIMIT_NOFILE of 5.
            check!(1, sys(p, nr::PRLIMIT64, &[0, 7, 0, BUF]) == Ok(0));
            check!(2, peek(BUF, 16) == [1024u64.to_le_bytes(), 4096u64.to_le_bytes()].concat());
            let _ = user::copy_to_user(BUF, &[5u64.to_le_bytes(), 4096u64.to_le_bytes()].concat());
            check!(3, sys(p, nr::PRLIMIT64, &[0, 7, BUF, 0]) == Ok(0));
            check!(4, p.files.fd_limit() == 5);
            let d = cstr(0, "/tmp");
            let a = sys(p, nr::OPENAT, &[AT_FDCWD, d, 0o200000, 0]).map_err(|_| 5u64)?;
            check!(6, a == 3);
            check!(7, sys(p, nr::OPENAT, &[AT_FDCWD, d, 0o200000, 0]) == Ok(4));
            check!(8, sys(p, nr::OPENAT, &[AT_FDCWD, d, 0o200000, 0]) == Err(Errno::EMFILE));
            check!(9, sys(p, nr::CLOSE, &[4]) == Ok(0));
            // Paths relative to a directory descriptor, getdents64, rename.
            check!(10, sys(p, nr::MKDIRAT, &[a, cstr(1, "fs-test-dir"), 0o755]) == Ok(0));
            let f = sys(p, nr::OPENAT, &[a, cstr(2, "fs-test-dir/f"), 0o102, 0o600]).map_err(|_| 11u64)?;
            check!(12, sys(p, nr::PWRITE64, &[f, cstr(3, "hello"), 5, 10]) == Ok(5));
            check!(13, sys(p, nr::LSEEK, &[f, 0, 2]) == Ok(15));
            check!(14, sys(p, nr::FTRUNCATE, &[f, 12]) == Ok(0));
            check!(15, sys(p, nr::PREAD64, &[f, BUF, 100, 9]) == Ok(3));
            check!(16, peek(BUF, 3) == b"\0he");
            check!(17, sys(p, nr::FCNTL, &[f, 3, 0]) == Ok(0o2));
            check!(18, sys(p, nr::RENAMEAT2, &[a, cstr(2, "fs-test-dir/f"), a, cstr(4, "fs-test-dir/g"), 1]) == Ok(0));
            check!(19, readlink(p, &alloc::format!("/proc/self/fd/{f}")) == Ok(b"/tmp/fs-test-dir/g".to_vec()));
            check!(20, sys(p, nr::CLOSE, &[f]) == Ok(0));
            let dir = sys(p, nr::OPENAT, &[a, cstr(5, "fs-test-dir"), 0o200000, 0]).map_err(|_| 21u64)?;
            check!(22, list(p, dir) == Ok(alloc::vec![b".".to_vec(), b"..".to_vec(), b"g".to_vec()]));
            check!(23, sys(p, nr::CLOSE, &[dir]) == Ok(0));
            check!(24, sys(p, nr::STATX, &[a, cstr(4, "fs-test-dir/g"), 0, 0x7ff, BUF]) == Ok(0));
            check!(25, u64::from_le_bytes(peek(BUF + 0x28, 8).try_into().unwrap_or_default()) == 12);
            check!(26, sys(p, nr::FACCESSAT2, &[a, cstr(4, "fs-test-dir/g"), 4, 0]) == Ok(0));
            check!(27, sys(p, nr::SYMLINKAT, &[cstr(6, "fs-test-dir/g"), a, cstr(7, "fs-test-link")]) == Ok(0));
            check!(28, readlink(p, "/tmp/fs-test-link") == Ok(b"fs-test-dir/g".to_vec()));
            check!(29, sys(p, nr::UNLINKAT, &[a, cstr(7, "fs-test-link"), 0]) == Ok(0));
            check!(30, sys(p, nr::UNLINKAT, &[a, cstr(4, "fs-test-dir/g"), 0]) == Ok(0));
            check!(31, sys(p, nr::UNLINKAT, &[a, cstr(1, "fs-test-dir"), 0x200]) == Ok(0));
            // dup family and the working directory.
            check!(32, sys(p, nr::DUP3, &[a, 4, 0o2000000]) == Ok(4));
            check!(33, sys(p, nr::FCNTL, &[4, 1, 0]) == Ok(1));
            check!(34, sys(p, nr::FCHDIR, &[a]) == Ok(0));
            let n = sys(p, nr::GETCWD, &[BUF, 64]).map_err(|_| 35u64)?;
            check!(36, peek(BUF, n as usize) == b"/tmp\0");
            check!(37, sys(p, nr::UMASK, &[0o077]) == Ok(0o022));
            check!(38, sys(p, nr::SYSINFO, &[BUF]) == Ok(0));
            check!(39, u64::from_le_bytes(peek(BUF + 32, 8).try_into().unwrap_or_default()) > 0);
            check!(40, sys(p, nr::SYNC, &[]) == Ok(0));
            check!(41, sys(p, nr::FSYNC, &[a]) == Ok(0));
            Ok(())
        })
    }
}
