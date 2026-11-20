//! The committed arm64 test programs (`tests/a64/xnu/bin`, built by its
//! `build.sh` from our C sources, and poly-dyld's `hello_a64`) loaded with
//! [`load`] and run through [`run`] with a host that answers `write`, the
//! same way the kernel and the runtime do it on Poly. Under the interpreter
//! on every host, under the JIT on x86-64 ones.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::string::String;
use std::vec::Vec;

use poly_a64_jit::jit::{Engine, Stop};
use poly_a64_jit::mem::{GuestMem, R, W, X};
use poly_abi::{RFLAGS_CF, SyscallRegs};
use poly_macho::CpuKind;

use crate::handoff::EngineKind;
use crate::layout::Layout;
use crate::load::{Args, Target, load};
use crate::run::{self, Bridge, Host};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn file(rel: &str) -> Vec<u8> {
    let p = root().join(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

const BIN: &str = "tests/a64/xnu/bin";
const DYLD: &str = "crates/poly-dyld/tests/data";

/// The disk the loader sees.
struct Disk(BTreeMap<String, Vec<u8>>);

impl poly_dyld::FileSystem for Disk {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        self.0.get(path).cloned()
    }
}

fn disk() -> Disk {
    let mut d = BTreeMap::new();
    for (at, rel) in [
        ("/bin/a64-hello-static", format!("{BIN}/hello-static")),
        ("/bin/a64-hello-fat", format!("{BIN}/hello-fat")),
        ("/bin/a64-hello", format!("{BIN}/hello")),
        ("/bin/a64-heap", format!("{BIN}/heap")),
        ("/bin/a64-crash", format!("{BIN}/crash")),
        ("/usr/lib/libSystem.B.dylib", format!("{BIN}/libSystem.B.dylib")),
        ("/bin/mhello-a64", format!("{DYLD}/hello_a64")),
        ("/usr/lib/poly-dyld-test/libgreet.dylib", format!("{DYLD}/libgreet_a64.dylib")),
    ] {
        d.insert(String::from(at), file(&rel));
    }
    Disk(d)
}

/// The kernel's view, on a guest window: no flags (the runtime sets them
/// from the hand-off), bytes written without checks.
struct WindowTarget<'a>(&'a mut GuestMem);

impl Target for WindowTarget<'_> {
    fn map(&mut self, addr: u64, len: u64) -> Result<(), String> {
        let end = self.0.lo() + self.0.size();
        if addr < self.0.lo() || addr + len > end { Err(format!("{addr:#x} outside the window")) } else { Ok(()) }
    }
    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), String> {
        self.0.poke(addr, data).map_err(|e| format!("{e:?}"))
    }
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), String> {
        self.0.peek(addr, buf).map_err(|e| format!("{e:?}"))
    }
}

/// A host with `write` (to a buffer), anonymous `mmap` at a fixed address,
/// `munmap`, `mprotect`, `getpid` and `kill` (recorded); everything else is
/// ENOSYS.
#[derive(Default)]
struct Console {
    out: Vec<u8>,
    calls: Vec<u64>,
    killed: Option<u64>,
}

fn answer(f: &mut SyscallRegs, r: Result<u64, u64>) {
    match r {
        Ok(v) => {
            f.rax = v;
            f.rflags &= !RFLAGS_CF;
        }
        Err(e) => {
            f.rax = e;
            f.rflags |= RFLAGS_CF;
        }
    }
}

impl Host for Console {
    fn syscall(&mut self, f: &mut SyscallRegs, _stack: [u64; 2], mem: &mut GuestMem) {
        self.calls.push(f.rax);
        let r = match f.rax {
            0x200_0004 if f.rdi == 1 || f.rdi == 2 => {
                let mut buf = std::vec![0u8; f.rdx as usize];
                if mem.peek(f.rsi, &mut buf).is_ok() {
                    self.out.extend_from_slice(&buf);
                    Ok(f.rdx)
                } else {
                    Err(14)
                }
            }
            0x200_0004 => Err(9),
            // mmap: what the bridge sends is MAP_FIXED; anonymous only.
            0x200_00c5 if f.r10 & 0x1000 == 0 && f.r8 as i32 == -1 => Err(9),
            0x200_00c5 if f.rsi == 0 => Ok(0),
            0x200_00c5 if f.r10 & 0x1000 != 0 && f.r10 & 0x10 != 0 => {
                let zeros = std::vec![0u8; f.rsi as usize];
                mem.poke(f.rdi, &zeros).map(|()| f.rdi).map_err(|_| 12)
            }
            0x200_00c5 => Err(19),
            0x200_0049 | 0x200_004a => Ok(0),
            0x200_0014 => Ok(1),
            0x200_0025 => {
                self.killed = Some(f.rsi);
                Ok(0)
            }
            _ => Err(78),
        };
        answer(f, r);
    }
}

