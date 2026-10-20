//! Extent trees ("ext4 Data Structures and Algorithms", "Extent Tree"):
//! which disk blocks hold a file's logical blocks, as runs.
//!
//! A node is a 12-byte header (magic `0xF30A`, entries, capacity, depth)
//! and 12-byte entries. The root lives in the inode's `i_block` (room for
//! 4 entries); other nodes are blocks, ending (after their capacity) with a
//! CRC-32C with `metadata_csum`. Leaves (depth 0) hold extents: first
//! logical block, length (up to 32768; above that the extent is
//! "uninitialised", length minus 32768, and reads as zeros) and first
//! physical block (48 bits). Index nodes hold the first logical block of
//! each child and the child's block. Entries are sorted, extents do not
//! overlap, and an index entry's key is its child's first key.
//!
//! - **Mapping a hole** extends the extent before it or after it when the
//!   new block is physically adjacent (files written in order become one
//!   extent per contiguous run), else inserts an extent.
//! - **A full node splits**: a leaf filled by appending keeps its entries
//!   and the new one starts a new leaf; otherwise it splits in half. The
//!   new node's key goes into the parent, which may split in turn. A full
//!   root moves its entries into a new block and becomes an index node
//!   with one entry: the tree gets one level deeper.
//! - **Writing into an uninitialised extent** splits it around the block,
//!   which becomes initialised (and is zero but for what is written).
//! - **Truncation** removes or shortens extents from the end, frees index
//!   and leaf blocks that become empty, and collapses the tree while the
//!   root's only child fits in the root.
//!
//! New nodes are written "fresh" and before the existing nodes that point
//! to them (see `balloc.rs`), so without a journal a crash never leaves a
//! pointer to an unwritten node; with one, all of it is in the transaction.

use crate::csum;
use crate::layout::{FL_EXTENTS, Inode, le16, le32, put16, put32};
use crate::{BlockIo, Ctx};
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{Errno, Result};

pub const MAGIC: u16 = 0xF30A;
/// Longest initialised extent (an uninitialised one is one shorter).
pub const INIT_MAX: u32 = 32768;
/// Entries in the root (in `i_block`).
pub const ROOT_MAX: usize = 4;
/// Deepest tree accepted (ext4's own limit).
const MAX_DEPTH: u16 = 5;
/// Logical blocks are 32 bits; the last one is not mappable (so an
/// extent's end fits).
pub const MAX_LBLK: u64 = u32::MAX as u64;
/// Extent tree blocks one operation keeps read.
const CACHE: usize = 8;

/// An entry: an extent (in a leaf) or a child (in an index node, `len` 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ent {
    pub lblk: u32,
    pub len: u32,
    pub uninit: bool,
    pub ptr: u64,
}

impl Ent {
    fn end(&self) -> u64 {
        self.lblk as u64 + self.len as u64
    }
}

/// A node, decoded.
#[derive(Clone, Debug)]
pub struct Node {
    /// Its block; 0 for the root in the inode.
    pub block: u64,
    pub depth: u16,
    pub max: usize,
    pub ents: Vec<Ent>,
}

#[derive(Default)]
pub(crate) struct ExtCache {
    nodes: Vec<Node>,
}

/// What a logical block maps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mapped {
    pub pblk: u64,
    pub uninit: bool,
    /// Blocks from this one to the end of its extent.
    pub run: u64,
}

/// A logical block's mapping, or the hole it is in and where the hole ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lookup {
    Mapped(Mapped),
    Hole { until: u64 },
}

/// One step down a tree: the node and the entry followed.
struct Step {
    node: Node,
    idx: usize,
    dirty: bool,
    fresh: bool,
}

/// Encodes a node into `out` (the 60 bytes of `i_block`, or a block).
fn encode(node: &Node, out: &mut [u8]) {
    put16(out, 0, MAGIC);
    put16(out, 2, node.ents.len() as u16);
    put16(out, 4, node.max as u16);
    put16(out, 6, node.depth);
    put32(out, 8, 0);
    for (i, e) in node.ents.iter().enumerate() {
        let at = 12 + 12 * i;
        put32(out, at, e.lblk);
        if node.depth == 0 {
            let len = if e.uninit { e.len + INIT_MAX } else { e.len };
            put16(out, at + 4, len as u16);
            put16(out, at + 6, (e.ptr >> 32) as u16);
            put32(out, at + 8, e.ptr as u32);
        } else {
            put32(out, at + 4, e.ptr as u32);
            put16(out, at + 8, (e.ptr >> 32) as u16);
            put16(out, at + 10, 0);
        }
    }
    for b in out[12 + 12 * node.ents.len()..12 + 12 * node.max].iter_mut() {
        *b = 0;
    }
}

