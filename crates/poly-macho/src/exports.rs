//! The export trie: the symbols a dylib or executable exports, as a trie of
//! name fragments (`LC_DYLD_EXPORTS_TRIE`, or the export range of
//! `LC_DYLD_INFO`).
//!
//! Layout (the comments of `<mach-o/loader.h>`, `struct dyld_info_command`):
//! a node starts with a ULEB128 `terminal size`. If it is not zero, that many
//! bytes of export information follow: ULEB128 flags, then an address offset
//! (regular), or a library ordinal and a name (re-export), or a stub and a
//! resolver offset (stub and resolver). After that comes one byte, the number
//! of edges, and each edge is a NUL-terminated name fragment and a ULEB128
//! offset of the child node in the trie. The first node is the root.
//!
//! The walk uses no recursion and no allocator: an explicit stack of
//! [`MAX_TRIE_DEPTH`] frames and a name buffer of [`MAX_EXPORT_NAME_LEN`]
//! bytes live in the walker's stack frame. A trie that is deeper than that,
//! or that visits more nodes than it has bytes (a cycle, or one subtree
//! reached from many edges), is rejected.

use core::ops::ControlFlow;

use crate::bytes::Cursor;
use crate::consts::*;
use crate::error::{Error, Result};

/// Deepest trie accepted, in edges from the root.
pub const MAX_TRIE_DEPTH: usize = 128;
/// Longest exported name accepted, in bytes.
pub const MAX_EXPORT_NAME_LEN: usize = 1024;

/// What kind of thing an export is (the low bits of its flags).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    /// `EXPORT_SYMBOL_FLAGS_KIND_REGULAR`: code or data at an address.
    Regular,
    /// `EXPORT_SYMBOL_FLAGS_KIND_THREAD_LOCAL`: a thread-local variable.
    ThreadLocal,
    /// `EXPORT_SYMBOL_FLAGS_KIND_ABSOLUTE`: a value, not an address.
    Absolute,
    /// The reserved value 3.
    Unknown,
}

/// What an export points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportTarget<'a> {
    /// An offset from the start of the image (its Mach-O header).
    Address(u64),
    /// A symbol re-exported from another library.
    Reexport {
        /// The 1-based library ordinal.
        ordinal: u64,
        /// The name in that library; empty means the same name.
        name: &'a [u8],
    },
    /// A function with a lazily resolved implementation.
    StubAndResolver {
        /// The offset of the stub.
        stub: u64,
        /// The offset of the resolver function.
        resolver: u64,
    },
}

/// One exported symbol's information.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Export<'a> {
    /// The flags (`EXPORT_SYMBOL_FLAGS_*`).
    pub flags: u64,
    /// What the export points at.
    pub target: ExportTarget<'a>,
}

impl Export<'_> {
    /// The kind of the export.
    pub const fn kind(&self) -> ExportKind {
        match self.flags & EXPORT_SYMBOL_FLAGS_KIND_MASK {
            EXPORT_SYMBOL_FLAGS_KIND_REGULAR => ExportKind::Regular,
            EXPORT_SYMBOL_FLAGS_KIND_THREAD_LOCAL => ExportKind::ThreadLocal,
            EXPORT_SYMBOL_FLAGS_KIND_ABSOLUTE => ExportKind::Absolute,
            _ => ExportKind::Unknown,
        }
    }

    /// Whether the definition is weak (`EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION`).
    pub const fn is_weak_definition(&self) -> bool {
        self.flags & EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION != 0
    }

    /// Whether the symbol is a re-export (`EXPORT_SYMBOL_FLAGS_REEXPORT`).
    pub const fn is_reexport(&self) -> bool {
        self.flags & EXPORT_SYMBOL_FLAGS_REEXPORT != 0
    }

    /// Whether the symbol has a stub and a resolver
    /// (`EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER`).
    pub const fn is_stub_and_resolver(&self) -> bool {
        self.flags & EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER != 0
    }

    /// Whether the resolver is static (`EXPORT_SYMBOL_FLAGS_STATIC_RESOLVER`).
    pub const fn is_static_resolver(&self) -> bool {
        self.flags & EXPORT_SYMBOL_FLAGS_STATIC_RESOLVER != 0
    }
}

/// An export trie, borrowed from the file.
#[derive(Clone, Copy, Debug)]
pub struct Exports<'a> {
    data: &'a [u8],
}

/// A parsed trie node: its export, and where its edges start.
struct Node<'a> {
    export: Option<Export<'a>>,
    edges_at: usize,
    edge_count: u8,
}

#[derive(Clone, Copy)]
struct Frame {
    /// Where the next edge of this node is.
    pos: usize,
    /// How many edges of this node are left.
    left: u8,
    /// How long the name was at this node.
    name_len: usize,
}

impl<'a> Exports<'a> {
    /// Wraps the bytes of a trie. Nothing is checked until it is walked.
    pub const fn new(data: &'a [u8]) -> Self {
        Exports { data }
    }

