//! A reader for the flattened devicetree blob (DTB) that QEMU's `virt` machine
//! passes to a guest: just enough to find the RAM, the PL011 UART, the
//! command line and the PSCI conduit.
//!
//! Layout from the Devicetree Specification, chapter 5 ("Flattened Devicetree
//! (DTB) Format"): a 40-byte header of big-endian 32-bit words, a structure
//! block of tokens (`FDT_BEGIN_NODE` and a name, `FDT_PROP` with a length, a
//! name offset and a value, `FDT_END_NODE`, `FDT_NOP`, `FDT_END`), and a
//! strings block of property names. `reg` is read with the `#address-cells`
//! and `#size-cells` of the parent node (defaults 2 and 1, section 2.3.5).
//!
//! Everything is bounds checked and returns `None` on a malformed blob: the
//! blob comes from the machine, but a reader that cannot fault is worth the
//! few extra lines.

/// The magic number at the start of a blob (specification, 5.2).
pub const MAGIC: u32 = 0xd00d_feed;

const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;

/// Deepest nesting followed (real trees are 3 or 4 deep).
const MAX_DEPTH: usize = 16;

/// Why a blob was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FdtError {
    /// Fewer bytes than the header (or than `totalsize`) says.
    TooShort,
    /// The first word is not [`MAGIC`].
    BadMagic,
    /// A block lies outside the blob or is misaligned.
    BadLayout,
    /// `last_comp_version` is newer than the 17 this reader understands.
    Unsupported,
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(bytes.try_into().ok()?))
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// The `totalsize` field of the header at the start of `header` (at least
/// its first 8 bytes), after checking the magic number.
pub fn total_size(header: &[u8]) -> Result<usize, FdtError> {
    let (magic, size) = match (be32(header, 0), be32(header, 4)) {
        (Some(m), Some(s)) => (m, s),
        _ => return Err(FdtError::TooShort),
    };
    if magic != MAGIC {
        return Err(FdtError::BadMagic);
    }
    Ok(size as usize)
}

/// A memory range: a `reg` entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reg {
    pub base: u64,
    pub size: u64,
}

/// A parsed blob header: the structure and strings blocks.
#[derive(Clone, Copy, Debug)]
pub struct Fdt<'a> {
    structs: &'a [u8],
    strings: &'a [u8],
}

impl<'a> Fdt<'a> {
    /// Parse the blob in `data` (which may be longer than `totalsize`).
    pub fn new(data: &'a [u8]) -> Result<Fdt<'a>, FdtError> {
        let total = total_size(data)?;
        let data = data.get(..total).ok_or(FdtError::TooShort)?;
        let word = |at| be32(data, at).ok_or(FdtError::TooShort);
        if word(24)? > 17 {
            return Err(FdtError::Unsupported);
        }
        let (off_struct, off_strings) = (word(8)? as usize, word(12)? as usize);
        let (size_strings, size_struct) = (word(32)? as usize, word(36)? as usize);
        if !off_struct.is_multiple_of(4) {
            return Err(FdtError::BadLayout);
        }
        let block = |off: usize, len: usize| data.get(off..off.checked_add(len)?);
        let structs = block(off_struct, size_struct).ok_or(FdtError::BadLayout)?;
        let strings = block(off_strings, size_strings).ok_or(FdtError::BadLayout)?;
        Ok(Fdt { structs, strings })
    }

    /// Every node, parents before children.
    pub fn nodes(&self) -> Nodes<'a> {
        Nodes { fdt: *self, pos: 0, depth: 0, cells: [(2, 1); MAX_DEPTH] }
    }

    /// The first node whose `compatible` list has `compatible`.
    pub fn find_compatible(&self, compatible: &str) -> Option<Node<'a>> {
        self.nodes().find(|n| n.is_compatible(compatible))
    }

    /// `reg` of the first node with `device_type = "memory"`: the RAM.
    pub fn memory(&self) -> Option<Reg> {
        self.nodes().find(|n| n.str_prop("device_type") == Some("memory"))?.reg()
    }

    /// `/chosen/bootargs`: the kernel command line (QEMU's `-append`).
    pub fn bootargs(&self) -> Option<&'a str> {
        self.nodes().find(|n| n.depth == 1 && n.name == "chosen")?.str_prop("bootargs")
    }

    /// `method` of the PSCI node: `"hvc"` or `"smc"`, how to call firmware.
    pub fn psci_method(&self) -> Option<&'a str> {
        let node =
            self.nodes().find(|n| ["arm,psci", "arm,psci-0.2", "arm,psci-1.0"].iter().any(|c| n.is_compatible(c)))?;
        node.str_prop("method")
    }
}

