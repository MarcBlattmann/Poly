//! Kernel symbol lookup for backtraces.
//!
//! Reads the function symbols from the kernel ELF's `.symtab` (a small
//! hand-written ELF64 reader; the kernel is always little-endian x86-64 for
//! now), demangles them, and rewrites the panic handler's backtrace lines:
//!
//! ```text
//!   #0 0xffffffff80106a8c   ->   #0 0xffffffff80106a8c poly_kernel::selftest::trigger_page_fault+0x1c
//! ```

use std::path::Path;

/// Function symbols sorted by address, without overlaps.
#[derive(Debug)]
pub struct Symbols {
    syms: Vec<Symbol>,
}

#[derive(Debug)]
struct Symbol {
    addr: u64,
    /// Never 0: labels without a size extend to the next symbol.
    size: u64,
    name: String,
}

const SHT_SYMTAB: u32 = 2;
const SHF_EXECINSTR: u64 = 0x4;
const STT_NOTYPE: u8 = 0;
const STT_FUNC: u8 = 2;
const STB_GLOBAL: u8 = 1;
const SHN_LORESERVE: u16 = 0xff00;

fn bytes<const N: usize>(data: &[u8], off: u64) -> Result<[u8; N], String> {
    usize::try_from(off)
        .ok()
        .and_then(|o| data.get(o..o.checked_add(N)?))
        .map(|b| b.try_into().unwrap())
        .ok_or_else(|| format!("ELF truncated at offset {off:#x}"))
}

fn u16_at(data: &[u8], off: u64) -> Result<u16, String> {
    bytes(data, off).map(u16::from_le_bytes)
}

fn u32_at(data: &[u8], off: u64) -> Result<u32, String> {
    bytes(data, off).map(u32::from_le_bytes)
}

fn u64_at(data: &[u8], off: u64) -> Result<u64, String> {
    bytes(data, off).map(u64::from_le_bytes)
}

struct Section {
    ty: u32,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    link: u32,
}

