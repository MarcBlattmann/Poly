//! What a process's own structures say, for the crash report (ADR 0155): the PEB, the loader's
//! lists (`PEB.Ldr`: `PEB_LDR_DATA` and the `LDR_DATA_TABLE_ENTRY` of each module), the process
//! parameters (`RTL_USER_PROCESS_PARAMETERS`) and `KUSER_SHARED_DATA` at 0x7FFE0000, read the way
//! a program reads them, straight from memory. Programs walk these without calling anything, so
//! an empty or missing field shows up as a fault at a small address (Chrome's `chrome_elf.dll`
//! faulted at 0x10).
//!
//! Layouts are the public x64 ones (`winternl.h`, `ntddk.h`/`wdm.h`, Learn's PEB, TEB and
//! `KUSER_SHARED_DATA` pages), the loader's are `poly_ldr::ldrdata`'s.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Where `KUSER_SHARED_DATA` is (`ntddk.h`: `MM_SHARED_USER_DATA_VA`).
pub const KUSER_SHARED_DATA: u64 = 0x7FFE_0000;

/// Reads `buf.len()` bytes at an address; `false` if they cannot be read.
pub type Read<'a> = &'a dyn Fn(u64, &mut [u8]) -> bool;

fn u64_at(r: Read<'_>, at: u64) -> Option<u64> {
    let mut b = [0u8; 8];
    r(at, &mut b).then(|| u64::from_le_bytes(b))
}

fn u32_at(r: Read<'_>, at: u64) -> Option<u32> {
    let mut b = [0u8; 4];
    r(at, &mut b).then(|| u32::from_le_bytes(b))
}

fn u16_at(r: Read<'_>, at: u64) -> Option<u16> {
    let mut b = [0u8; 2];
    r(at, &mut b).then(|| u16::from_le_bytes(b))
}

fn hex(v: Option<u64>) -> String {
    v.map_or_else(|| String::from("?"), |v| format!("{v:#x}"))
}