/// A node with its properties.
#[derive(Clone, Copy, Debug)]
pub struct Node<'a> {
    /// The node's name, with its unit address (`memory@40000000`).
    pub name: &'a str,
    /// 0 for the root.
    pub depth: usize,
    props: &'a [u8],
    strings: &'a [u8],
    /// `(#address-cells, #size-cells)` its `reg` is written with.
    parent_cells: (u32, u32),
}

impl<'a> Node<'a> {
    /// The value of the property `name`.
    pub fn prop(&self, name: &str) -> Option<&'a [u8]> {
        let mut pos = 0;
        while let Some((prop_name, value, next)) = read_prop(self.props, self.strings, pos) {
            if prop_name == name.as_bytes() {
                return Some(value);
            }
            pos = next;
        }
        None
    }

    /// A string property (value up to its NUL).
    pub fn str_prop(&self, name: &str) -> Option<&'a str> {
        let value = self.prop(name)?;
        let end = value.iter().position(|&b| b == 0)?;
        core::str::from_utf8(&value[..end]).ok()
    }

    /// Whether `compatible` (a list of NUL-terminated strings) has `s`.
    pub fn is_compatible(&self, s: &str) -> bool {
        self.prop("compatible").is_some_and(|list| list.split(|&b| b == 0).any(|item| item == s.as_bytes()))
    }

    /// The first `(address, size)` of `reg`.
    pub fn reg(&self) -> Option<Reg> {
        let (ac, sc) = (self.parent_cells.0 as usize, self.parent_cells.1 as usize);
        if ac == 0 || ac > 2 || sc > 2 {
            return None;
        }
        let value = self.prop("reg")?;
        let cell = |first: usize, count: usize| -> Option<u64> {
            let mut v = 0u64;
            for i in first..first + count {
                v = (v << 32) | u64::from(be32(value, i * 4)?);
            }
            Some(v)
        };
        Some(Reg { base: cell(0, ac)?, size: cell(ac, sc)? })
    }
}

/// The property at `pos` of a run of property tokens: its name, value and
/// the position after it. `None` where the run ends.
fn read_prop<'a>(props: &'a [u8], strings: &'a [u8], mut pos: usize) -> Option<(&'a [u8], &'a [u8], usize)> {
    while be32(props, pos)? == FDT_NOP {
        pos += 4;
    }
    if be32(props, pos)? != FDT_PROP {
        return None;
    }
    let len = be32(props, pos + 4)? as usize;
    let name_off = be32(props, pos + 8)? as usize;
    let value = props.get(pos + 12..(pos + 12).checked_add(len)?)?;
    let name_bytes = strings.get(name_off..)?;
    let name = &name_bytes[..name_bytes.iter().position(|&b| b == 0)?];
    // The padding to a 4-byte boundary must be there too.
    let next = align4(pos + 12 + len);
    (next <= props.len()).then_some((name, value, next))
}

/// An iterator over the nodes, see [`Fdt::nodes`].
#[derive(Clone, Debug)]
pub struct Nodes<'a> {
    fdt: Fdt<'a>,
    pos: usize,
    /// Nodes open now.
    depth: usize,
    /// `cells[d]`: the `(#address-cells, #size-cells)` of the children of
    /// the open node at depth `d`.
    cells: [(u32, u32); MAX_DEPTH],
}