    /// The raw bytes of the trie.
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    fn node(&self, off: u64) -> Result<Node<'a>> {
        let off = usize::try_from(off).map_err(|_| Error::TrieOutOfRange)?;
        if off >= self.data.len() {
            return Err(Error::TrieOutOfRange);
        }
        let mut c = Cursor::at(self.data, off);
        let size = usize::try_from(c.uleb()?).map_err(|_| Error::TrieOutOfRange)?;
        let info_at = c.pos();
        let info_end = info_at.checked_add(size).ok_or(Error::TrieOutOfRange)?;
        let info = self.data.get(info_at..info_end).ok_or(Error::TrieOutOfRange)?;
        let edge_count = *self.data.get(info_end).ok_or(Error::TrieOutOfRange)?;
        let export = if size == 0 { None } else { Some(parse_terminal(info)?) };
        Ok(Node { export, edges_at: info_end + 1, edge_count })
    }

    /// Calls `f` with the name and information of every export, in trie
    /// order, until `f` breaks or the trie ends.
    ///
    /// Fails if the trie is malformed; `f` has then already seen the
    /// exports before the error.
    pub fn walk(&self, mut f: impl FnMut(&[u8], &Export<'a>) -> ControlFlow<()>) -> Result<()> {
        if self.data.is_empty() {
            return Ok(());
        }
        let mut name = [0u8; MAX_EXPORT_NAME_LEN];
        let mut stack = [Frame { pos: 0, left: 0, name_len: 0 }; MAX_TRIE_DEPTH];
        // A trie visits each node once; no node takes fewer than two bytes.
        let mut budget = self.data.len();

        let root = self.node(0)?;
        if let Some(e) = &root.export
            && f(&name[..0], e).is_break()
        {
            return Ok(());
        }
        stack[0] = Frame { pos: root.edges_at, left: root.edge_count, name_len: 0 };
        let mut depth = 1;
        while depth > 0 {
            let top = stack[depth - 1];
            if top.left == 0 {
                depth -= 1;
                continue;
            }
            let mut c = Cursor::at(self.data, top.pos);
            let label = c.cstr(MAX_EXPORT_NAME_LEN).ok_or(Error::TrieOutOfRange)?;
            let child = c.uleb()?;
            stack[depth - 1].pos = c.pos();
            stack[depth - 1].left -= 1;

            let new_len = top.name_len + label.len();
            if new_len > MAX_EXPORT_NAME_LEN {
                return Err(Error::NameTooLong);
            }
            if depth == MAX_TRIE_DEPTH || budget == 0 {
                return Err(Error::TrieTooComplex);
            }
            budget -= 1;
            name[top.name_len..new_len].copy_from_slice(label);

            let node = self.node(child)?;
            if let Some(e) = &node.export
                && f(&name[..new_len], e).is_break()
            {
                return Ok(());
            }
            stack[depth] = Frame { pos: node.edges_at, left: node.edge_count, name_len: new_len };
            depth += 1;
        }
        Ok(())
    }

    /// The number of exports.
    pub fn count(&self) -> Result<usize> {
        let mut n = 0;
        self.walk(|_, _| {
            n += 1;
            ControlFlow::Continue(())
        })?;
        Ok(n)
    }

    /// Looks one name up, following its characters down the trie.
    pub fn lookup(&self, name: &[u8]) -> Result<Option<Export<'a>>> {
        if self.data.is_empty() {
            return Ok(None);
        }
        let mut node = self.node(0)?;
        let mut matched = 0usize;
        // Steps are bounded like a walk: one node per byte of the trie.
        let mut budget = self.data.len();
        loop {
            if matched == name.len() {
                return Ok(node.export);
            }
            let mut c = Cursor::at(self.data, node.edges_at);
            let mut next = None;
            for _ in 0..node.edge_count {
                let label = c.cstr(MAX_EXPORT_NAME_LEN).ok_or(Error::TrieOutOfRange)?;
                let child = c.uleb()?;
                if !label.is_empty() && name[matched..].starts_with(label) {
                    next = Some((label.len(), child));
                    break;
                }
            }
            let Some((len, child)) = next else {
                return Ok(None);
            };
            if budget == 0 {
                return Err(Error::TrieTooComplex);
            }
            budget -= 1;
            matched += len;
            node = self.node(child)?;
        }
    }
}

/// Reads the export information of a terminal node.
fn parse_terminal(info: &[u8]) -> Result<Export<'_>> {
    let mut c = Cursor::new(info);
    let flags = c.uleb()?;
    let target = if flags & EXPORT_SYMBOL_FLAGS_REEXPORT != 0 {
        let ordinal = c.uleb()?;
        let name = c.cstr(MAX_EXPORT_NAME_LEN).ok_or(Error::TrieOutOfRange)?;
        ExportTarget::Reexport { ordinal, name }
    } else if flags & EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER != 0 {
        ExportTarget::StubAndResolver { stub: c.uleb()?, resolver: c.uleb()? }
    } else {
        ExportTarget::Address(c.uleb()?)
    };
    Ok(Export { flags, target })
}