/// The `UNICODE_STRING` at `at`, as text (at most 120 characters).
pub fn unicode(r: Read<'_>, at: u64) -> String {
    let (Some(len), Some(buf)) = (u16_at(r, at), u64_at(r, at + 8)) else { return String::from("?") };
    if len == 0 || buf == 0 {
        return String::from("\"\"");
    }
    let n = usize::from(len / 2).min(120);
    let mut bytes = alloc::vec![0u8; n * 2];
    if !r(buf, &mut bytes) {
        return format!("<unreadable {buf:#x}>");
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    format!("{:?}", String::from_utf16_lossy(&units))
}

/// One list of `PEB_LDR_DATA` (`head` the `LIST_ENTRY`, `link_offset` where the entry's links are
/// in `LDR_DATA_TABLE_ENTRY`), as lines.
fn list(r: Read<'_>, name: &str, head: u64, link_offset: u64, out: &mut Vec<String>) {
    let (flink, blink) = (u64_at(r, head), u64_at(r, head + 8));
    out.push(format!("  {name}: head {head:#x} flink {} blink {}", hex(flink), hex(blink)));
    let mut at = flink.unwrap_or(0);
    let mut n = 0;
    while at != head && at != 0 && n < 64 {
        let entry = at - link_offset;
        let line = format!(
            "    [{n}] entry {entry:#x} base {} entry point {} size {} full {} base name {} links {} {}",
            hex(u64_at(r, entry + 0x30)),
            hex(u64_at(r, entry + 0x38)),
            hex(u32_at(r, entry + 0x40).map(u64::from)),
            unicode(r, entry + 0x48),
            unicode(r, entry + 0x58),
            hex(u64_at(r, at)),
            hex(u64_at(r, at + 8)),
        );
        out.push(line);
        match u64_at(r, at) {
            Some(next) => at = next,
            None => break,
        }
        n += 1;
    }
    if at == 0 {
        out.push(String::from("    (a link is NULL)"));
    }
}

/// The report: lines describing the PEB at `teb + 0x60`'s target.
pub fn describe(r: Read<'_>, teb: u64) -> Vec<String> {
    let mut out = Vec::new();
    let peb = u64_at(r, teb + 0x60).unwrap_or(0);
    out.push(format!(
        "peb {peb:#x}: image base {} ldr {} params {} heap {} api set map {} session {} os {}.{}.{} processors {}",
        hex(u64_at(r, peb + 0x10)),
        hex(u64_at(r, peb + 0x18)),
        hex(u64_at(r, peb + 0x20)),
        hex(u64_at(r, peb + 0x30)),
        hex(u64_at(r, peb + 0x68)),
        hex(u32_at(r, peb + 0x2C0).map(u64::from)),
        hex(u32_at(r, peb + 0x118).map(u64::from)),
        hex(u32_at(r, peb + 0x11C).map(u64::from)),
        hex(u16_at(r, peb + 0x120).map(u64::from)),
        hex(u32_at(r, peb + 0xB8).map(u64::from)),
    ));
    let ldr = u64_at(r, peb + 0x18).unwrap_or(0);
    if ldr == 0 {
        out.push(String::from("ldr: NULL"));
    } else {
        out.push(format!(
            "ldr {ldr:#x}: length {} initialized {}",
            hex(u32_at(r, ldr).map(u64::from)),
            hex(r_byte(r, ldr + 4).map(u64::from))
        ));
        list(r, "in load order", ldr + 0x10, 0x00, &mut out);
        list(r, "in memory order", ldr + 0x20, 0x10, &mut out);
        list(r, "in initialization order", ldr + 0x30, 0x20, &mut out);
    }
    let pp = u64_at(r, peb + 0x20).unwrap_or(0);
    if pp == 0 {
        out.push(String::from("process parameters: NULL"));
    } else {
        out.push(format!(
            "params {pp:#x}: length {} flags {} console {} console flags {} std in/out/err {} {} {} cwd {} cwd handle {} \
             dll path {} image {} command line {} environment {} size {}",
            hex(u32_at(r, pp + 4).map(u64::from)),
            hex(u32_at(r, pp + 8).map(u64::from)),
            hex(u64_at(r, pp + 0x10)),
            hex(u32_at(r, pp + 0x18).map(u64::from)),
            hex(u64_at(r, pp + 0x20)),
            hex(u64_at(r, pp + 0x28)),
            hex(u64_at(r, pp + 0x30)),
            unicode(r, pp + 0x38),
            hex(u64_at(r, pp + 0x48)),
            unicode(r, pp + 0x50),
            unicode(r, pp + 0x60),
            unicode(r, pp + 0x70),
            hex(u64_at(r, pp + 0x80)),
            hex(u64_at(r, pp + 0x3F0)),
        ));
    }
    let k = KUSER_SHARED_DATA;
    match u32_at(r, k) {
        None => out.push(format!("KUSER_SHARED_DATA {k:#x}: not mapped")),
        Some(tick) => out.push(format!(
            "KUSER_SHARED_DATA {k:#x}: tick count low {tick:#x} multiplier {} nt build {} product type {} major {} minor {} \
             suite {} processor features[0..4] {:?}",
            hex(u32_at(r, k + 4).map(u64::from)),
            hex(u32_at(r, k + 0x260).map(u64::from)),
            hex(u32_at(r, k + 0x264).map(u64::from)),
            hex(u32_at(r, k + 0x26C).map(u64::from)),
            hex(u32_at(r, k + 0x270).map(u64::from)),
            hex(u32_at(r, k + 0x2D0).map(u64::from)),
            (0..4).map(|i| r_byte(r, k + 0x274 + i).unwrap_or(0xFF)).collect::<Vec<u8>>(),
        )),
    }
    out
}

/// The base address of every module on `PEB.Ldr`'s load-order list (at most 256), the process's
/// own image first: what a stack walk needs to find each module's unwind data.
pub fn module_bases(r: Read<'_>, teb: u64) -> Vec<u64> {
    let mut out = Vec::new();
    let peb = u64_at(r, teb + 0x60).unwrap_or(0);
    let ldr = u64_at(r, peb + 0x18).unwrap_or(0);
    if ldr == 0 {
        return out;
    }
    let head = ldr + 0x10;
    let mut at = u64_at(r, head).unwrap_or(0);
    while at != head && at != 0 && out.len() < 256 {
        if let Some(base) = u64_at(r, at + 0x30).filter(|&b| b != 0) {
            out.push(base);
        }
        at = u64_at(r, at).unwrap_or(0);
    }
    out
}

/// The kernel argument that logs every exception delivered to user mode (ADR 0155 step 9):
/// `poly.nt.exceptions`, or `poly.nt.exceptions=N` to dump N quadwords of the stack.
pub const EXCEPTIONS_FLAG: &str = "poly.nt.exceptions";
/// Stack quadwords dumped when the argument names no number.
pub const EXCEPTION_STACK_DEFAULT: usize = 256;
/// The most stack quadwords one exception may dump (32 KiB of stack).
pub const EXCEPTION_STACK_MAX: usize = 4096;

/// How many stack quadwords `poly.nt.exceptions[=N]` dumps: N, at least 16 and at most
/// [`EXCEPTION_STACK_MAX`]; the default for no value or one that is not a number.
pub fn exception_stack_words(value: Option<&str>) -> usize {
    value.and_then(|v| v.parse::<usize>().ok()).map_or(EXCEPTION_STACK_DEFAULT, |n| n.clamp(16, EXCEPTION_STACK_MAX))
}

/// A module of the process: where it is mapped and its base name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleSpan {
    pub base: u64,
    pub size: u64,
    pub name: String,
}

