//! Real executables from the host toolchain, checked against `readelf`.
//!
//! The tests compile tiny C programs with the host C compiler (`$CC`, else
//! `cc`) into `CARGO_TARGET_TMPDIR`: static ones without a C library, static
//! and static-PIE ones with the host's C library, and a dynamically linked
//! one. Each is parsed and planned, compared field by field with what
//! `readelf -lW` reports, and its plan is checked by reading every page back:
//! file bytes where the segment has them, zeros after. A variant the host
//! cannot build (no compiler, no static C library) is skipped with a note on
//! stderr, so the suite still runs on a bare machine; `readelf` is likewise
//! optional.

use poly_elf::*;
use std::path::PathBuf;
use std::process::Command;

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("poly-elf");
    std::fs::create_dir_all(&dir).expect("create the test output directory");
    dir
}

/// A static program without a C library: touches `.data` and `.bss`, then
/// calls `exit(0)` directly.
const NOLIBC: &str = r#"
char bss[100000];
volatile long data = 42;
void _start(void) {
    bss[sizeof bss - 1] = (char)data;
    __asm__ volatile("mov $60, %%eax\n\txor %%edi, %%edi\n\tsyscall" ::: "memory");
    for (;;) {}
}
"#;

/// A C library program with initialised data, a large `.bss` and TLS.
const HELLO: &str = r#"
#include <stdio.h>
char big[1 << 20];
static int counter = 7;
static __thread int per_thread = 3;
int main(int argc, char **argv) {
    big[argc] = (char)counter;
    printf("hello %s %d %d\n", argv[0], big[argc], per_thread);
    return 0;
}
"#;

/// Compiles `src` with `flags`; `None` (and a note) if the host cannot.
fn build(name: &str, src: &str, flags: &[&str]) -> Option<Vec<u8>> {
    let dir = out_dir();
    let c = dir.join(format!("{name}.c"));
    let exe = dir.join(name);
    std::fs::write(&c, src).expect("write the test program");
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let status = Command::new(&cc).args(flags).arg("-o").arg(&exe).arg(&c).output();
    match status {
        Ok(out) if out.status.success() => Some(std::fs::read(&exe).expect("read the compiled program")),
        Ok(out) => {
            eprintln!("skipping {name}: {cc} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
            None
        }
        Err(e) => {
            eprintln!("skipping {name}: cannot run {cc}: {e}");
            None
        }
    }
}

const NOLIBC_FLAGS: &[&str] = &["-nostdlib", "-fno-stack-protector", "-O1"];

/// What `readelf -lW` says about a file.
#[derive(Debug, Default)]
struct Readelf {
    is_dyn: bool,
    entry: u64,
    interp: Option<String>,
    /// (type name, offset, vaddr, filesz, memsz, flags, align)
    phdrs: Vec<(String, u64, u64, u64, u64, u32, u64)>,
}

fn hex(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|_| panic!("not hex: {s}"))
}

fn readelf(name: &str) -> Option<Readelf> {
    let out = match Command::new("readelf").arg("-lW").arg(out_dir().join(name)).output() {
        Ok(out) if out.status.success() => out,
        _ => {
            eprintln!("readelf not available; not comparing {name}");
            return None;
        }
    };
    let text = String::from_utf8(out.stdout).expect("readelf prints UTF-8");
    let mut r = Readelf::default();
    let mut in_table = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(t) = line.strip_prefix("Elf file type is ") {
            r.is_dyn = t.starts_with("DYN");
        } else if let Some(e) = line.strip_prefix("Entry point ") {
            r.entry = hex(e);
        } else if let Some(p) = line.strip_prefix("[Requesting program interpreter: ") {
            r.interp = Some(p.trim_end_matches(']').to_string());
        } else if line.starts_with("Type") && line.contains("Offset") {
            in_table = true;
        } else if line.starts_with("Section to Segment") {
            in_table = false;
        } else if in_table && !line.is_empty() {
            // Type Offset VirtAddr PhysAddr FileSiz MemSiz Flg... Align; Flg may contain spaces ("R E").
            let f: Vec<&str> = line.split_whitespace().collect();
            let flags = f[6..f.len() - 1].concat();
            let pf = |c, bit| if flags.contains(c) { bit } else { 0 };
            let flags = pf('R', PF_R) | pf('W', PF_W) | pf('E', PF_X);
            r.phdrs.push((f[0].to_string(), hex(f[1]), hex(f[2]), hex(f[4]), hex(f[5]), flags, hex(f[f.len() - 1])));
        }
    }
    Some(r)
}