impl Symbols {
    pub fn from_file(path: &Path) -> Result<Symbols, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Symbols::parse(&data).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parse an ELF64 little-endian image. Layouts are from the System V
    /// ABI (`Elf64_Ehdr`, `Elf64_Shdr`, `Elf64_Sym`).
    pub fn parse(data: &[u8]) -> Result<Symbols, String> {
        if data.get(..6) != Some(&[0x7f, b'E', b'L', b'F', 2, 1]) {
            return Err("not a little-endian ELF64 file".into());
        }
        let shoff = u64_at(data, 0x28)?;
        let shentsize = u64::from(u16_at(data, 0x3a)?);
        let shnum = u16_at(data, 0x3c)?;
        let sections = (0..u64::from(shnum))
            .map(|i| {
                let h = shoff + i * shentsize;
                Ok(Section {
                    ty: u32_at(data, h + 4)?,
                    flags: u64_at(data, h + 8)?,
                    addr: u64_at(data, h + 16)?,
                    offset: u64_at(data, h + 24)?,
                    size: u64_at(data, h + 32)?,
                    link: u32_at(data, h + 40)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let symtab = sections.iter().find(|s| s.ty == SHT_SYMTAB).ok_or("no .symtab (stripped binary?)")?;
        let strtab = sections.get(symtab.link as usize).ok_or("bad .symtab string table link")?;

        // (addr, size, rank, name); rank orders aliases at the same address.
        let mut raw = Vec::new();
        for i in 0..symtab.size / 24 {
            let s = symtab.offset + i * 24;
            let (name, info, shndx) = (u32_at(data, s)?, bytes::<1>(data, s + 4)?[0], u16_at(data, s + 6)?);
            let (value, size) = (u64_at(data, s + 8)?, u64_at(data, s + 16)?);
            let ty = info & 0xf;
            if !(ty == STT_FUNC || ty == STT_NOTYPE) || shndx == 0 || shndx >= SHN_LORESERVE {
                continue;
            }
            let Some(sec) = sections.get(usize::from(shndx)) else { continue };
            if sec.flags & SHF_EXECINSTR == 0 || name == 0 {
                continue;
            }
            let name = c_str(data, strtab.offset + u64::from(name))?;
            // Size-less labels (assembly) may extend at most to the end of their section.
            let limit = sec.addr + sec.size;
            let rank = match (ty, info >> 4) {
                (STT_FUNC, STB_GLOBAL) => 0,
                (STT_FUNC, _) => 1,
                _ => 2,
            };
            raw.push((value, size, limit, rank, name));
        }
        raw.sort_by_key(|&(addr, _, _, rank, _)| (addr, rank));
        raw.dedup_by_key(|s| s.0);

        let mut syms = Vec::with_capacity(raw.len());
        for i in 0..raw.len() {
            let (addr, size, limit, _, name) = &raw[i];
            let next = raw.get(i + 1).map_or(*limit, |n| n.0.min(*limit));
            let size = if *size == 0 { next.saturating_sub(*addr) } else { *size };
            if size > 0 {
                syms.push(Symbol { addr: *addr, size, name: demangle(name) });
            }
        }
        Ok(Symbols { syms })
    }

    /// The function containing `addr` and the offset into it.
    pub fn lookup(&self, addr: u64) -> Option<(&str, u64)> {
        let i = self.syms.partition_point(|s| s.addr <= addr).checked_sub(1)?;
        let s = &self.syms[i];
        (addr - s.addr < s.size).then_some((s.name.as_str(), addr - s.addr))
    }

    /// `name+0xoff` for an address, or `??` if no function contains it.
    pub fn describe(&self, addr: u64) -> String {
        match self.lookup(addr) {
            Some((name, off)) => format!("{name}+{off:#x}"),
            None => "??".into(),
        }
    }

    /// Append the symbol to a backtrace frame line (`#N 0xADDR`); other
    /// lines are returned unchanged.
    pub fn symbolize_line(&self, line: &str) -> String {
        let Some((n, addr, "")) = parse_frame(line) else { return line.to_string() };
        // Frames after #0 are return addresses: the call that made them is
        // the instruction before, which may even end the previous function.
        let probe = if n == 0 { addr } else { addr.wrapping_sub(1) };
        match self.lookup(probe) {
            Some((name, off)) => format!("{} {name}+{:#x}", line.trim_end(), off + (addr - probe)),
            None => line.to_string(),
        }
    }
}

fn c_str(data: &[u8], off: u64) -> Result<&str, String> {
    let tail = usize::try_from(off).ok().and_then(|o| data.get(o..)).ok_or("symbol name out of range")?;
    let len = tail.iter().position(|&b| b == 0).ok_or("unterminated symbol name")?;
    std::str::from_utf8(&tail[..len]).map_err(|_| "symbol name is not UTF-8".to_string())
}

/// Rust v0 and legacy names lose their hashes; anything else is kept.
fn demangle(name: &str) -> String {
    format!("{:#}", rustc_demangle::demangle(name))
}

/// Parse a backtrace frame line, `#N 0xADDR` as printed by the kernel and
/// optionally indented, into the frame number, the address and whatever
/// follows it (a symbol, once symbolised). `None` for any other line.
pub fn parse_frame(line: &str) -> Option<(usize, u64, &str)> {
    let rest = line.trim().strip_prefix('#')?;
    let (n, rest) = rest.split_once(' ')?;
    let (addr, tail) = rest.split_once(' ').unwrap_or((rest, ""));
    Some((n.parse().ok()?, parse_hex(addr.strip_prefix("0x")?)?, tail))
}

/// Hex number with digits only (no sign, spaces or suffix), `0x` optional.
pub fn parse_hex(s: &str) -> Option<u64> {
    let digits = s.strip_prefix("0x").unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const V0: &str = "_RNvNtCshIkM8DxHMxn_11poly_kernel8selftest18trigger_page_fault";
    const LEGACY: &str = "_ZN4core9panicking9panic_fmt17h0123456789abcdefE";

    /// Build a minimal ELF64 with sections: null, .text (0x1000..0x1100,
    /// executable), .data (0x2000..0x2100), .symtab, .strtab.
    fn elf(symbols: &[(&str, u64, u64, u8, u16)]) -> Vec<u8> {
        let mut strtab = vec![0u8];
        let mut symtab = vec![0u8; 24]; // null symbol
        for &(name, value, size, info, shndx) in symbols {
            symtab.extend((strtab.len() as u32).to_le_bytes());
            strtab.extend(name.bytes().chain([0]));
            symtab.extend([info, 0]);
            symtab.extend(shndx.to_le_bytes());
            symtab.extend(value.to_le_bytes());
            symtab.extend(size.to_le_bytes());
        }
        let mut data = vec![0u8; 64];
        data[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
        let symtab_off = data.len() as u64;
        data.extend(&symtab);
        let strtab_off = data.len() as u64;
        data.extend(&strtab);
        let shoff = data.len() as u64;
        // (type, flags, addr, offset, size, link)
        let headers = [
            (0, 0, 0, 0, 0, 0),
            (1, 0x6, 0x1000, 0, 0x100, 0),
            (1, 0x3, 0x2000, 0, 0x100, 0),
            (SHT_SYMTAB, 0, 0, symtab_off, symtab.len() as u64, 4),
            (3, 0, 0, strtab_off, strtab.len() as u64, 0),
        ];
        for (ty, flags, addr, offset, size, link) in headers {
            let mut h = [0u8; 64];
            h[4..8].copy_from_slice(&u32::to_le_bytes(ty));
            h[8..16].copy_from_slice(&u64::to_le_bytes(flags));
            h[16..24].copy_from_slice(&u64::to_le_bytes(addr));
            h[24..32].copy_from_slice(&u64::to_le_bytes(offset));
            h[32..40].copy_from_slice(&u64::to_le_bytes(size));
            h[40..44].copy_from_slice(&u32::to_le_bytes(link));
            data.extend(h);
        }
        data[0x28..0x30].copy_from_slice(&shoff.to_le_bytes());
        data[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
        data[0x3c..0x3e].copy_from_slice(&(headers.len() as u16).to_le_bytes());
        data
    }

    const FUNC_GLOBAL: u8 = 0x12;
    const FUNC_LOCAL: u8 = 0x02;
    const NOTYPE_LOCAL: u8 = 0x00;
    const OBJECT_GLOBAL: u8 = 0x11;

    fn sample() -> Symbols {
        Symbols::parse(&elf(&[
            (LEGACY, 0x1000, 0x40, FUNC_GLOBAL, 1),
            ("alias_of_panic_fmt", 0x1000, 0, NOTYPE_LOCAL, 1),
            (V0, 0x1040, 0x20, FUNC_LOCAL, 1),
            ("isr_common", 0x1080, 0, NOTYPE_LOCAL, 1),
            ("DATA", 0x2000, 8, OBJECT_GLOBAL, 2),
            ("undefined", 0, 0, FUNC_GLOBAL, 0),
        ]))
        .unwrap()
    }

    #[test]
    fn looks_up_and_demangles_functions() {
        let s = sample();
        assert_eq!(s.lookup(0x1000), Some(("core::panicking::panic_fmt", 0)));
        assert_eq!(s.lookup(0x103f), Some(("core::panicking::panic_fmt", 0x3f)));
        assert_eq!(s.lookup(0x1044), Some(("poly_kernel::selftest::trigger_page_fault", 4)));
        assert_eq!(s.lookup(0xfff), None);
        assert_eq!(s.lookup(0x1060), None, "gap after a sized function");
        assert_eq!(s.lookup(0x2000), None, "data symbols are not functions");
    }

    #[test]
    fn sizeless_labels_end_at_the_section_end() {
        let s = sample();
        assert_eq!(s.lookup(0x10ff), Some(("isr_common", 0x7f)));
        assert_eq!(s.lookup(0x1100), None);
    }

    #[test]
    fn rewrites_backtrace_lines() {
        let s = sample();
        assert_eq!(
            s.symbolize_line("  #0 0x0000000000001044"),
            "  #0 0x0000000000001044 poly_kernel::selftest::trigger_page_fault+0x4"
        );
        // A return address right past the end of a function belongs to its caller.
        assert_eq!(
            s.symbolize_line("  #1 0x0000000000001040"),
            "  #1 0x0000000000001040 core::panicking::panic_fmt+0x40"
        );
        assert_eq!(s.symbolize_line("  #2 0x0000000000009999"), "  #2 0x0000000000009999");
        assert_eq!(s.symbolize_line("mem: 0x1000"), "mem: 0x1000");
        let done = s.symbolize_line("#0 0x1044");
        assert_eq!(s.symbolize_line(&done), done, "already symbolised lines stay as they are");
    }

    #[test]
    fn parses_frames_and_hex() {
        assert_eq!(parse_frame("  #12 0xffffffff80106a8c"), Some((12, 0xffff_ffff_8010_6a8c, "")));
        assert_eq!(parse_frame("#0 0x10 foo+0x2"), Some((0, 0x10, "foo+0x2")));
        assert_eq!(parse_frame("#0 10"), None);
        assert_eq!(parse_frame("# 0x10"), None);
        assert_eq!(parse_hex("ffffffff80106a8c"), Some(0xffff_ffff_8010_6a8c));
        assert_eq!(parse_hex("0x"), None);
        assert_eq!(parse_hex("+1"), None);
    }

    #[test]
    fn rejects_non_elf_and_stripped_files() {
        assert!(Symbols::parse(b"MZ\x90\x00").is_err());
        let mut stripped = elf(&[]);
        let shoff = u64::from_le_bytes(stripped[0x28..0x30].try_into().unwrap()) as usize;
        stripped[shoff + 3 * 64 + 4] = 1; // turn .symtab into PROGBITS
        assert!(Symbols::parse(&stripped).unwrap_err().contains("no .symtab"));
        let truncated = &elf(&[(V0, 0x1040, 0x20, FUNC_LOCAL, 1)])[..100];
        assert!(Symbols::parse(truncated).is_err());
    }
}