/// Every module on `PEB.Ldr`'s load-order list (at most 256) with its size and base name.
pub fn modules(r: Read<'_>, teb: u64) -> Vec<ModuleSpan> {
    let mut out = Vec::new();
    let peb = u64_at(r, teb + 0x60).unwrap_or(0);
    let ldr = u64_at(r, peb + 0x18).unwrap_or(0);
    if ldr == 0 {
        return out;
    }
    let head = ldr + 0x10;
    let mut at = u64_at(r, head).unwrap_or(0);
    while at != head && at != 0 && out.len() < 256 {
        if let Some(base) = u64_at(r, at + 0x30).filter(|&b| b != 0) {
            let size = u64::from(u32_at(r, at + 0x40).unwrap_or(0));
            let name = unicode(r, at + 0x58).trim_matches('"').into();
            out.push(ModuleSpan { base, size, name });
        }
        at = u64_at(r, at).unwrap_or(0);
    }
    out
}

/// `address` as `module+0xoffset` if a module holds it.
pub fn locate(modules: &[ModuleSpan], address: u64) -> Option<String> {
    modules
        .iter()
        .find(|m| address >= m.base && address - m.base < m.size)
        .map(|m| format!("{}+{:#x}", m.name, address - m.base))
}

/// Where and what an exception was when it was raised.
#[derive(Clone, Copy, Debug)]
pub struct Raised {
    pub pid: u32,
    pub tid: u64,
    pub code: u32,
    pub address: u64,
}

/// The most code addresses and pieces of text one report lists.
const REPORT_ITEMS: usize = 48;
/// The longest piece of text shown.
const TEXT_MAX: usize = 96;

/// Printable text of at least 5 characters in the stack: ASCII (bytes) and UTF-16, with the
/// offset from the stack pointer where each starts; a run of 3 or 4 decimal digits counts too.
/// Programs leave the reason they stopped as text on the stack (Chrome's failed delay load: the
/// DLL's name and the error number, "126", "127" or "182").
fn stack_text(bytes: &[u8]) -> Vec<(usize, String)> {
    let printable = |b: u8| (0x20..0x7F).contains(&b);
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() && found.len() < REPORT_ITEMS {
        // UTF-16: printable bytes with zeros between them.
        let mut wide = String::new();
        let mut j = i;
        while j + 1 < bytes.len() && printable(bytes[j]) && bytes[j + 1] == 0 && wide.len() < TEXT_MAX {
            wide.push(char::from(bytes[j]));
            j += 2;
        }
        if wide.len() >= 5 {
            found.push((i, format!("L{wide:?}")));
            i = j;
            continue;
        }
        let mut text = String::new();
        let mut j = i;
        while j < bytes.len() && printable(bytes[j]) && text.len() < TEXT_MAX {
            text.push(char::from(bytes[j]));
            j += 1;
        }
        if text.len() >= 5 || (text.len() >= 3 && text.bytes().all(|b| b.is_ascii_digit())) {
            found.push((i, format!("{text:?}")));
            i = j;
        } else {
            i += 1;
        }
    }
    found
}