fn type_name(t: u32) -> Option<&'static str> {
    Some(match t {
        PT_LOAD => "LOAD",
        PT_DYNAMIC => "DYNAMIC",
        PT_INTERP => "INTERP",
        PT_NOTE => "NOTE",
        PT_PHDR => "PHDR",
        PT_TLS => "TLS",
        PT_GNU_STACK => "GNU_STACK",
        PT_GNU_RELRO => "GNU_RELRO",
        _ => return None,
    })
}

/// Compares the parsed headers with `readelf`'s view.
fn compare_with_readelf(name: &str, elf: &Elf<'_>) {
    let Some(r) = readelf(name) else { return };
    assert_eq!(r.is_dyn, elf.elf_type() == ElfType::Dyn, "{name}: type");
    assert_eq!(r.entry, elf.entry(), "{name}: entry");
    assert_eq!(r.interp.as_deref().map(str::as_bytes), elf.interp(), "{name}: interpreter");
    let ours: Vec<_> = elf.program_headers().collect();
    assert_eq!(r.phdrs.len(), ours.len(), "{name}: program header count");
    for (theirs, ph) in r.phdrs.iter().zip(&ours) {
        if let Some(t) = type_name(ph.p_type) {
            assert_eq!(theirs.0, t, "{name}: type of {ph:?}");
        }
        let mine = (ph.offset, ph.vaddr, ph.filesz, ph.memsz, ph.flags & (PF_R | PF_W | PF_X), ph.align);
        assert_eq!((theirs.1, theirs.2, theirs.3, theirs.4, theirs.5, theirs.6), mine, "{name}: {}", theirs.0);
    }
}

/// Reads the byte at `addr` of the planned address space.
fn byte_at(plan: &LoadPlan, file: &[u8], addr: u64, cache: &mut (u64, Vec<u8>)) -> Option<u8> {
    let page = addr & !(PAGE_SIZE - 1);
    if cache.0 != page || cache.1.is_empty() {
        cache.1 = vec![0; PAGE_SIZE as usize];
        if !plan.mapping_at(page)?.fill_page(file, page, &mut cache.1) {
            return None;
        }
        cache.0 = page;
    }
    Some(cache.1[(addr - page) as usize])
}

/// Checks the plan against the segments: every file byte of a segment is at
/// its relocated address, the rest of the segment is zero, protections
/// match, and AT_PHDR points at a copy of the program header table.
fn check_plan(name: &str, elf: &Elf<'_>, plan: &LoadPlan) {
    let file = elf.data();
    let bias = plan.bias();
    let mut cache = (u64::MAX, Vec::new());
    for ph in elf.load_segments().filter(|ph| ph.memsz != 0) {
        let start = ph.vaddr + bias;
        let m = plan.mapping_at(start).unwrap_or_else(|| panic!("{name}: {start:#x} not mapped"));
        assert_eq!(m.prot, Prot::from_flags(ph.flags), "{name}: protection at {start:#x}");
        let seg = &file[ph.offset as usize..(ph.offset + ph.filesz) as usize];
        for (i, &want) in seg.iter().enumerate() {
            assert_eq!(byte_at(plan, file, start + i as u64, &mut cache), Some(want), "{name}: file byte {i}");
        }
        for a in start + ph.filesz..start + ph.memsz {
            assert_eq!(byte_at(plan, file, a, &mut cache), Some(0), "{name}: bss byte at {a:#x}");
        }
    }
    let phdr = plan.phdr().expect("the program header table is mapped");
    let table = elf.program_header_table();
    let mapped: Vec<u8> =
        (0..table.len() as u64).map(|i| byte_at(plan, file, phdr + i, &mut cache).expect("phdr byte")).collect();
    assert_eq!(mapped, table, "{name}: AT_PHDR");
    assert!(plan.mapping_at(plan.entry()).expect("entry is mapped").prot.executable(), "{name}: entry");
    let last = elf.load_segments().last().unwrap();
    assert_eq!(plan.brk(), (last.vaddr + last.memsz + bias).next_multiple_of(PAGE_SIZE), "{name}: brk");
}

