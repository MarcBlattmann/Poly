//! The buffer cache; see [`BufferCache`].

use crate::{BlockDevice, IoBuf, IoError, Op, RequestQueue, SubmitError, Ticket};

const NIL: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Io {
    None,
    Write(Ticket),
    Read(Ticket),
}

/// Bookkeeping for one buffer. The caller provides a table of them
/// (`[CacheSlot::EMPTY; N]`) and otherwise leaves them alone.
#[derive(Clone, Copy, Debug)]
pub struct CacheSlot {
    block: u64,
    /// Holds `block`'s contents (and is in the hash table).
    valid: bool,
    /// Changed since it was last read or written.
    dirty: bool,
    pins: u32,
    /// Neighbours on the LRU list, on which only unpinned slots are.
    prev: u32,
    next: u32,
    /// Next slot in the same hash chain.
    hash_next: u32,
    /// First slot of hash chain number (this slot's index).
    bucket: u32,
    /// A batched transfer in flight.
    io: Io,
}

impl CacheSlot {
    pub const EMPTY: CacheSlot = CacheSlot {
        block: 0,
        valid: false,
        dirty: false,
        pins: 0,
        prev: NIL,
        next: NIL,
        hash_next: NIL,
        bucket: NIL,
        io: Io::None,
    };
}

/// A pinned block. Give it back with [`BufferCache::release`].
#[must_use = "a buffer stays pinned until it is released"]
#[derive(Debug, PartialEq, Eq)]
pub struct Buf {
    slot: u32,
    block: u64,
}

impl Buf {
    pub const fn block(&self) -> u64 {
        self.block
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheError {
    Io(IoError),
    /// Every buffer is pinned: nothing can be evicted to make room.
    AllPinned,
}

impl From<IoError> for CacheError {
    fn from(e: IoError) -> CacheError {
        CacheError::Io(e)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    /// Blocks written to the device.
    pub write_backs: u64,
    /// Blocks dropped to make room.
    pub evictions: u64,
    /// Blocks read ahead.
    pub read_ahead: u64,
}

/// The buffer cache: recently used blocks of one device, kept in memory.
///
/// The caller provides the memory: a table of [`CacheSlot`]s and one block
/// of data per slot. Each slot holds at most one block; a hash table threaded
/// through the slots finds it by block number.
///
/// - **Pinning.** [`BufferCache::get`] hands out a [`Buf`], which pins the
///   block until [`BufferCache::release`] takes it back. A pinned block is
///   never evicted. `Buf` cannot be copied, so it cannot be released twice;
///   dropping one without releasing it leaves the block pinned for good.
/// - **Eviction** takes the least recently released unpinned block (exact
///   LRU: a list of the unpinned slots, least recent first, with empty
///   slots at the very front).
/// - **Dirty blocks** ([`BufferCache::data_mut`]) are written back when
///   evicted, by [`BufferCache::write_back`], or all together by
///   [`BufferCache::sync`], which then flushes the device. Only after a
///   successful `sync` are the changes durable.
/// - **Read-ahead.** [`BufferCache::prefetch`] reads a run of blocks in one
///   go, into slots that hold nothing or clean blocks only.
///
/// The cache does its I/O through a [`RequestQueue`] and waits for it, so it
/// hands out data only for blocks it has completely read, and no buffer
/// changes while the device works on it. Batched writes (sync) and reads
/// (read-ahead) go to the queue together, which sorts and merges them.
///
/// On a write-back error the block stays dirty and cached, and the
/// operation that needed the slot fails with the error. On a read error
/// nothing is cached.
pub struct BufferCache<'a> {
    slots: &'a mut [CacheSlot],
    data: &'a mut [u8],
    block_size: usize,
    /// Hash chains minus one; the count is a power of two.
    mask: u32,
    /// Least recently used end of the LRU list.
    lru_head: u32,
    lru_tail: u32,
    stats: CacheStats,
}

impl core::fmt::Debug for BufferCache<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BufferCache")
            .field("buffers", &self.slots.len())
            .field("block_size", &self.block_size)
            .field("stats", &self.stats)
            .finish()
    }
}