/// The lines `poly.nt.exceptions` logs for one exception: the code and where it was raised as
/// `module+offset`, its parameters, the general registers (in `CONTEXT` order), the stack as
/// quadwords (four to a line, runs of zero lines folded), the stack values that point into a
/// module (return addresses, most likely) and the text found on the stack.
pub fn exception_report(
    at: &Raised,
    parameters: &[u64],
    registers: &[u64; 16],
    stack: &[u64],
    modules: &[ModuleSpan],
) -> Vec<String> {
    const NAMES: [&str; 16] =
        ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"];
    let head = format!("nt-exc: pid {} tid {}", at.pid, at.tid);
    let mut out = Vec::new();
    out.push(format!(
        "{head} code {:#010x} at {} ({:#x}) params {parameters:x?}",
        at.code,
        locate(modules, at.address).unwrap_or_else(|| String::from("?")),
        at.address
    ));
    let regs: Vec<String> = NAMES.iter().zip(registers).map(|(n, v)| format!("{n}={v:x}")).collect();
    out.push(format!("{head} regs {}", regs.join(" ")));
    let mut zeros = 0usize;
    for (i, line) in stack.chunks(4).enumerate() {
        if line.iter().all(|&w| w == 0) {
            zeros += 1;
            continue;
        }
        if zeros != 0 {
            out.push(format!("{head} stack ... {zeros} lines of zeros"));
            zeros = 0;
        }
        let words: Vec<String> = line.iter().map(|w| format!("{w:016x}")).collect();
        out.push(format!("{head} stack +{:#06x}: {}", i * 32, words.join(" ")));
    }
    if zeros != 0 {
        out.push(format!("{head} stack ... {zeros} lines of zeros"));
    }
    let code: Vec<String> = stack
        .iter()
        .enumerate()
        .filter_map(|(i, &w)| locate(modules, w).map(|m| format!("+{:#x} {m}", i * 8)))
        .take(REPORT_ITEMS)
        .collect();
    if !code.is_empty() {
        out.push(format!("{head} code addresses on the stack: {}", code.join(", ")));
    }
    let bytes: Vec<u8> = stack.iter().flat_map(|w| w.to_le_bytes()).collect();
    let text: Vec<String> = stack_text(&bytes).into_iter().map(|(o, t)| format!("+{o:#x} {t}")).collect();
    if !text.is_empty() {
        out.push(format!("{head} text on the stack: {}", text.join(", ")));
    }
    out
}