/// Parses, plans, compares and builds a stack for one compiled program.
fn check(name: &str, bytes: &[u8]) -> LoadPlan {
    let elf = Elf::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    compare_with_readelf(name, &elf);
    let plan = LoadPlan::new(&elf, &LoadOptions::default()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    check_plan(name, &elf, &plan);

    let top = 0x7fff_ffff_f000;
    let mut stack = vec![0u8; 8192];
    let argv: &[&[u8]] = &[name.as_bytes(), b"one"];
    let p = StackParams {
        argv,
        envp: &[b"PATH=/bin"],
        execfn: name.as_bytes(),
        platform: PLATFORM_X86_64,
        random: &[7; 16],
        aux: AuxInfo::new(&plan, None, Credentials::default()),
        phdr_table: elf.program_header_table(),
        extra_aux: &[],
    };
    let l = build_initial_stack(&mut stack, top, &p).unwrap();
    let s = InitialStack::parse(&stack, top, l.rsp).unwrap();
    assert_eq!(l.rsp % 16, 0);
    assert_eq!(s.arg(0), Some(name.as_bytes()));
    assert_eq!(s.aux(AT_PHDR), plan.phdr());
    assert_eq!(s.aux(AT_PHNUM), Some(u64::from(elf.phnum())));
    assert_eq!(s.aux(AT_ENTRY), Some(plan.entry()));
    plan
}

#[test]
fn static_without_libc() {
    let Some(b) = build("nolibc-static", NOLIBC, &[NOLIBC_FLAGS, &["-static", "-no-pie"]].concat()) else { return };
    let plan = check("nolibc-static", &b);
    let elf = Elf::parse(&b).unwrap();
    assert_eq!(elf.elf_type(), ElfType::Exec);
    assert!(elf.is_static());
    assert_eq!(plan.bias(), 0);
    // 100000 bytes of .bss reach past the file-backed pages.
    assert!(plan.mappings().iter().any(|m| m.file.is_none() && m.prot.writable()));
}

#[test]
fn static_pie_without_libc() {
    let Some(b) = build("nolibc-static-pie", NOLIBC, &[NOLIBC_FLAGS, &["-static-pie", "-fPIE"]].concat()) else {
        return;
    };
    let plan = check("nolibc-static-pie", &b);
    let elf = Elf::parse(&b).unwrap();
    assert_eq!(elf.elf_type(), ElfType::Dyn);
    assert!(elf.is_static());
    assert!(elf.dynamic().is_some());
    assert_eq!(plan.bias(), DEFAULT_DYN_BASE);
    assert_eq!(plan.entry(), DEFAULT_DYN_BASE + elf.entry());
}

#[test]
fn executable_stack_is_reported() {
    let flags = [NOLIBC_FLAGS, &["-static", "-no-pie", "-Wl,-z,execstack"]].concat();
    let Some(b) = build("nolibc-execstack", NOLIBC, &flags) else { return };
    let plan = check("nolibc-execstack", &b);
    assert!(plan.stack_executable());
    assert!(plan.warnings().contains(Warnings::EXECUTABLE_STACK));
}

#[test]
fn large_page_alignment_raises_the_base() {
    let flags = [NOLIBC_FLAGS, &["-static-pie", "-fPIE", "-Wl,-z,max-page-size=0x200000"]].concat();
    let Some(b) = build("nolibc-2m", NOLIBC, &flags) else { return };
    let plan = check("nolibc-2m", &b);
    let elf = Elf::parse(&b).unwrap();
    let align = elf.load_segments().map(|p| p.align).max().unwrap();
    assert_eq!(align, 0x20_0000);
    assert_eq!(plan.bias() % align, 0);
}

#[test]
fn static_with_libc() {
    let Some(b) = build("hello-static", HELLO, &["-static", "-no-pie", "-O1"]) else { return };
    let plan = check("hello-static", &b);
    let elf = Elf::parse(&b).unwrap();
    assert!(elf.is_static());
    assert!(elf.tls().is_some(), "a __thread variable gives a PT_TLS segment");
    assert!(plan.tls().is_some());
    assert!(plan.warnings().is_empty(), "{:?}", plan.warnings());
    // The 1 MiB .bss is anonymous memory and ends at the initial break.
    let bss = plan.mappings().last().unwrap();
    assert!(bss.file.is_none() && bss.size() >= 1 << 20);
    assert_eq!(plan.brk(), bss.end);
}

#[test]
fn static_pie_with_libc() {
    let Some(b) = build("hello-static-pie", HELLO, &["-static-pie", "-O1"]) else { return };
    let plan = check("hello-static-pie", &b);
    let elf = Elf::parse(&b).unwrap();
    assert_eq!(elf.elf_type(), ElfType::Dyn);
    assert!(elf.is_static() && elf.dynamic().is_some());
    assert_eq!(plan.start(), DEFAULT_DYN_BASE);
}

#[test]
fn dynamic_executable_names_its_interpreter() {
    let Some(b) = build("hello-dynamic", HELLO, &["-O1"]) else { return };
    let elf = Elf::parse(&b).unwrap();
    compare_with_readelf("hello-dynamic", &elf);
    let interp = elf.interp().expect("a dynamically linked program has PT_INTERP");
    assert!(interp.starts_with(b"/"), "{}", String::from_utf8_lossy(interp));
    let plan = LoadPlan::new(&elf, &LoadOptions::default()).unwrap();
    check_plan("hello-dynamic", &elf, &plan);
}

/// The test binary itself: whatever rustc and the linker made of it.
#[test]
fn this_test_binary() {
    let path = std::env::current_exe().unwrap();
    let b = std::fs::read(&path).unwrap();
    let elf = Elf::parse(&b).unwrap();
    let plan = LoadPlan::new(&elf, &LoadOptions::default()).unwrap();
    check_plan("test binary", &elf, &plan);
}

/// Damaged copies of real binaries never panic the parser or the planner.
#[test]
fn mutated_real_binaries_never_panic() {
    let Some(b) = build("nolibc-fuzz", NOLIBC, &[NOLIBC_FLAGS, &["-static", "-no-pie"]].concat()) else { return };
    let elf = Elf::parse(&b).unwrap();
    let headers = 64 + 56 * usize::from(elf.phnum());
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut page = vec![0u8; PAGE_SIZE as usize];
    for _ in 0..5000 {
        let mut m = b.clone();
        for _ in 0..1 + next() % 4 {
            let off = (next() % headers as u64) as usize;
            m[off] = next() as u8;
        }
        if next() % 10 == 0 {
            m.truncate((next() % m.len() as u64) as usize);
        }
        let Ok(elf) = Elf::parse(&m) else { continue };
        let Ok(plan) = LoadPlan::new(&elf, &LoadOptions::default()) else { continue };
        for map in plan.mappings() {
            assert!(map.fill_page(&m, map.start, &mut page));
            assert!(map.fill_page(&m, map.end - PAGE_SIZE, &mut page));
        }
    }
}
