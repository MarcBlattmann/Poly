//! Helpers shared by the integration tests: fixture loading, little-endian
//! patching, and a builder for synthetic Mach-O images.

#![allow(dead_code)]

use poly_macho::*;

/// The bytes of a committed test file under `tests/data/`.
pub fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// Every thin test file (the ones a fuzz run mutates).
pub const THIN_FIXTURES: &[&str] = &[
    "arm64_chained.exe",
    "arm64_classic.exe",
    "x86_64_classic.exe",
    "x86_64_chained.exe",
    "x86_64_nopie.exe",
    "libtiny.dylib",
    "libre.dylib",
];

pub fn w16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
pub fn w32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn w64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_le_bytes());
}
pub fn r32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
pub fn be32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_be_bytes());
}
pub fn be64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_be_bytes());
}

/// Unsigned LEB128.
pub fn uleb(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// A ULEB128 number always 3 bytes long (non-canonical, but valid), so
/// offsets can be laid out in one pass.
pub fn uleb3(v: u64) -> [u8; 3] {
    assert!(v < 1 << 21);
    [(v & 0x7f) as u8 | 0x80, ((v >> 7) & 0x7f) as u8 | 0x80, (v >> 14) as u8]
}

/// Signed LEB128.
pub fn sleb(mut v: i64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && byte & 0x40 == 0) || (v == -1 && byte & 0x40 != 0);
        if done {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn pad8(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(8) {
        v.push(0);
    }
}

fn name16(s: &str) -> [u8; 16] {
    let mut n = [0u8; 16];
    n[..s.len()].copy_from_slice(s.as_bytes());
    n
}

/// A section of a synthetic segment.
pub struct Sect {
    pub name: &'static str,
    pub addr: u64,
    pub size: u64,
    pub offset: u32,
    pub flags: u32,
}

/// Builds a 64-bit little-endian Mach-O image: a header, the load commands
/// added one by one, and a zero-filled body the test patches afterwards.
#[derive(Clone)]
pub struct Image {
    pub cputype: i32,
    pub cpusubtype: i32,
    pub filetype: u32,
    pub flags: u32,
    pub cmds: Vec<u8>,
    pub ncmds: u32,
}

impl Image {
    pub fn new(cputype: i32, filetype: u32) -> Image {
        Image { cputype, cpusubtype: 0, filetype, flags: 0, cmds: Vec::new(), ncmds: 0 }
    }

    pub fn arm64() -> Image {
        Image::new(CPU_TYPE_ARM64, MH_EXECUTE)
    }

    /// Adds a command: `cmd`, the computed `cmdsize`, then `body` padded to
    /// a multiple of 8.
    pub fn cmd(mut self, cmd: u32, body: &[u8]) -> Image {
        let mut c = Vec::new();
        c.extend_from_slice(&cmd.to_le_bytes());
        c.extend_from_slice(&0u32.to_le_bytes());
        c.extend_from_slice(body);
        pad8(&mut c);
        let len = c.len() as u32;
        c[4..8].copy_from_slice(&len.to_le_bytes());
        self.cmds.extend_from_slice(&c);
        self.ncmds += 1;
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn segment(
        self,
        name: &str,
        vmaddr: u64,
        vmsize: u64,
        fileoff: u64,
        filesize: u64,
        prot: u32,
        sections: &[Sect],
    ) -> Image {
        let mut b = Vec::new();
        b.extend_from_slice(&name16(name));
        b.extend_from_slice(&vmaddr.to_le_bytes());
        b.extend_from_slice(&vmsize.to_le_bytes());
        b.extend_from_slice(&fileoff.to_le_bytes());
        b.extend_from_slice(&filesize.to_le_bytes());
        b.extend_from_slice(&prot.to_le_bytes());
        b.extend_from_slice(&prot.to_le_bytes());
        b.extend_from_slice(&(sections.len() as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        for s in sections {
            b.extend_from_slice(&name16(s.name));
            b.extend_from_slice(&name16(name));
            b.extend_from_slice(&s.addr.to_le_bytes());
            b.extend_from_slice(&s.size.to_le_bytes());
            b.extend_from_slice(&s.offset.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes()); // align
            b.extend_from_slice(&0u32.to_le_bytes()); // reloff
            b.extend_from_slice(&0u32.to_le_bytes()); // nreloc
            b.extend_from_slice(&s.flags.to_le_bytes());
            b.extend_from_slice(&[0u8; 12]);
        }
        self.cmd(LC_SEGMENT_64, &b)
    }

    /// A command that is a path: `cmd`, `cmdsize`, offset 12, the string.
    pub fn string_cmd(self, cmd: u32, s: &str) -> Image {
        let mut b = Vec::new();
        b.extend_from_slice(&12u32.to_le_bytes());
        b.extend_from_slice(s.as_bytes());
        b.push(0);
        self.cmd(cmd, &b)
    }

    pub fn dylib(self, cmd: u32, name: &str, cur: u32, compat: u32) -> Image {
        let mut b = Vec::new();
        b.extend_from_slice(&24u32.to_le_bytes());
        b.extend_from_slice(&7u32.to_le_bytes()); // timestamp
        b.extend_from_slice(&cur.to_le_bytes());
        b.extend_from_slice(&compat.to_le_bytes());
        b.extend_from_slice(name.as_bytes());
        b.push(0);
        self.cmd(cmd, &b)
    }

    pub fn linkedit(self, cmd: u32, off: u32, size: u32) -> Image {
        let mut b = Vec::new();
        b.extend_from_slice(&off.to_le_bytes());
        b.extend_from_slice(&size.to_le_bytes());
        self.cmd(cmd, &b)
    }

    pub fn symtab(self, symoff: u32, nsyms: u32, stroff: u32, strsize: u32) -> Image {
        let mut b = Vec::new();
        for v in [symoff, nsyms, stroff, strsize] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        self.cmd(LC_SYMTAB, &b)
    }

    /// `LC_DYLD_INFO_ONLY` with the ranges rebase, bind, weak, lazy, export,
    /// each `(offset, size)`.
    pub fn dyld_info(self, ranges: [(u32, u32); 5]) -> Image {
        let mut b = Vec::new();
        for (o, s) in ranges {
            b.extend_from_slice(&o.to_le_bytes());
            b.extend_from_slice(&s.to_le_bytes());
        }
        self.cmd(LC_DYLD_INFO_ONLY, &b)
    }

    /// `LC_UNIXTHREAD` with one state of `count` words, `regs` set as given
    /// (register index, value).
    pub fn thread(self, cmd: u32, flavor: u32, count: u32, regs: &[(usize, u64)]) -> Image {
        let mut state = vec![0u8; count as usize * 4];
        for &(i, v) in regs {
            state[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        let mut b = Vec::new();
        b.extend_from_slice(&flavor.to_le_bytes());
        b.extend_from_slice(&count.to_le_bytes());
        b.extend_from_slice(&state);
        self.cmd(cmd, &b)
    }

    /// The file: header, commands, then zeros up to `len`.
    pub fn build(&self, len: usize) -> Vec<u8> {
        let mut f = vec![0u8; len];
        w32(&mut f, 0, MH_MAGIC_64);
        w32(&mut f, 4, self.cputype as u32);
        w32(&mut f, 8, self.cpusubtype as u32);
        w32(&mut f, 12, self.filetype);
        w32(&mut f, 16, self.ncmds);
        w32(&mut f, 20, self.cmds.len() as u32);
        w32(&mut f, 24, self.flags);
        f[32..32 + self.cmds.len()].copy_from_slice(&self.cmds);
        f
    }
}

/// The offset of the first load command with this `cmd` in a parsed file.
pub fn cmd_offset(data: &[u8], cmd: u32) -> usize {
    let m = MachO::parse(data).expect("fixture parses");
    for lc in m.raw_commands() {
        let lc = lc.unwrap();
        if lc.cmd() == cmd {
            return lc.data().as_ptr() as usize - data.as_ptr() as usize;
        }
    }
    panic!("no load command {cmd:#x}");
}

/// The offsets of every load command, in order.
pub fn cmd_offsets(data: &[u8]) -> Vec<usize> {
    let m = MachO::parse(data).expect("fixture parses");
    m.raw_commands().map(|lc| lc.unwrap().data().as_ptr() as usize - data.as_ptr() as usize).collect()
}

/// A small deterministic PRNG (xorshift64*), so tests are reproducible.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

// ---------------------------------------------------------------------------
// Synthetic images with chained fixups
// ---------------------------------------------------------------------------

/// Where the `__DATA` segment of [`chained_image`] sits.
pub const DATA_FILE: usize = 0x4000;
pub const DATA_VM: u64 = 0x1_0000_4000;
/// The file offset of the chained fixups payload.
pub const LINKEDIT_FILE: usize = 0xc000;
pub const IMAGE_LEN: usize = 0xd000;

/// One import to put in a payload: raw library ordinal bits as the format
/// stores them, the weak flag, the name and the addend.
pub struct ImportSpec {
    pub ordinal: u64,
    pub weak: bool,
    pub name: &'static str,
    pub addend: i64,
}

/// The payload of `LC_DYLD_CHAINED_FIXUPS` for an image whose segment 1
/// (`__DATA`) has `page_starts`.
pub fn chained_payload(
    pointer_format: u16,
    page_size: u16,
    page_starts: &[u16],
    imports_format: u32,
    imports: &[ImportSpec],
) -> Vec<u8> {
    let isize_ = match imports_format {
        1 => 4,
        2 => 8,
        _ => 16,
    };
    let starts_off = 32usize;
    // starts_in_image: seg_count 3, offsets [0, 16, 0]
    let seg_off = starts_off + 16;
    let seg_size = 22 + 2 * page_starts.len();
    let imports_off = (seg_off + seg_size + 7) & !7;
    let symbols_off = imports_off + imports.len() * isize_;
    let mut names = Vec::new();
    let mut name_offs = Vec::new();
    for i in imports {
        name_offs.push(names.len() as u64);
        names.extend_from_slice(i.name.as_bytes());
        names.push(0);
    }
    let mut p = vec![0u8; symbols_off + names.len()];
    w32(&mut p, 0, 0);
    w32(&mut p, 4, starts_off as u32);
    w32(&mut p, 8, imports_off as u32);
    w32(&mut p, 12, symbols_off as u32);
    w32(&mut p, 16, imports.len() as u32);
    w32(&mut p, 20, imports_format);
    w32(&mut p, 24, 0);
    w32(&mut p, starts_off, 3);
    w32(&mut p, starts_off + 4, 0);
    w32(&mut p, starts_off + 8, 16);
    w32(&mut p, starts_off + 12, 0);
    w32(&mut p, seg_off, seg_size as u32);
    w16(&mut p, seg_off + 4, page_size);
    w16(&mut p, seg_off + 6, pointer_format);
    w64(&mut p, seg_off + 8, 0x4000);
    w32(&mut p, seg_off + 16, 0);
    w16(&mut p, seg_off + 20, page_starts.len() as u16);
    for (i, &s) in page_starts.iter().enumerate() {
        w16(&mut p, seg_off + 22 + 2 * i, s);
    }
    for (i, imp) in imports.iter().enumerate() {
        let at = imports_off + i * isize_;
        match imports_format {
            1 | 2 => {
                let v = (imp.ordinal as u32 & 0xff) | (u32::from(imp.weak) << 8) | ((name_offs[i] as u32) << 9);
                w32(&mut p, at, v);
                if imports_format == 2 {
                    w32(&mut p, at + 4, imp.addend as i32 as u32);
                }
            }
            _ => {
                let v = (imp.ordinal & 0xffff) | (u64::from(imp.weak) << 16) | (name_offs[i] << 32);
                w64(&mut p, at, v);
                w64(&mut p, at + 8, imp.addend as u64);
            }
        }
    }
    p[symbols_off..].copy_from_slice(&names);
    p
}

/// An arm64 image with `__TEXT`, a `__DATA` of 0x8000 bytes at file offset
/// 0x4000 and a `__LINKEDIT` holding `payload`; `slots` are written into
/// `__DATA` (offset in the segment, value).
pub fn chained_image(payload: &[u8], slots: &[(usize, u64)]) -> Vec<u8> {
    let img = Image::arm64()
        .segment("__TEXT", 0x1_0000_0000, 0x4000, 0, 0x4000, 5, &[])
        .segment("__DATA", DATA_VM, 0x8000, DATA_FILE as u64, 0x8000, 3, &[])
        .segment("__LINKEDIT", 0x1_0000_c000, 0x1000, LINKEDIT_FILE as u64, 0x1000, 1, &[])
        .linkedit(LC_DYLD_CHAINED_FIXUPS, LINKEDIT_FILE as u32, payload.len() as u32);
    let mut f = img.build(IMAGE_LEN);
    f[LINKEDIT_FILE..LINKEDIT_FILE + payload.len()].copy_from_slice(payload);
    for &(off, v) in slots {
        w64(&mut f, DATA_FILE + off, v);
    }
    f
}

/// arm64e slot encoders, from the bit layouts of `<mach-o/fixup-chains.h>`.
pub mod slot {
    pub fn arm64e_rebase(target: u64, high8: u64, next: u64) -> u64 {
        target | high8 << 43 | next << 51
    }
    pub fn arm64e_auth_rebase(target: u64, div: u64, addr: u64, key: u64, next: u64) -> u64 {
        target | div << 32 | addr << 48 | key << 49 | next << 51 | 1 << 63
    }
    pub fn arm64e_bind(ordinal: u64, addend: i64, next: u64) -> u64 {
        ordinal | ((addend as u64) & 0x7ffff) << 32 | next << 51 | 1 << 62
    }
    pub fn arm64e_auth_bind(ordinal: u64, div: u64, addr: u64, key: u64, next: u64) -> u64 {
        ordinal | div << 32 | addr << 48 | key << 49 | next << 51 | 1 << 62 | 1 << 63
    }
    pub fn ptr64_rebase(target: u64, high8: u64, next: u64) -> u64 {
        target | high8 << 36 | next << 51
    }
    pub fn ptr64_bind(ordinal: u64, addend: u64, next: u64) -> u64 {
        ordinal | addend << 24 | next << 51 | 1 << 63
    }
}

// ---------------------------------------------------------------------------
// Export tries
// ---------------------------------------------------------------------------

/// A node of a trie under construction.
#[derive(Clone, Default)]
pub struct TrieNode {
    pub terminal: Option<Vec<u8>>,
    pub edges: Vec<(Vec<u8>, usize)>,
}

/// Export information of a regular symbol.
pub fn regular(flags: u64, addr: u64) -> Vec<u8> {
    let mut v = uleb(flags);
    v.extend(uleb(addr));
    v
}

/// Lays the nodes out one after the other (node 0 first); child offsets are
/// written as fixed-width ULEB128s so one pass is enough.
pub fn trie_bytes(nodes: &[TrieNode]) -> Vec<u8> {
    let size = |n: &TrieNode| {
        let t = match &n.terminal {
            Some(t) => uleb(t.len() as u64).len() + t.len(),
            None => 1,
        };
        t + 1 + n.edges.iter().map(|(l, _)| l.len() + 1 + 3).sum::<usize>()
    };
    let mut offs = Vec::new();
    let mut at = 0;
    for n in nodes {
        offs.push(at);
        at += size(n);
    }
    let mut out = Vec::new();
    for n in nodes {
        match &n.terminal {
            Some(t) => {
                out.extend(uleb(t.len() as u64));
                out.extend_from_slice(t);
            }
            None => out.push(0),
        }
        out.push(n.edges.len() as u8);
        for (label, child) in &n.edges {
            out.extend_from_slice(label);
            out.push(0);
            out.extend_from_slice(&uleb3(offs[*child] as u64));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Images with classic dyld info
// ---------------------------------------------------------------------------

pub const STREAM_AT: usize = 0x3000;
pub const CLASSIC_LEN: usize = 0xb000;

/// An image with segments `__TEXT` (vm 0x1000, 0x1000 bytes), `__DATA` (vm
/// 0x2000, 0x2000 bytes) and `__LINKEDIT` (0x8000 bytes), and the four
/// streams of `LC_DYLD_INFO_ONLY` at 0x3000 (rebase, up to 0x5000 bytes),
/// 0x8000, 0x8800 (up to 0x800 bytes each) and 0x9000 (up to 0x1000).
pub fn classic_image(rebase: &[u8], bind: &[u8], weak: &[u8], lazy: &[u8]) -> Vec<u8> {
    let at = [0x3000usize, 0x8000, 0x8800, 0x9000];
    let streams = [rebase, bind, weak, lazy];
    let img = Image::arm64()
        .segment("__TEXT", 0x1000, 0x1000, 0, 0x1000, 5, &[])
        .segment("__DATA", 0x2000, 0x2000, 0x1000, 0x2000, 3, &[])
        .segment("__LINKEDIT", 0x4000, 0x8000, 0x3000, 0x8000, 1, &[])
        .dyld_info([
            (at[0] as u32, rebase.len() as u32),
            (at[1] as u32, bind.len() as u32),
            (at[2] as u32, weak.len() as u32),
            (at[3] as u32, lazy.len() as u32),
            (0, 0),
        ]);
    let mut f = img.build(CLASSIC_LEN);
    for (a, s) in at.iter().zip(streams) {
        let room = match *a {
            0x3000 => 0x5000,
            0x9000 => 0x1000,
            _ => 0x800,
        };
        assert!(s.len() <= room);
        f[*a..*a + s.len()].copy_from_slice(s);
    }
    f
}