struct Ran {
    status: Option<i32>,
    out: String,
    stop: Stop,
    engine: Engine,
    calls: u64,
}

fn engine_for(kind: EngineKind) -> Engine {
    match kind {
        EngineKind::Interpreter => Engine::interpreter(),
        #[cfg(all(target_arch = "x86_64", unix))]
        EngineKind::Jit => jit_engine(),
        #[cfg(not(all(target_arch = "x86_64", unix)))]
        EngineKind::Jit => Engine::interpreter(),
    }
}

#[cfg(all(target_arch = "x86_64", unix))]
fn jit_engine() -> Engine {
    const LEN: usize = 16 << 20;
    // SAFETY: an anonymous private mapping, checked; never unmapped (tests
    // are short-lived processes).
    let p = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            LEN,
            libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    assert_ne!(p, libc::MAP_FAILED, "mmap of the code area");
    // SAFETY: a fresh RWX mapping of LEN bytes, used only by this engine.
    unsafe { Engine::with_code(p.cast(), LEN) }
}

fn run_program(path: &str, argv: &[&str], envp: &[&[u8]], kind: EngineKind) -> Ran {
    let layout = Layout::compact(0x1_0000_0000);
    let opts = poly_dyld::Options { library_base: layout.library_base, ..poly_dyld::Options::ARM64 };
    let program = poly_dyld::link(&mut disk(), path, &opts).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut mem = GuestMem::new(layout.window_lo, layout.window_size());
    let argv: Vec<&[u8]> = argv.iter().map(|a| a.as_bytes()).collect();
    let h = load(&mut WindowTarget(&mut mem), &program, &Args { path, argv: &argv, envp, layout: &layout })
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut cpu = run::guest(&h, &mut mem).expect("guest");
    let mut bridge = Bridge::new(Console::default(), &h);
    let mut engine = engine_for(kind);
    let stop = run::run(&mut engine, &mut cpu, &mut mem, &mut bridge);
    Ran {
        status: bridge.exit,
        out: String::from_utf8_lossy(&bridge.host.out).into_owned(),
        stop,
        engine,
        calls: bridge.calls,
    }
}

const STATIC_LINES: &[&str] = &[
    "a64-static: hello from an arm64 program",
    "a64-static: argc=2 argv[0]=/bin/a64-hello-static",
    "a64-static: write to fd 99: carry set, EBADF",
    "a64-static: write returns its count",
    "a64-static: done",
];

const HELLO_LINES: &[&str] = &[
    "a64-hello: hello from arm64 through Poly's dyld",
    "ok initializers ran, libSystem's first",
    "ok __mh_execute_header is the mapped header",
    "ok thread-local variables",
    "ok malloc gives writable memory",
    "ok argc and argv",
    "ok envp ends with null",
    "ok apple[0] is executable_path=",
    "a64-hello: done",
];

fn engines() -> [EngineKind; 2] {
    [EngineKind::Interpreter, EngineKind::Jit]
}

fn lines(out: &str) -> Vec<&str> {
    out.lines().collect()
}

#[test]
fn static_hello() {
    for kind in engines() {
        for path in ["/bin/a64-hello-static", "/bin/a64-hello-fat"] {
            let r = run_program(path, &[path, "x"], &[b"A=1"], kind);
            let want: Vec<String> = STATIC_LINES.iter().map(|l| l.replace("/bin/a64-hello-static", path)).collect();
            assert_eq!(lines(&r.out), want, "{path} {kind:?}");
            assert_eq!((r.status, r.stop), (Some(0), Stop::Exit(0)), "{path} {kind:?}");
        }
    }
}

#[test]
fn dynamic_hello() {
    for kind in engines() {
        let r = run_program("/bin/a64-hello", &["/bin/a64-hello"], &[b"HOME=/"], kind);
        assert_eq!(lines(&r.out), HELLO_LINES, "{kind:?}");
        assert_eq!(r.status, Some(0), "{kind:?}");
    }
}

#[test]
fn poly_dyld_fixture_with_rpath_and_two_libraries() {
    for kind in engines() {
        let r = run_program("/bin/mhello-a64", &["/bin/mhello-a64"], &[], kind);
        assert!(r.out.ends_with("hello: done\n"), "{kind:?}: {}", r.out);
        assert!(!r.out.contains("not ok"), "{kind:?}: {}", r.out);
        assert!(r.out.starts_with("libgreet: init\nhello: init\n"), "{kind:?}: {}", r.out);
        assert_eq!(r.status, Some(0), "{kind:?}");
    }
}