fn r_byte(r: Read<'_>, at: u64) -> Option<u8> {
    let mut b = [0u8; 1];
    r(at, &mut b).then_some(b[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    /// A sparse memory of bytes.
    struct Mem(BTreeMap<u64, u8>);

    impl Mem {
        fn new() -> Mem {
            Mem(BTreeMap::new())
        }
        fn put(&mut self, at: u64, bytes: &[u8]) {
            for (i, b) in bytes.iter().enumerate() {
                self.0.insert(at + i as u64, *b);
            }
        }
        fn q(&mut self, at: u64, v: u64) {
            self.put(at, &v.to_le_bytes());
        }
        fn string(&mut self, at: u64, buf: u64, s: &str) {
            let units: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
            self.put(buf, &units);
            self.put(at, &(units.len() as u16).to_le_bytes());
            self.put(at + 2, &(units.len() as u16).to_le_bytes());
            self.q(at + 8, buf);
        }
        fn reader(&self) -> impl Fn(u64, &mut [u8]) -> bool + '_ {
            move |at, buf| {
                for (i, b) in buf.iter_mut().enumerate() {
                    match self.0.get(&(at + i as u64)) {
                        Some(v) => *b = *v,
                        None => return false,
                    }
                }
                true
            }
        }
    }

    #[test]
    fn the_exception_option_bounds_the_stack_dump() {
        assert_eq!(exception_stack_words(None), EXCEPTION_STACK_DEFAULT);
        assert_eq!(exception_stack_words(Some("1536")), 1536);
        assert_eq!(exception_stack_words(Some("1")), 16);
        assert_eq!(exception_stack_words(Some("1000000")), EXCEPTION_STACK_MAX);
        assert_eq!(exception_stack_words(Some("all")), EXCEPTION_STACK_DEFAULT);
    }

    #[test]
    fn modules_are_read_with_size_and_name_and_locate_an_address() {
        let mut m = Mem::new();
        let (teb, peb, ldr) = (0x1000, 0x2000, 0x3000);
        m.q(teb + 0x60, peb);
        m.q(peb + 0x18, ldr);
        let (a, b) = (0x4000u64, 0x6000u64);
        m.q(a + 0x30, 0x1_4000_0000);
        m.put(a + 0x40, &0x45_0000u32.to_le_bytes());
        m.string(a + 0x58, 0x5100, "chrome.exe");
        m.q(b + 0x30, 0x7ff8_0000_0000);
        m.put(b + 0x40, &0x1300_0000u32.to_le_bytes());
        m.string(b + 0x58, 0x5200, "chrome.dll");
        // The load-order list: head -> a -> b -> head.
        m.q(ldr + 0x10, a);
        m.q(a, b);
        m.q(b, ldr + 0x10);
        let mods = modules(&m.reader(), teb);
        assert_eq!(
            mods,
            [
                ModuleSpan { base: 0x1_4000_0000, size: 0x45_0000, name: "chrome.exe".into() },
                ModuleSpan { base: 0x7ff8_0000_0000, size: 0x1300_0000, name: "chrome.dll".into() },
            ]
        );
        assert_eq!(locate(&mods, 0x7ff8_08e8_4bbf).as_deref(), Some("chrome.dll+0x8e84bbf"));
        assert_eq!(locate(&mods, 0x1_4000_0000).as_deref(), Some("chrome.exe+0x0"));
        // One past the end of the image, and an address in no module.
        assert_eq!(locate(&mods, 0x1_4045_0000), None);
        assert_eq!(locate(&mods, 0x10), None);
        // No loader data: no modules.
        assert!(modules(&Mem::new().reader(), teb).is_empty());
    }

    #[test]
    fn an_exception_report_names_the_module_the_stack_and_its_text() {
        let mods = [ModuleSpan { base: 0x7ff8_0000_0000, size: 0x1300_0000, name: "chrome.dll".into() }];
        let at = Raised { pid: 7, tid: 9, code: 0x8000_0003, address: 0x7ff8_08e8_4bbf };
        let mut regs = [0u64; 16];
        regs[0] = 0x7e;
        regs[4] = 0x50_f000;
        // The stack: a return address, eight zero quadwords, "SHLWAPI.dll" as bytes, then L"126 x".
        let mut stack = alloc::vec![0x7ff8_0000_1234u64, 0, u64::from_le_bytes(*b"126\0\0\0\0\0"), 1];
        stack.extend([0u64; 8]);
        stack.push(u64::from_le_bytes(*b"SHLWAPI."));
        stack.push(u64::from_le_bytes(*b"dll\0\0\0\0\0"));
        let wide: Vec<u8> = "126 x".encode_utf16().flat_map(u16::to_le_bytes).chain([0; 6]).collect();
        stack.extend(wide.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())));
        let lines = exception_report(&at, &[1, 2], &regs, &stack, &mods);
        assert_eq!(
            lines[0],
            "nt-exc: pid 7 tid 9 code 0x80000003 at chrome.dll+0x8e84bbf (0x7ff808e84bbf) params [1, 2]"
        );
        assert!(lines[1].starts_with("nt-exc: pid 7 tid 9 regs rax=7e rcx=0 "), "{}", lines[1]);
        assert!(lines[1].contains(" rsp=50f000 "));
        assert_eq!(
            lines[2],
            "nt-exc: pid 7 tid 9 stack +0x0000: 00007ff800001234 0000000000000000 0000000000363231 0000000000000001"
        );
        assert_eq!(lines[3], "nt-exc: pid 7 tid 9 stack ... 2 lines of zeros");
        assert!(lines[4].starts_with("nt-exc: pid 7 tid 9 stack +0x0060: "), "{}", lines[4]);
        assert!(
            lines.contains(&String::from("nt-exc: pid 7 tid 9 code addresses on the stack: +0x0 chrome.dll+0x1234"))
        );
        let text = lines.last().unwrap();
        assert_eq!(
            text,
            "nt-exc: pid 7 tid 9 text on the stack: +0x10 \"126\", +0x60 \"SHLWAPI.dll\", +0x70 L\"126 x\""
        );
        // An address in no module, no stack: two lines.
        let none = exception_report(&Raised { address: 0x10, ..at }, &[], &regs, &[], &mods);
        assert_eq!(none.len(), 2);
        assert!(none[0].contains(" at ? (0x10) params []"));
    }

    #[test]
    fn a_missing_loader_data_is_said_so() {
        let mut m = Mem::new();
        m.q(0x1000 + 0x60, 0x2000);
        m.q(0x2000 + 0x18, 0);
        let lines = describe(&m.reader(), 0x1000);
        assert!(lines.iter().any(|l| l == "ldr: NULL"), "{lines:?}");
        assert!(lines.iter().any(|l| l == "process parameters: NULL"));
        assert!(lines.last().unwrap().contains("not mapped"));
    }

    #[test]
    fn the_lists_are_walked_with_names() {
        let mut m = Mem::new();
        let (teb, peb, ldr) = (0x1000, 0x2000, 0x3000);
        m.q(teb + 0x60, peb);
        m.q(peb + 0x18, ldr);
        // One module (an entry at 0x4000) on all three lists.
        let e = 0x4000u64;
        m.q(e + 0x30, 0x1_4000_0000);
        m.string(e + 0x48, 0x5000, "C:\\p\\chrome.exe");
        m.string(e + 0x58, 0x5100, "chrome.exe");
        for (head, off) in [(ldr + 0x10, 0u64), (ldr + 0x20, 0x10), (ldr + 0x30, 0x20)] {
            m.q(head, e + off);
            m.q(head + 8, e + off);
            m.q(e + off, head);
            m.q(e + off + 8, head);
        }
        let lines = describe(&m.reader(), teb);
        let walked: Vec<&String> = lines.iter().filter(|l| l.contains("base name \"chrome.exe\"")).collect();
        assert_eq!(walked.len(), 3, "{lines:?}");
        assert!(walked[0].contains("base 0x140000000"));
        assert!(!lines.iter().any(|l| l.contains("a link is NULL")));
    }

    #[test]
    fn the_module_bases_follow_the_load_order() {
        let mut m = Mem::new();
        let (teb, peb, ldr) = (0x1000, 0x2000, 0x3000);
        m.q(teb + 0x60, peb);
        m.q(peb + 0x18, ldr);
        let head = ldr + 0x10;
        let (a, b) = (0x4000u64, 0x4100u64);
        m.q(a + 0x30, 0x1_4000_0000);
        m.q(b + 0x30, 0x7ff0_0000_0000);
        m.q(head, a);
        m.q(a, b);
        m.q(b, head);
        assert_eq!(module_bases(&m.reader(), teb), [0x1_4000_0000, 0x7ff0_0000_0000]);
        // No loader data: none.
        m.q(peb + 0x18, 0);
        assert!(module_bases(&m.reader(), teb).is_empty());
    }

    #[test]
    fn a_null_link_is_reported_not_followed() {
        let mut m = Mem::new();
        m.q(0x1000 + 0x60, 0x2000);
        m.q(0x2000 + 0x18, 0x3000);
        m.q(0x3000 + 0x10, 0);
        m.q(0x3000 + 0x18, 0);
        let lines = describe(&m.reader(), 0x1000);
        assert!(lines.iter().any(|l| l.contains("a link is NULL")), "{lines:?}");
    }
}
