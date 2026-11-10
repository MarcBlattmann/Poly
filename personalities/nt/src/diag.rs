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