/// An empty root: a leaf with room for four extents.
pub fn empty_root() -> [u8; 60] {
    let mut out = [0u8; 60];
    encode(&Node { block: 0, depth: 0, max: ROOT_MAX, ents: Vec::new() }, &mut out);
    out
}

impl<D: BlockIo> Ctx<'_, D> {
    /// Whether `[start, start + len)` holds no fixed metadata and is inside
    /// the volume.
    fn run_ok(&self, start: u64, len: u64) -> bool {
        let geo = self.geo;
        if start < geo.first_data || len == 0 || start.checked_add(len).is_none_or(|e| e > geo.blocks) {
            return false;
        }
        let i = geo.fixed.partition_point(|r| r.1 <= start);
        geo.fixed.get(i).is_none_or(|r| r.0 >= start + len)
    }

    /// Decodes and checks a node from `raw` (header, entries).
    fn decode(&self, raw: &[u8], block: u64, depth: Option<u16>) -> Result<Node> {
        let room = (raw.len() - 12) / 12;
        if le16(raw, 0) != MAGIC {
            return Err(self.corrupt());
        }
        let (n, max, d) = (le16(raw, 2) as usize, le16(raw, 4) as usize, le16(raw, 6));
        if max > room || n > max || max == 0 || d > MAX_DEPTH || depth.is_some_and(|want| want != d) {
            return Err(self.corrupt());
        }
        let mut ents: Vec<Ent> = Vec::with_capacity(n);
        for i in 0..n {
            let at = 12 + 12 * i;
            let lblk = le32(raw, at);
            let e = if d == 0 {
                let raw_len = le16(raw, at + 4) as u32;
                let (len, uninit) = if raw_len > INIT_MAX { (raw_len - INIT_MAX, true) } else { (raw_len, false) };
                let ptr = (le16(raw, at + 6) as u64) << 32 | le32(raw, at + 8) as u64;
                if len == 0 || lblk as u64 + len as u64 > MAX_LBLK + 1 || !self.run_ok(ptr, len as u64) {
                    return Err(self.corrupt());
                }
                Ent { lblk, len, uninit, ptr }
            } else {
                let ptr = le32(raw, at + 4) as u64 | (le16(raw, at + 8) as u64) << 32;
                if !self.block_ok(ptr) {
                    return Err(self.corrupt());
                }
                Ent { lblk, len: 0, uninit: false, ptr }
            };
            if let Some(prev) = ents.last() {
                let prev_end = if d == 0 { prev.end() } else { prev.lblk as u64 + 1 };
                if (e.lblk as u64) < prev_end {
                    return Err(self.corrupt());
                }
            }
            ents.push(e);
        }
        Ok(Node { block, depth: d, max, ents })
    }

    fn root(&self, inode: &Inode) -> Result<Node> {
        let raw = inode.block_bytes();
        self.decode(&raw, 0, None)
    }

    fn iseed(&self, inode: &Inode) -> Option<u32> {
        self.geo.meta_seed().map(|s| csum::inode_seed(s, inode.ino, inode.generation))
    }

    /// Tree block `block`, expected at `depth`.
    fn load(&mut self, inode: &Inode, block: u64, depth: u16) -> Result<Node> {
        if let Some(n) = self.ext.nodes.iter().find(|n| n.block == block) {
            if n.depth != depth {
                return Err(self.corrupt());
            }
            return Ok(n.clone());
        }
        if !self.block_ok(block) {
            return Err(self.corrupt());
        }
        let raw = self.read_block(block)?;
        let node = self.decode(&raw, block, Some(depth))?;
        if let Some(seed) = self.iseed(inode) {
            let tail = 12 + 12 * node.max;
            if tail + 4 > raw.len() || le32(&raw, tail) != csum::extent_block(seed, &raw, tail) {
                return Err(self.bad_csum());
            }
        }
        self.cache_node(&node);
        Ok(node)
    }

    fn cache_node(&mut self, node: &Node) {
        self.ext.nodes.retain(|n| n.block != node.block);
        if self.ext.nodes.len() >= CACHE {
            self.ext.nodes.remove(0);
        }
        self.ext.nodes.push(node.clone());
    }

    /// Writes a node: the root into the inode, others to their block.
    fn store(&mut self, inode: &mut Inode, node: &Node, fresh: bool) -> Result<()> {
        if node.block == 0 {
            let mut raw = [0u8; 60];
            encode(node, &mut raw);
            inode.set_block_bytes(&raw);
            return Ok(());
        }
        let mut raw = vec![0u8; self.geo.bs];
        encode(node, &mut raw);
        if let Some(seed) = self.iseed(inode) {
            let tail = 12 + 12 * node.max;
            let c = csum::extent_block(seed, &raw, tail);
            put32(&mut raw, tail, c);
        }
        self.cache_node(node);
        if fresh { self.write_fresh(node.block, &raw) } else { self.write_meta(node.block, &raw) }
    }

    /// The entry of `node` to follow for `lblk`: the last whose key is at
    /// most `lblk`, or the first.
    fn child_index(node: &Node, lblk: u64) -> usize {
        node.ents.partition_point(|e| e.lblk as u64 <= lblk).saturating_sub(1)
    }

    /// Where logical block `lblk` is.
    pub fn ext_lookup(&mut self, inode: &Inode, lblk: u64) -> Result<Lookup> {
        if lblk > MAX_LBLK {
            return Ok(Lookup::Hole { until: u64::MAX });
        }
        let mut node = self.root(inode)?;
        let mut until = MAX_LBLK + 1;
        loop {
            if node.depth == 0 {
                let i = node.ents.partition_point(|e| e.lblk as u64 <= lblk);
                if let Some(next) = node.ents.get(i) {
                    until = until.min(next.lblk as u64);
                }
                if let Some(e) = i.checked_sub(1).map(|p| node.ents[p])
                    && lblk < e.end()
                {
                    let off = lblk - e.lblk as u64;
                    return Ok(Lookup::Mapped(Mapped { pblk: e.ptr + off, uninit: e.uninit, run: e.len as u64 - off }));
                }
                return Ok(Lookup::Hole { until });
            }
            if node.ents.is_empty() {
                return Ok(Lookup::Hole { until });
            }
            let i = Self::child_index(&node, lblk);
            if lblk < node.ents[i].lblk as u64 {
                return Ok(Lookup::Hole { until: until.min(node.ents[i].lblk as u64) });
            }
            if let Some(next) = node.ents.get(i + 1) {
                until = until.min(next.lblk as u64);
            }
            let child = node.ents[i].ptr;
            node = self.load(inode, child, node.depth - 1)?;
        }
    }

    pub fn ext_map(&mut self, inode: &Inode, lblk: u64) -> Result<Option<Mapped>> {
        match self.ext_lookup(inode, lblk)? {
            Lookup::Mapped(m) => Ok(Some(m)),
            Lookup::Hole { .. } => Ok(None),
        }
    }

    /// The nodes from the root to the leaf that covers `lblk`.
    fn path_to(&mut self, inode: &Inode, lblk: u64) -> Result<Vec<Step>> {
        let mut node = self.root(inode)?;
        let mut path = Vec::new();
        loop {
            let idx = if node.depth == 0 { 0 } else { Self::child_index(&node, lblk) };
            let next = (node.depth > 0).then(|| node.ents.get(idx).map(|e| (e.ptr, node.depth - 1)));
            path.push(Step { node, idx, dirty: false, fresh: false });
            match next {
                None => return Ok(path),
                Some(None) => return Err(self.corrupt()), // an empty index node
                Some(Some((child, depth))) => node = self.load(inode, child, depth)?,
            }
        }
    }

    /// Makes each index key on the path equal to its child's first key.
    fn fix_keys(path: &mut [Step]) {
        for l in (0..path.len() - 1).rev() {
            let Some(first) = path[l + 1].node.ents.first().map(|e| e.lblk) else { break };
            let idx = path[l].idx;
            if path[l].node.ents[idx].lblk == first {
                break;
            }
            path[l].node.ents[idx].lblk = first;
            path[l].dirty = true;
        }
    }

    /// Writes the dirty nodes of a path: new ones first, then the existing
    /// ones (which may point to them), the root into the inode.
    fn store_path(&mut self, inode: &mut Inode, path: &[Step], extra: &[Node]) -> Result<()> {
        for n in extra {
            self.store(inode, n, true)?;
        }
        for s in path.iter().rev().filter(|s| s.dirty) {
            self.store(inode, &s.node, s.fresh)?;
        }
        Ok(())
    }

    /// Inserts `ent` into the leaf of `path` (at its place), splitting full
    /// nodes upwards and growing the tree at a full root. Writes the
    /// changed nodes.
    fn insert_in(&mut self, inode: &mut Inode, mut path: Vec<Step>, ent: Ent, goal: u64) -> Result<()> {
        // Blocks the splits need: each full node from the leaf up, and one
        // more if that reaches a full root.
        let mut need = 0;
        for s in path.iter().rev() {
            if s.node.ents.len() < s.node.max {
                break;
            }
            need += 1;
        }
        let mut blocks = Vec::with_capacity(need);
        for _ in 0..need {
            match self.alloc_block(goal) {
                Ok(b) => blocks.push(b),
                Err(e) => {
                    self.free_blocks(&blocks)?;
                    return Err(e);
                }
            }
        }
        for _ in &blocks {
            self.charge(inode)?;
        }
        let leaf = path.len() - 1;
        let appended = {
            let l = &mut path[leaf];
            let pos = l.node.ents.partition_point(|e| e.lblk < ent.lblk);
            l.node.ents.insert(pos, ent);
            l.dirty = true;
            pos + 1 == l.node.ents.len()
        };
        Self::fix_keys(&mut path);
        let mut fresh: Vec<Node> = Vec::new();
        let mut level = leaf;
        let bmax = (self.geo.bs - 12) / 12;
        while path[level].node.ents.len() > path[level].node.max {
            let b = blocks.pop().ok_or_else(|| self.corrupt())?;
            if level == 0 {
                // Grow: the root's entries move into a new block.
                let root = &mut path[0];
                let child = Node { block: b, depth: root.node.depth, max: bmax, ents: core::mem::take(&mut root.node.ents) };
                root.node.ents.push(Ent { lblk: child.ents[0].lblk, len: 0, uninit: false, ptr: b });
                root.node.depth += 1;
                let idx = root.idx;
                root.idx = 0;
                path.insert(1, Step { node: child, idx, dirty: true, fresh: true });
                level = 1;
                continue;
            }
            let node = &mut path[level].node;
            let split = if appended && level == leaf { node.ents.len() - 1 } else { node.ents.len() / 2 };
            let upper = node.ents.split_off(split);
            let new = Node { block: b, depth: node.depth, max: node.max, ents: upper };
            path[level].dirty = true;
            let parent = &mut path[level - 1];
            parent.node.ents.insert(parent.idx + 1, Ent { lblk: new.ents[0].lblk, len: 0, uninit: false, ptr: b });
            parent.dirty = true;
            fresh.push(new);
            level -= 1;
        }
        // Splits put nodes in front of those that point to them.
        self.store_path(inode, &path, &fresh)
    }

    /// Maps logical block `lblk`, allocating a block near `goal` (which
    /// moves past it) if it is a hole. Returns the block and whether it is
    /// new or was uninitialised (its contents count as zeros).
    pub fn ext_alloc(&mut self, inode: &mut Inode, lblk: u64, goal: &mut u64) -> Result<(u64, bool)> {
        if lblk >= MAX_LBLK {
            return Err(Errno::EFBIG);
        }
        match self.ext_lookup(inode, lblk)? {
            Lookup::Mapped(m) if !m.uninit => return Ok((m.pblk, false)),
            Lookup::Mapped(m) => {
                self.ext_convert(inode, lblk)?;
                return Ok((m.pblk, true));
            }
            Lookup::Hole { .. } => {}
        }
        let b = self.alloc_block(*goal)?;
        if let Err(e) = self.charge(inode).and_then(|()| self.ext_add(inode, lblk as u32, b, *goal)) {
            self.free_blocks(&[b])?;
            return Err(e);
        }
        *goal = b + 1;
        Ok((b, true))
    }

    /// Records that `lblk` (a hole) is block `b`: extends a neighbouring
    /// extent when they are adjacent, else inserts one.
    fn ext_add(&mut self, inode: &mut Inode, lblk: u32, b: u64, goal: u64) -> Result<()> {
        let mut path = self.path_to(inode, lblk as u64)?;
        let leaf = path.len() - 1;
        let l = &mut path[leaf];
        let pos = l.node.ents.partition_point(|e| e.lblk < lblk);
        let joins_prev = pos > 0 && {
            let p = &l.node.ents[pos - 1];
            !p.uninit && p.end() == lblk as u64 && p.ptr + p.len as u64 == b && p.len < INIT_MAX
        };
        let joins_next = l.node.ents.get(pos).is_some_and(|n| {
            !n.uninit && n.lblk as u64 == lblk as u64 + 1 && n.ptr == b + 1 && n.len < INIT_MAX
        });
        if joins_prev {
            l.node.ents[pos - 1].len += 1;
            if joins_next && l.node.ents[pos - 1].len + l.node.ents[pos].len <= INIT_MAX {
                l.node.ents[pos - 1].len += l.node.ents[pos].len;
                l.node.ents.remove(pos);
            }
            l.dirty = true;
            return self.store_path(inode, &path, &[]);
        }
        if joins_next {
            let n = &mut l.node.ents[pos];
            n.lblk -= 1;
            n.ptr -= 1;
            n.len += 1;
            l.dirty = true;
            Self::fix_keys(&mut path);
            return self.store_path(inode, &path, &[]);
        }
        self.insert_in(inode, path, Ent { lblk, len: 1, uninit: false, ptr: b }, goal)
    }

    /// Makes block `lblk` of an uninitialised extent initialised: the
    /// extent splits into up to three.
    fn ext_convert(&mut self, inode: &mut Inode, lblk: u64) -> Result<()> {
        let mut path = self.path_to(inode, lblk)?;
        let leaf = path.len() - 1;
        let l = &mut path[leaf];
        let pos = l.node.ents.partition_point(|e| e.lblk as u64 <= lblk);
        let Some(i) = pos.checked_sub(1).filter(|&i| lblk < l.node.ents[i].end() && l.node.ents[i].uninit) else {
            return Err(self.corrupt());
        };
        let e = l.node.ents[i];
        let off = lblk - e.lblk as u64;
        let mut pieces = Vec::new();
        if off > 0 {
            pieces.push(Ent { lblk: e.lblk, len: off as u32, uninit: true, ptr: e.ptr });
        }
        pieces.push(Ent { lblk: lblk as u32, len: 1, uninit: false, ptr: e.ptr + off });
        if off + 1 < e.len as u64 {
            pieces.push(Ent {
                lblk: lblk as u32 + 1,
                len: e.len - off as u32 - 1,
                uninit: true,
                ptr: e.ptr + off + 1,
            });
        }
        l.node.ents[i] = pieces[0];
        l.dirty = true;
        Self::fix_keys(&mut path);
        self.store_path(inode, &path, &[])?;
        let goal = e.ptr;
        for p in &pieces[1..] {
            let path = self.path_to(inode, p.lblk as u64)?;
            self.insert_in(inode, path, *p, goal)?;
        }
        Ok(())
    }

    /// The logical block after the file's last mapped one (0 if none).
    pub fn ext_end(&mut self, inode: &Inode) -> Result<u64> {
        let mut node = self.root(inode)?;
        loop {
            let Some(last) = node.ents.last().copied() else { return Ok(0) };
            if node.depth == 0 {
                return Ok(last.end());
            }
            node = self.load(inode, last.ptr, node.depth - 1)?;
        }
    }

    /// Frees a run of data blocks of the file.
    fn drop_run(&mut self, inode: &mut Inode, start: u64, len: u64, freed: &mut Vec<u64>) {
        for b in start..start + len {
            freed.push(b);
        }
        inode.blocks = inode.blocks.saturating_sub(len * self.geo.spb as u64);
    }

    /// Frees a tree block of the file.
    fn drop_node(&mut self, inode: &mut Inode, block: u64, freed: &mut Vec<u64>) {
        self.ext.nodes.retain(|n| n.block != block);
        self.drop_run(inode, block, 1, freed);
    }

    /// Frees everything under (and including) node `node`.
    fn free_subtree(&mut self, inode: &mut Inode, node: &Node, freed: &mut Vec<u64>) -> Result<()> {
        if node.depth == 0 {
            for e in &node.ents {
                self.drop_run(inode, e.ptr, e.len as u64, freed);
            }
        } else {
            for e in &node.ents {
                match self.load(inode, e.ptr, node.depth - 1) {
                    Ok(child) => self.free_subtree(inode, &child, freed)?,
                    Err(Errno::EUCLEAN | Errno::EBADMSG) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        if node.block != 0 {
            self.drop_node(inode, node.block, freed);
        }
        Ok(())
    }

    /// Cuts `node`'s subtree back to the blocks below `keep`; returns
    /// whether anything in it changed (`node` itself is the caller's to
    /// store).
    fn trunc_node(&mut self, inode: &mut Inode, node: &mut Node, keep: u64, freed: &mut Vec<u64>) -> Result<bool> {
        let mut changed = false;
        if node.depth == 0 {
            while let Some(last) = node.ents.last().copied() {
                if last.lblk as u64 >= keep {
                    self.drop_run(inode, last.ptr, last.len as u64, freed);
                    node.ents.pop();
                } else if last.end() > keep {
                    let kept = keep - last.lblk as u64;
                    self.drop_run(inode, last.ptr + kept, last.len as u64 - kept, freed);
                    if let Some(e) = node.ents.last_mut() {
                        e.len = kept as u32;
                    }
                    changed = true;
                    break;
                } else {
                    break;
                }
                changed = true;
            }
            return Ok(changed);
        }
        while let Some(last) = node.ents.last().copied() {
            let mut child = self.load(inode, last.ptr, node.depth - 1)?;
            if last.lblk as u64 >= keep {
                self.free_subtree(inode, &child, freed)?;
                node.ents.pop();
                changed = true;
                continue;
            }
            if self.trunc_node(inode, &mut child, keep, freed)? {
                changed = true;
                if child.ents.is_empty() {
                    self.drop_node(inode, child.block, freed);
                    node.ents.pop();
                    continue;
                }
                self.store(inode, &child, false)?;
            }
            break;
        }
        Ok(changed)
    }

    /// Frees every block of the file at or past logical block `keep`
    /// (data and tree blocks, appended to `freed`; the caller frees them
    /// after writing the inode), and shrinks the tree.
    pub fn ext_truncate(&mut self, inode: &mut Inode, keep: u64, freed: &mut Vec<u64>) -> Result<()> {
        if keep > MAX_LBLK {
            return Ok(());
        }
        let mut root = self.root(inode)?;
        let changed = self.trunc_node(inode, &mut root, keep, freed)?;
        if !changed {
            return Ok(());
        }
        if root.ents.is_empty() {
            root.depth = 0;
        }
        // Collapse while the root's only child fits in the root.
        while root.depth > 0 && root.ents.len() == 1 {
            let child = self.load(inode, root.ents[0].ptr, root.depth - 1)?;
            if child.ents.len() > ROOT_MAX {
                break;
            }
            self.drop_node(inode, child.block, freed);
            root.ents = child.ents;
            root.depth = child.depth;
        }
        self.store(inode, &root, false)
    }

    /// Gives a new inode an empty extent tree.
    pub fn ext_init(&self, inode: &mut Inode) {
        inode.flags |= FL_EXTENTS;
        inode.set_block_bytes(&empty_root());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_encode_as_documented() {
        let node = Node {
            block: 0,
            depth: 0,
            max: 4,
            ents: vec![
                Ent { lblk: 0, len: 5, uninit: false, ptr: 1 << 33 | 7 },
                Ent { lblk: 9, len: 2, uninit: true, ptr: 100 },
            ],
        };
        let mut raw = [0u8; 60];
        encode(&node, &mut raw);
        assert_eq!(&raw[..12], &[0x0A, 0xF3, 2, 0, 4, 0, 0, 0, 0, 0, 0, 0]);
        // ee_block, ee_len, ee_start_hi, ee_start_lo.
        assert_eq!(&raw[12..24], &[0, 0, 0, 0, 5, 0, 2, 0, 7, 0, 0, 0]);
        // Uninitialised: length plus 32768.
        assert_eq!(le16(&raw, 28), 32768 + 2);
        let idx = Node { block: 0, depth: 1, max: 4, ents: vec![Ent { lblk: 3, len: 0, uninit: false, ptr: 1 << 32 | 9 }] };
        encode(&idx, &mut raw);
        // ei_block, ei_leaf_lo, ei_leaf_hi.
        assert_eq!(&raw[12..24], &[3, 0, 0, 0, 9, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(&empty_root()[..8], &[0x0A, 0xF3, 0, 0, 4, 0, 0, 0]);
    }
}