impl<'a> BufferCache<'a> {
    /// An empty cache of `slots.len()` buffers of `block_size` bytes (the
    /// device's block size), their contents in `data`. `None` if there are no
    /// slots, too many (2^32 - 1 or more), the block size is not a power of
    /// two, or `data` is too short.
    pub fn new(slots: &'a mut [CacheSlot], data: &'a mut [u8], block_size: usize) -> Option<BufferCache<'a>> {
        let n = slots.len();
        if n == 0 || n >= NIL as usize || !block_size.is_power_of_two() || data.len() / block_size < n {
            return None;
        }
        // Largest power of two not above the slot count: chains average at
        // most two slots.
        let buckets = 1u32 << (usize::BITS - 1 - n.leading_zeros());
        let mut cache = BufferCache {
            slots,
            data,
            block_size,
            mask: buckets - 1,
            lru_head: NIL,
            lru_tail: NIL,
            stats: CacheStats::default(),
        };
        cache.slots.fill(CacheSlot::EMPTY);
        for i in 0..n as u32 {
            cache.lru_push_back(i);
        }
        Some(cache)
    }

    pub const fn block_size(&self) -> usize {
        self.block_size
    }

    /// Number of buffers.
    pub const fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub const fn stats(&self) -> &CacheStats {
        &self.stats
    }

    pub fn is_cached(&self, block: u64) -> bool {
        self.lookup(block).is_some()
    }

    pub fn dirty_count(&self) -> usize {
        self.slots.iter().filter(|s| s.valid && s.dirty).count()
    }

    pub fn pinned_count(&self) -> usize {
        self.slots.iter().filter(|s| s.pins > 0).count()
    }

    /// The block, read from the device unless it is cached, pinned.
    pub fn get<D: BlockDevice>(&mut self, q: &mut RequestQueue<'_, D>, block: u64) -> Result<Buf, CacheError> {
        self.check_device(q)?;
        if let Some(i) = self.lookup(block) {
            self.stats.hits += 1;
            self.pin(i);
            return Ok(Buf { slot: i, block });
        }
        self.stats.misses += 1;
        let i = self.victim(q)?;
        if let Err(e) = q.read(block, self.block_mut(i)) {
            self.lru_push_front(i);
            return Err(e.into());
        }
        self.install(i, block);
        self.slots[i as usize].pins = 1;
        Ok(Buf { slot: i, block })
    }

    /// The block, pinned, filled with zeros and dirty, without reading it:
    /// for a block about to be overwritten in full.
    pub fn get_zeroed<D: BlockDevice>(&mut self, q: &mut RequestQueue<'_, D>, block: u64) -> Result<Buf, CacheError> {
        self.check_device(q)?;
        let info = q.info();
        if block >= info.blocks {
            return Err(IoError::OutOfRange.into());
        }
        if info.read_only {
            return Err(IoError::ReadOnly.into());
        }
        let i = match self.lookup(block) {
            Some(i) => {
                self.stats.hits += 1;
                self.pin(i);
                i
            }
            None => {
                self.stats.misses += 1;
                let i = self.victim(q)?;
                self.install(i, block);
                self.slots[i as usize].pins = 1;
                i
            }
        };
        self.block_mut(i).fill(0);
        self.slots[i as usize].dirty = true;
        Ok(Buf { slot: i, block })
    }

    pub fn data(&self, b: &Buf) -> &[u8] {
        self.check(b);
        let at = b.slot as usize * self.block_size;
        &self.data[at..at + self.block_size]
    }

    /// The block's contents for changing; marks it dirty.
    pub fn data_mut(&mut self, b: &Buf) -> &mut [u8] {
        self.check(b);
        self.slots[b.slot as usize].dirty = true;
        self.block_mut(b.slot)
    }

    /// Unpin the block. It stays cached (and dirty, if it is).
    pub fn release(&mut self, b: Buf) {
        self.check(&b);
        let s = &mut self.slots[b.slot as usize];
        s.pins -= 1;
        if s.pins == 0 {
            self.lru_push_back(b.slot);
        }
    }

    /// Write the block to the device now if it is dirty. (It is durable only
    /// after a flush; see [`BufferCache::sync`].)
    pub fn write_back<D: BlockDevice>(&mut self, q: &mut RequestQueue<'_, D>, b: &Buf) -> Result<(), CacheError> {
        self.check(b);
        self.check_device(q)?;
        if self.slots[b.slot as usize].dirty {
            self.write_slot(q, b.slot)?;
        }
        Ok(())
    }

    /// Write every dirty block (pinned ones too) and flush the device, so
    /// everything changed so far is durable. Blocks that fail to write stay
    /// dirty; the first error is returned, and then no flush is sent.
    pub fn sync<D: BlockDevice>(&mut self, q: &mut RequestQueue<'_, D>) -> Result<(), CacheError> {
        self.check_device(q)?;
        let mut error = None;
        for i in 0..self.slots.len() as u32 {
            let s = self.slots[i as usize];
            if !(s.valid && s.dirty) {
                continue;
            }
            let buf = self.io_buf(i);
            loop {
                // SAFETY: the memory is this cache's, which we hold mutably,
                // and nothing touches the slot's data until `finish_io`
                // below has collected the request.
                match unsafe { q.enqueue(Op::Write, s.block, buf) } {
                    Ok(t) => {
                        self.slots[i as usize].io = Io::Write(t);
                        break;
                    }
                    // Wait for ours to make room, then retry.
                    Err(SubmitError::Busy) if self.has_io() => self.finish_io(q, &mut error),
                    Err(SubmitError::Busy) => {
                        error.get_or_insert(IoError::Busy);
                        break;
                    }
                    Err(SubmitError::Rejected(e)) => {
                        error.get_or_insert(e);
                        break;
                    }
                }
            }
        }
        self.finish_io(q, &mut error);
        if let Some(e) = error {
            return Err(e.into());
        }
        q.flush()?;
        Ok(())
    }

    /// Read ahead up to `count` blocks from `first` (stopping at the end of
    /// the device) that are not cached yet, into empty or clean unpinned
    /// slots, least recently used first; it never writes anything back to
    /// make room. The blocks arrive unpinned, as most recently used. Returns
    /// how many were read.
    pub fn prefetch<D: BlockDevice>(&mut self, q: &mut RequestQueue<'_, D>, first: u64, count: u64) -> usize {
        if self.check_device(q).is_err() {
            return 0;
        }
        let end = first.saturating_add(count).min(q.info().blocks);
        let mut error = None;
        for block in first..end {
            if self.lookup(block).is_some() {
                continue;
            }
            let Some(i) = self.clean_victim() else { break };
            self.lru_unlink(i);
            self.evict(i);
            self.slots[i as usize].block = block;
            let buf = self.io_buf(i);
            // SAFETY: as in `sync`; the slot is off the LRU list and out of
            // the hash table, so nothing else reaches it until `finish_io`.
            match unsafe { q.enqueue(Op::Read, block, buf) } {
                Ok(t) => self.slots[i as usize].io = Io::Read(t),
                Err(_) => {
                    self.lru_push_front(i);
                    break;
                }
            }
        }
        let before = self.stats.read_ahead;
        self.finish_io(q, &mut error);
        (self.stats.read_ahead - before) as usize
    }

    /// Forget the block without writing it back (its contents no longer
    /// matter: the file system freed it). False if it is pinned.
    pub fn discard(&mut self, block: u64) -> bool {
        let Some(i) = self.lookup(block) else { return true };
        if self.slots[i as usize].pins > 0 {
            return false;
        }
        self.lru_unlink(i);
        self.evict(i);
        self.lru_push_front(i);
        true
    }

    // Internals.

    fn check(&self, b: &Buf) {
        let s = self.slots.get(b.slot as usize);
        assert!(
            s.is_some_and(|s| s.valid && s.pins > 0 && s.block == b.block),
            "buffer {b:?} does not belong to this cache"
        );
    }

    fn check_device<D: BlockDevice>(&self, q: &RequestQueue<'_, D>) -> Result<(), CacheError> {
        if q.info().block_size as usize != self.block_size {
            return Err(IoError::BadRequest.into());
        }
        Ok(())
    }

    fn block_mut(&mut self, i: u32) -> &mut [u8] {
        let at = i as usize * self.block_size;
        &mut self.data[at..at + self.block_size]
    }

    fn io_buf(&mut self, i: u32) -> IoBuf {
        IoBuf::from_mut(self.block_mut(i))
    }

    fn hash(&self, block: u64) -> usize {
        ((block.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32) as u32 & self.mask) as usize
    }

    fn lookup(&self, block: u64) -> Option<u32> {
        let mut i = self.slots[self.hash(block)].bucket;
        // Chains are acyclic; the bound only guards against a bug.
        for _ in 0..self.slots.len() {
            if i == NIL {
                return None;
            }
            let s = &self.slots[i as usize];
            if s.block == block {
                debug_assert!(s.valid);
                return Some(i);
            }
            i = s.hash_next;
        }
        None
    }

    /// Make slot `i` hold `block` (its data is about to be, or is).
    fn install(&mut self, i: u32, block: u64) {
        let h = self.hash(block);
        let chain = self.slots[h].bucket;
        let s = &mut self.slots[i as usize];
        s.block = block;
        s.valid = true;
        s.dirty = false;
        s.hash_next = chain;
        self.slots[h].bucket = i;
    }

    /// Drop whatever slot `i` holds (it is off the LRU list).
    fn evict(&mut self, i: u32) {
        let s = self.slots[i as usize];
        if !s.valid {
            return;
        }
        let h = self.hash(s.block);
        if self.slots[h].bucket == i {
            self.slots[h].bucket = s.hash_next;
        } else {
            let mut p = self.slots[h].bucket;
            while self.slots[p as usize].hash_next != i {
                p = self.slots[p as usize].hash_next;
            }
            self.slots[p as usize].hash_next = s.hash_next;
        }
        let s = &mut self.slots[i as usize];
        s.valid = false;
        s.dirty = false;
        s.hash_next = NIL;
        self.stats.evictions += 1;
    }

    /// A slot to reuse, off the LRU list and empty. Writes the least
    /// recently used block back first if it is dirty.
    fn victim<D: BlockDevice>(&mut self, q: &mut RequestQueue<'_, D>) -> Result<u32, CacheError> {
        let i = self.lru_head;
        if i == NIL {
            return Err(CacheError::AllPinned);
        }
        let s = self.slots[i as usize];
        if s.valid && s.dirty {
            self.write_slot(q, i)?;
        }
        self.lru_unlink(i);
        self.evict(i);
        Ok(i)
    }

    /// The least recently used unpinned slot that is empty or clean.
    fn clean_victim(&self) -> Option<u32> {
        let mut i = self.lru_head;
        while i != NIL {
            let s = &self.slots[i as usize];
            if !(s.valid && s.dirty) {
                return Some(i);
            }
            i = s.next;
        }
        None
    }

    fn write_slot<D: BlockDevice>(&mut self, q: &mut RequestQueue<'_, D>, i: u32) -> Result<(), IoError> {
        let at = i as usize * self.block_size;
        q.write(self.slots[i as usize].block, &self.data[at..at + self.block_size])?;
        self.slots[i as usize].dirty = false;
        self.stats.write_backs += 1;
        Ok(())
    }

    fn has_io(&self) -> bool {
        self.slots.iter().any(|s| s.io != Io::None)
    }

    /// Wait for every batched transfer and account for it; remember the
    /// first error.
    fn finish_io<D: BlockDevice>(&mut self, q: &mut RequestQueue<'_, D>, error: &mut Option<IoError>) {
        for i in 0..self.slots.len() as u32 {
            let io = core::mem::replace(&mut self.slots[i as usize].io, Io::None);
            match io {
                Io::None => {}
                Io::Write(t) => match q.wait(t) {
                    Ok(()) => {
                        self.slots[i as usize].dirty = false;
                        self.stats.write_backs += 1;
                    }
                    Err(e) => {
                        error.get_or_insert(e);
                    }
                },
                Io::Read(t) => match q.wait(t) {
                    Ok(()) => {
                        let block = self.slots[i as usize].block;
                        self.install(i, block);
                        self.lru_push_back(i);
                        self.stats.read_ahead += 1;
                    }
                    Err(e) => {
                        error.get_or_insert(e);
                        self.lru_push_front(i);
                    }
                },
            }
        }
    }

    fn pin(&mut self, i: u32) {
        if self.slots[i as usize].pins == 0 {
            self.lru_unlink(i);
        }
        self.slots[i as usize].pins += 1;
    }

    fn lru_unlink(&mut self, i: u32) {
        let CacheSlot { prev, next, .. } = self.slots[i as usize];
        match prev {
            NIL => self.lru_head = next,
            p => self.slots[p as usize].next = next,
        }
        match next {
            NIL => self.lru_tail = prev,
            n => self.slots[n as usize].prev = prev,
        }
        let s = &mut self.slots[i as usize];
        s.prev = NIL;
        s.next = NIL;
    }

    /// Most recently used end.
    fn lru_push_back(&mut self, i: u32) {
        let tail = self.lru_tail;
        let s = &mut self.slots[i as usize];
        s.prev = tail;
        s.next = NIL;
        match tail {
            NIL => self.lru_head = i,
            t => self.slots[t as usize].next = i,
        }
        self.lru_tail = i;
    }

    /// Least recently used end, for empty slots.
    fn lru_push_front(&mut self, i: u32) {
        let head = self.lru_head;
        let s = &mut self.slots[i as usize];
        s.prev = NIL;
        s.next = head;
        match head {
            NIL => self.lru_tail = i,
            h => self.slots[h as usize].prev = i,
        }
        self.lru_head = i;
    }
}

#[cfg(test)]
mod tests;
