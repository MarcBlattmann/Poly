//! A simulated disk for the unit tests: it keeps requests until polled and
//! then runs and completes them in random order, can have a volatile write
//! cache that a crash partly loses, fails chosen blocks, and logs what it
//! sees so tests can check ordering.

extern crate std;

use crate::{
    BlockDevice, Completion, DeviceInfo, DeviceSlot, IoBuf, IoError, Op, Part, QueueStorage, Request, RequestId,
    RequestSlot, SubmitError,
};
use std::collections::BTreeMap;
use std::{vec, vec::Vec};

/// xorshift64*: deterministic and dependency-free.
#[derive(Clone, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// True one time in `n`.
    pub fn one_in(&mut self, n: u64) -> bool {
        self.below(n) == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Submit { id: u64, op: Op, lba: u64, blocks: u32, segments: usize },
    Complete { id: u64 },
}

struct Pending {
    id: RequestId,
    op: Op,
    lba: u64,
    blocks: u32,
    segs: Vec<IoBuf>,
}

pub struct SimDisk {
    pub info: DeviceInfo,
    /// What survives a crash.
    pub durable: Vec<u8>,
    /// Written but not flushed yet, by block.
    pub volatile: BTreeMap<u64, Vec<u8>>,
    pending: Vec<Pending>,
    /// Requests held at once before `submit` says Busy.
    pub max_pending: usize,
    pub rng: Rng,
    pub log: Vec<Event>,
    /// Reads and writes touching these blocks fail.
    pub bad: Vec<u64>,
    /// Complete the oldest request first instead of a random one.
    pub in_order: bool,
}

impl SimDisk {
    pub fn new(blocks: u64, block_size: u32, write_cache: bool, seed: u64) -> SimDisk {
        let info = DeviceInfo {
            block_size,
            blocks,
            read_only: false,
            write_cache,
            max_blocks: 1 << 20,
            max_segments: 1024,
            max_segment_len: u32::MAX,
        };
        SimDisk {
            info,
            durable: vec![0; (blocks * block_size as u64) as usize],
            volatile: BTreeMap::new(),
            pending: Vec::new(),
            max_pending: 64,
            rng: Rng(seed | 1),
            log: Vec::new(),
            bad: Vec::new(),
            in_order: false,
        }
    }

    fn bs(&self) -> usize {
        self.info.block_size as usize
    }

    /// What a read of block `b` returns now.
    pub fn block(&self, b: u64) -> Vec<u8> {
        match self.volatile.get(&b) {
            Some(v) => v.clone(),
            None => {
                let at = b as usize * self.bs();
                self.durable[at..at + self.bs()].to_vec()
            }
        }
    }

    /// Power loss: each unflushed block reaches the medium or not, at
    /// random. Requests in flight are lost (the tests crash only when the
    /// layers above are idle).
    pub fn crash(&mut self) {
        assert!(self.pending.is_empty(), "crash with requests in flight");
        let volatile = core::mem::take(&mut self.volatile);
        for (b, data) in volatile {
            if self.rng.one_in(2) {
                let at = b as usize * self.bs();
                self.durable[at..at + data.len()].copy_from_slice(&data);
            }
        }
    }

    fn run(&mut self, p: &Pending) -> Result<(), IoError> {
        let bs = self.bs();
        let blocks = p.lba..p.lba + p.blocks as u64;
        if self.bad.iter().any(|b| blocks.contains(b)) {
            return Err(IoError::Media);
        }
        match p.op {
            Op::Read => {
                let data: Vec<u8> = blocks.flat_map(|b| self.block(b)).collect();
                let mut at = 0;
                for s in &p.segs {
                    // SAFETY: the submitter lent us the segment for writing
                    // until this completion.
                    unsafe { core::ptr::copy_nonoverlapping(data[at..].as_ptr(), s.ptr.as_ptr(), s.len) };
                    at += s.len;
                }
            }
            Op::Write => {
                let mut data = Vec::new();
                for s in &p.segs {
                    // SAFETY: lent to us for reading until this completion.
                    data.extend_from_slice(unsafe { core::slice::from_raw_parts(s.ptr.as_ptr(), s.len) });
                }
                for (i, b) in blocks.enumerate() {
                    let block = data[i * bs..(i + 1) * bs].to_vec();
                    if self.info.write_cache {
                        self.volatile.insert(b, block);
                    } else {
                        self.durable[b as usize * bs..(b as usize + 1) * bs].copy_from_slice(&block);
                    }
                }
            }
            Op::Flush => {
                let volatile = core::mem::take(&mut self.volatile);
                for (b, data) in volatile {
                    self.durable[b as usize * bs..(b as usize + 1) * bs].copy_from_slice(&data);
                }
            }
        }
        Ok(())
    }
}

impl BlockDevice for SimDisk {
    fn info(&self) -> DeviceInfo {
        self.info
    }

    unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError> {
        let blocks = req.check(&self.info).map_err(SubmitError::Rejected)?;
        // The driver's view of the limits, checked independently.
        assert!(req.segments.len() <= self.info.max_segments as usize);
        assert!(req.segments.iter().all(|s| s.len as u64 <= self.info.max_segment_len as u64));
        assert!(blocks <= self.info.max_blocks);
        assert!(self.pending.iter().all(|p| p.id != req.id), "id {:?} already in flight", req.id);
        if self.pending.len() == self.max_pending {
            return Err(SubmitError::Busy);
        }
        self.log.push(Event::Submit { id: req.id.0, op: req.op, lba: req.lba, blocks, segments: req.segments.len() });
        self.pending.push(Pending { id: req.id, op: req.op, lba: req.lba, blocks, segs: req.segments.to_vec() });
        Ok(())
    }

    fn poll(&mut self) -> Option<Completion> {
        if self.pending.is_empty() {
            return None;
        }
        let i = if self.in_order { 0 } else { self.rng.below(self.pending.len() as u64) as usize };
        let p = self.pending.remove(i);
        let result = self.run(&p);
        self.log.push(Event::Complete { id: p.id.0 });
        Some(Completion { id: p.id, result })
    }
}

/// Storage for a request queue.
pub struct Tables {
    pub requests: Vec<RequestSlot>,
    pub device: Vec<DeviceSlot>,
    pub segments: Vec<IoBuf>,
    pub parts: Vec<Part>,
}

impl Tables {
    pub fn new(requests: usize, device: usize, segments_each: usize) -> Tables {
        Tables {
            requests: vec![RequestSlot::EMPTY; requests],
            device: vec![DeviceSlot::EMPTY; device],
            segments: vec![IoBuf::EMPTY; device * segments_each],
            parts: vec![Part::EMPTY; device * segments_each],
        }
    }

    pub fn storage(&mut self) -> QueueStorage<'_> {
        QueueStorage {
            requests: &mut self.requests,
            device: &mut self.device,
            segments: &mut self.segments,
            parts: &mut self.parts,
        }
    }
}