#[test]
fn heap_matches_the_native_build() {
    let want = String::from_utf8(file(&format!("{BIN}/heap-2.out"))).unwrap();
    for kind in engines() {
        let r = run_program("/bin/a64-heap", &["/bin/a64-heap", "2"], &[], kind);
        assert_eq!(r.out, want, "{kind:?} {:?} {}", r.stop, r.calls);
        assert_eq!(r.status, Some(0), "{kind:?}");
        // malloc's chunks and the large blocks came from mmap and went back
        // with munmap through the bridge.
        assert!(r.calls > 10, "{kind:?}: {} calls", r.calls);
        if cfg!(all(target_arch = "x86_64", unix)) && kind == EngineKind::Jit {
            assert!(r.engine.stats.translated > 0);
        }
    }
}

#[test]
fn runnable_cpu_picks_the_slice() {
    assert_eq!(crate::runnable_cpu(&file(&format!("{BIN}/hello-static"))), Some(CpuKind::Arm64));
    assert_eq!(crate::runnable_cpu(&file(&format!("{BIN}/hello-fat"))), Some(CpuKind::Arm64));
    assert_eq!(crate::runnable_cpu(&file(&format!("{DYLD}/static_x64"))), Some(CpuKind::X86_64));
    assert_eq!(crate::runnable_cpu(b"\x7fELF not a mach-o file"), None);
    assert_eq!(crate::runnable_cpu(&[]), None);
}

#[test]
fn handoff_regions_have_the_segment_protections() {
    let layout = Layout::compact(0x1_0000_0000);
    let opts = poly_dyld::Options { library_base: layout.library_base, ..poly_dyld::Options::ARM64 };
    let program = poly_dyld::link(&mut disk(), "/bin/a64-hello", &opts).unwrap();
    let mut mem = GuestMem::new(layout.window_lo, layout.window_size());
    let argv: [&[u8]; 1] = [b"/bin/a64-hello"];
    let h = load(
        &mut WindowTarget(&mut mem),
        &program,
        &Args { path: "/bin/a64-hello", argv: &argv, envp: &[b"POLY_A64_STATS=1"], layout: &layout },
    )
    .unwrap();
    assert!(h.stats);
    assert_eq!(h.engine, EngineKind::Jit);
    // The program's __TEXT is readable and executable, its stack and TSD
    // page readable and writable, and nothing is writable and executable.
    let text = h.regions.iter().find(|r| r.addr == 0x1_0000_0000).unwrap();
    assert_eq!(text.prot, R | X);
    assert!(h.regions.iter().all(|r| r.prot & (W | X) != W | X));
    assert!(h.regions.iter().any(|r| r.addr == h.tpidrro && r.prot == R | W));
    // The start code is in the system area; sp is at the header address.
    assert!(h.pc >= layout.system && h.pc < layout.system_end);
    let mut word = [0u8; 8];
    mem.peek(h.sp, &mut word).unwrap();
    assert_eq!(u64::from_le_bytes(word), 0x1_0000_0000);
}

#[test]
fn segments_outside_the_window_are_refused() {
    let layout = Layout { library_base: 0x2_0000_0000, ..Layout::compact(0x1_0000_0000) };
    let opts = poly_dyld::Options { library_base: layout.library_base, ..poly_dyld::Options::ARM64 };
    let program = poly_dyld::link(&mut disk(), "/bin/a64-hello", &opts).unwrap();
    let mut mem = GuestMem::new(layout.window_lo, layout.window_size());
    let argv: [&[u8]; 1] = [b"/bin/a64-hello"];
    let e = load(
        &mut WindowTarget(&mut mem),
        &program,
        &Args { path: "/bin/a64-hello", argv: &argv, envp: &[], layout: &layout },
    )
    .unwrap_err();
    assert!(e.contains("outside the translated program's window"), "{e}");
}