impl<'a> Iterator for Nodes<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Node<'a>> {
        let structs = self.fdt.structs;
        loop {
            match be32(structs, self.pos)? {
                FDT_BEGIN_NODE => {
                    let name_at = self.pos + 4;
                    let rest = structs.get(name_at..)?;
                    let name_len = rest.iter().position(|&b| b == 0)?;
                    let name = core::str::from_utf8(&rest[..name_len]).unwrap_or("");
                    let props_at = align4(name_at + name_len + 1);
                    let depth = self.depth;
                    if depth >= MAX_DEPTH {
                        return None;
                    }
                    self.cells[depth] = (2, 1);
                    // The properties come right after the name; read them
                    // for the cell sizes their children are written with.
                    let mut end = props_at;
                    let run = structs.get(props_at..)?;
                    while let Some((prop, value, next)) = read_prop(run, self.fdt.strings, end - props_at) {
                        let cells = be32(value, 0);
                        match (prop, cells) {
                            (b"#address-cells", Some(c)) => self.cells[depth].0 = c,
                            (b"#size-cells", Some(c)) => self.cells[depth].1 = c,
                            _ => {}
                        }
                        end = props_at + next;
                    }
                    self.pos = end;
                    self.depth += 1;
                    let parent_cells = if depth == 0 { (2, 1) } else { self.cells[depth - 1] };
                    return Some(Node {
                        name,
                        depth,
                        props: &structs[props_at..end],
                        strings: self.fdt.strings,
                        parent_cells,
                    });
                }
                FDT_END_NODE => {
                    self.depth = self.depth.saturating_sub(1);
                    self.pos += 4;
                }
                FDT_NOP => self.pos += 4,
                // FDT_END, or anything else: nothing more to read.
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// A blob builder, laid out as the specification describes it.
    #[derive(Default)]
    struct Builder {
        structs: Vec<u8>,
        strings: Vec<u8>,
    }

    impl Builder {
        fn word(&mut self, w: u32) {
            self.structs.extend(w.to_be_bytes());
        }
        fn pad(&mut self) {
            while !self.structs.len().is_multiple_of(4) {
                self.structs.push(0);
            }
        }
        fn begin(&mut self, name: &str) -> &mut Self {
            self.word(FDT_BEGIN_NODE);
            self.structs.extend(name.as_bytes());
            self.structs.push(0);
            self.pad();
            self
        }
        fn end(&mut self) -> &mut Self {
            self.word(FDT_END_NODE);
            self
        }
        fn prop(&mut self, name: &str, value: &[u8]) -> &mut Self {
            let off = match self
                .strings
                .windows(name.len() + 1)
                .position(|w| w[..name.len()] == *name.as_bytes() && w[name.len()] == 0)
            {
                Some(off) => off,
                None => {
                    self.strings.extend(name.as_bytes());
                    self.strings.push(0);
                    self.strings.len() - name.len() - 1
                }
            };
            self.word(FDT_PROP);
            self.word(value.len() as u32);
            self.word(off as u32);
            self.structs.extend(value);
            self.pad();
            self
        }
        fn cells(&mut self, name: &str, cells: &[u32]) -> &mut Self {
            let bytes: Vec<u8> = cells.iter().flat_map(|c| c.to_be_bytes()).collect();
            self.prop(name, &bytes)
        }
        fn finish(mut self) -> Vec<u8> {
            self.word(9);
            let off_struct = 40 + 16; // header, one empty reservation entry
            let off_strings = off_struct + self.structs.len();
            let total = off_strings + self.strings.len();
            let mut out = Vec::new();
            for w in [MAGIC, total as u32, off_struct as u32, off_strings as u32, 40, 17, 16, 0] {
                out.extend(w.to_be_bytes());
            }
            out.extend((self.strings.len() as u32).to_be_bytes());
            out.extend((self.structs.len() as u32).to_be_bytes());
            out.extend([0u8; 16]);
            out.extend(&self.structs);
            out.extend(&self.strings);
            out
        }
    }

    /// The shape of QEMU's `virt` tree, with made-up numbers.
    fn virt() -> Vec<u8> {
        let mut b = Builder::default();
        b.begin("");
        b.cells("#address-cells", &[2]).cells("#size-cells", &[2]);
        b.begin("chosen").prop("bootargs", b"poly.a poly.selftest.panic\0").end();
        b.begin("memory@40000000")
            .prop("device_type", b"memory\0")
            .cells("reg", &[0, 0x4000_0000, 0, 0x1000_0000])
            .end();
        b.begin("psci").prop("compatible", b"arm,psci-1.0\0arm,psci-0.2\0").prop("method", b"hvc\0").end();
        b.begin("pl011@9000000")
            .prop("compatible", b"arm,pl011\0arm,primecell\0")
            .cells("reg", &[0, 0x0900_0000, 0, 0x1000])
            .end();
        b.begin("soc").cells("#address-cells", &[1]).cells("#size-cells", &[1]);
        b.begin("uart@1000").prop("compatible", b"vendor,uart\0").cells("reg", &[0x1000, 0x20]).end();
        b.end();
        b.end();
        b.finish()
    }

    #[test]
    fn finds_memory_uart_bootargs_and_psci() {
        let blob = virt();
        let fdt = Fdt::new(&blob).unwrap();
        assert_eq!(fdt.memory(), Some(Reg { base: 0x4000_0000, size: 0x1000_0000 }));
        let uart = fdt.find_compatible("arm,pl011").unwrap();
        assert_eq!(uart.name, "pl011@9000000");
        assert_eq!(uart.reg(), Some(Reg { base: 0x0900_0000, size: 0x1000 }));
        assert_eq!(fdt.bootargs(), Some("poly.a poly.selftest.panic"));
        assert_eq!(fdt.psci_method(), Some("hvc"));
    }

    #[test]
    fn reg_uses_the_parents_cell_sizes() {
        let blob = virt();
        let fdt = Fdt::new(&blob).unwrap();
        let node = fdt.find_compatible("vendor,uart").unwrap();
        assert_eq!(node.depth, 2);
        assert_eq!(node.reg(), Some(Reg { base: 0x1000, size: 0x20 }));
        // Siblings after `soc` still see the root's cells.
        assert!(fdt.find_compatible("no,such").is_none());
    }

    #[test]
    fn cells_default_to_two_and_one() {
        let mut b = Builder::default();
        b.begin("").begin("dev@1").cells("reg", &[0, 0x10, 0x20]).end().end();
        let blob = b.finish();
        let node = Fdt::new(&blob).unwrap().nodes().nth(1).unwrap();
        assert_eq!(node.reg(), Some(Reg { base: 0x10, size: 0x20 }));
    }

    #[test]
    fn compatible_is_a_list_and_matches_whole_strings() {
        let blob = virt();
        let fdt = Fdt::new(&blob).unwrap();
        assert!(fdt.find_compatible("arm,primecell").is_some());
        assert!(fdt.find_compatible("arm,pl01").is_none());
        assert!(fdt.find_compatible("arm,pl0111").is_none());
    }

    #[test]
    fn rejects_bad_blobs() {
        assert_eq!(Fdt::new(&[]).unwrap_err(), FdtError::TooShort);
        let mut blob = virt();
        assert_eq!(total_size(&blob), Ok(blob.len()));
        assert_eq!(Fdt::new(&blob[..blob.len() - 1]).unwrap_err(), FdtError::TooShort);
        blob[0] ^= 1;
        assert_eq!(Fdt::new(&blob).unwrap_err(), FdtError::BadMagic);
        blob[0] ^= 1;
        blob[8..12].copy_from_slice(&0xffff_fff0u32.to_be_bytes());
        assert_eq!(Fdt::new(&blob).unwrap_err(), FdtError::BadLayout);
        let mut blob = virt();
        blob[24..28].copy_from_slice(&18u32.to_be_bytes());
        assert_eq!(Fdt::new(&blob).unwrap_err(), FdtError::Unsupported);
    }

    /// A blob cut off anywhere reads without panicking.
    #[test]
    fn truncated_structure_block_never_panics() {
        let blob = virt();
        let fdt = Fdt::new(&blob).unwrap();
        for cut in 0..fdt.structs.len() {
            let short = Fdt { structs: &fdt.structs[..cut], strings: fdt.strings };
            let _ = short.memory();
            let _ = short.bootargs();
            let _ = short.psci_method();
            let _ = short.find_compatible("arm,pl011");
        }
    }
}