#[test]
fn bridge_mmap_munmap_mprotect() {
    use poly_a64::Memory;
    let layout = Layout::compact(0x1_0000_0000);
    let h = crate::Handoff {
        window_lo: layout.window_lo,
        window_end: layout.window_end,
        mmap_lo: layout.mmap_lo,
        mmap_end: layout.mmap_end,
        ..Default::default()
    };
    let mut mem = GuestMem::new(layout.window_lo, layout.window_size());
    let mut b = Bridge::new(Console::default(), &h);
    let mut cpu = poly_a64::Cpu::new();
    let mut call = |b: &mut Bridge<Console>, mem: &mut GuestMem, n: u64, args: &[u64]| {
        cpu.x = [0; 31];
        cpu.x[16] = n;
        cpu.x[..args.len()].copy_from_slice(args);
        poly_a64_jit::jit::Kernel::svc(b, 0x80, &mut cpu, mem);
        (cpu.x[0], cpu.nzcv & poly_personality_xnu::arm64::NZCV_C != 0)
    };
    // Anonymous, one byte: a whole 16 KiB page, readable and writable.
    let (a, err) = call(&mut b, &mut mem, 197, &[0, 1, 3, 0x1002, u64::MAX, 0]);
    assert!(!err);
    assert_eq!(a, layout.mmap_lo);
    mem.write(a + 16383, &[7]).unwrap();
    // Length 0 is 0; a file mapping without a descriptor is EBADF (the
    // kernel's answer).
    assert_eq!(call(&mut b, &mut mem, 197, &[0, 0, 3, 0x1002, u64::MAX, 0]), (0, false));
    assert_eq!(call(&mut b, &mut mem, 197, &[0, 4096, 1, 2, u64::MAX, 0]), (9, true));
    // MAP_FIXED unaligned to 16 KiB is EINVAL; outside the window ENOMEM.
    assert_eq!(call(&mut b, &mut mem, 197, &[a + 4096, 4096, 3, 0x1012, u64::MAX, 0]), (22, true));
    assert_eq!(call(&mut b, &mut mem, 197, &[layout.window_end, 4096, 3, 0x1012, u64::MAX, 0]), (12, true));
    // munmap and mprotect outside the window are refused before the kernel.
    assert_eq!(call(&mut b, &mut mem, 73, &[layout.window_end, 16384]), (22, true));
    // mprotect to read-only: writes fault; munmap: reads fault.
    assert_eq!(call(&mut b, &mut mem, 74, &[a, 16384, 1]), (0, false));
    assert!(mem.write(a, &[1]).is_err());
    let mut byte = [0u8];
    mem.read(a + 16383, &mut byte).unwrap();
    assert_eq!(byte, [7]);
    assert_eq!(call(&mut b, &mut mem, 73, &[a, 16384]), (0, false));
    assert!(mem.read(a, &mut byte).is_err());
    // MAP_FIXED over the unmapped page: zero again.
    assert_eq!(call(&mut b, &mut mem, 197, &[a, 16384, 3, 0x1012, u64::MAX, 0]), (a, false));
    mem.read(a + 16383, &mut byte).unwrap();
    assert_eq!(byte, [0]);
    // Every call reached the host in the x86-64 convention, the memory
    // calls with guest pages: mmap at the anonymous area, MAP_FIXED.
    let (pid, err) = call(&mut b, &mut mem, 20, &[]);
    assert_eq!((pid, err), (1, false));
    assert_eq!(b.host.calls.last(), Some(&0x200_0014));
    assert_eq!(b.host.calls[0], 0x200_00c5);
    // exit stops the guest with its status.
    cpu.x[16] = 1;
    cpu.x[0] = 3;
    assert_eq!(poly_a64_jit::jit::Kernel::svc(&mut b, 0x80, &mut cpu, &mut mem), Some(3));
    assert_eq!(b.exit, Some(3));
    // Another svc immediate stops it too.
    assert_eq!(poly_a64_jit::jit::Kernel::svc(&mut b, 0x10, &mut cpu, &mut mem), Some(-1));
    assert_eq!(b.bad_svc, Some(0x10));
}

#[test]
fn a_stopped_guest_ends_with_its_signal() {
    use crate::run::sig;
    let layout = Layout::compact(0x1_0000_0000);
    let mut mem = GuestMem::new(layout.window_lo, layout.window_size());
    mem.set_mapped(layout.window_lo, 16384, R).unwrap();
    let fault = |addr, access| Stop::Fault(poly_a64::MemFault { addr, access, alignment: false });
    // Unmapped: SIGSEGV; mapped but not allowed: SIGBUS.
    assert_eq!(run::stop_signal(fault(8, poly_a64::Access::Read), &mem, None), sig::SIGSEGV);
    assert_eq!(run::stop_signal(fault(layout.window_lo, poly_a64::Access::Write), &mem, None), sig::SIGBUS);
    assert_eq!(run::stop_signal(Stop::Brk(1), &mem, None), sig::SIGTRAP);
    assert_eq!(run::stop_signal(Stop::Undefined(0), &mem, None), sig::SIGILL);
    assert_eq!(run::stop_signal(Stop::Exit(-1), &mem, Some(0x10)), sig::SIGSYS);
    let h = crate::Handoff::default();
    let mut b = Bridge::new(Console::default(), &h);
    b.kill_self(sig::SIGSEGV, &mut mem);
    assert_eq!(b.host.killed, Some(11));
    assert_eq!(b.host.calls, [0x200_0014, 0x200_0025]);
}

#[test]
fn a_crashing_program_stops_at_its_fault() {
    for kind in engines() {
        let r = run_program("/bin/a64-crash", &["/bin/a64-crash"], &[], kind);
        assert_eq!(lines(&r.out), ["a64-crash: about to read address 8"], "{kind:?}");
        assert_eq!(r.status, None);
        assert!(matches!(r.stop, Stop::Fault(f) if f.addr == 8), "{kind:?}: {:?}", r.stop);
    }
}
