//! The request queue in front of one block device; see [`RequestQueue`].

use crate::{BlockDevice, DeviceInfo, IoBuf, IoError, IoQueue, Op, Request, RequestId, SubmitError, user_error};
use core::ptr::NonNull;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    /// Accepted and not finished: waiting, partly or wholly at the device.
    Queued,
    /// Finished; the result waits for its owner.
    Done,
}

/// Storage for one user request. The caller provides a table of them
/// (`[RequestSlot::EMPTY; N]`) and otherwise leaves them alone.
#[derive(Clone, Copy, Debug)]
pub struct RequestSlot {
    state: State,
    /// Bumped whenever the slot is freed, so old tickets are recognised.
    generation: u32,
    op: Op,
    lba: u64,
    /// Blocks to transfer. A flush counts as one unit, sent as a whole.
    units: u32,
    /// Units handed to the device so far, from the start.
    dispatched: u32,
    /// Device requests carrying part of this one that are not back yet.
    outstanding: u16,
    buf: IoBuf,
    /// Submission order.
    seq: u64,
    /// The first error any piece reported.
    result: Result<(), IoError>,
}

impl RequestSlot {
    pub const EMPTY: RequestSlot = RequestSlot {
        state: State::Free,
        generation: 0,
        op: Op::Read,
        lba: 0,
        units: 0,
        dispatched: 0,
        outstanding: 0,
        buf: IoBuf::EMPTY,
        seq: 0,
        result: Ok(()),
    };

    fn pending(&self) -> bool {
        self.state == State::Queued && self.dispatched < self.units
    }

    /// First block not yet dispatched.
    fn next_lba(&self) -> u64 {
        self.lba + self.dispatched as u64
    }

    /// Record that one piece is back (or never left), and finish the
    /// request if nothing else is outstanding. True if it finished now.
    #[must_use]
    fn settle(&mut self, result: Result<(), IoError>) -> bool {
        if let Err(e) = result {
            if self.result.is_ok() {
                self.result = Err(e);
            }
            // Do not send the rest.
            self.dispatched = self.units;
        }
        if self.outstanding == 0 && self.dispatched == self.units {
            self.state = State::Done;
            return true;
        }
        false
    }
}

/// Storage for one request at the device. Its id at the device is its index.
#[derive(Clone, Copy, Debug)]
pub struct DeviceSlot {
    busy: bool,
    /// Pieces of user requests it carries, in the `parts` table.
    parts: u16,
}

impl DeviceSlot {
    pub const EMPTY: DeviceSlot = DeviceSlot { busy: false, parts: 0 };
}

/// One user request's piece of a device request.
#[derive(Clone, Copy, Debug)]
pub struct Part {
    owner: u16,
    units: u32,
}

impl Part {
    pub const EMPTY: Part = Part { owner: 0, units: 0 };
}

/// The tables a [`RequestQueue`] works in.
#[derive(Debug)]
pub struct QueueStorage<'a> {
    /// User requests that are queued, or finished and not yet collected. At
    /// most 65535.
    pub requests: &'a mut [RequestSlot],
    /// Requests the queue keeps at the device at once. At most 65535.
    pub device: &'a mut [DeviceSlot],
    /// Segment lists, shared out evenly among the device slots.
    pub segments: &'a mut [IoBuf],
    /// At least as long as `segments`.
    pub parts: &'a mut [Part],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// The device reports limits that fail [`DeviceInfo::is_valid`].
    BadDevice,
    /// No request slots, or more than 65535.
    Requests,
    /// No device slots, or more than 65535.
    DeviceSlots,
    /// A device slot's share of `segments` (or the device's segment limit)
    /// cannot carry one block, or `parts` is shorter than `segments`.
    Segments,
}

/// Counters, for diagnostics and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueStats {
    /// User requests accepted.
    pub requests: u64,
    /// Requests sent to the device.
    pub device_requests: u64,
    /// User requests that joined a device request another one started.
    pub merged: u64,
    /// Device requests that ended before the user request they carried.
    pub split: u64,
    /// User requests finished (successfully or not), collected or not. A
    /// change after [`RequestQueue::process`] means there are results for
    /// waiters to pick up.
    pub finished: u64,
}

/// Names an enqueued request until its result is collected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket {
    slot: u16,
    generation: u32,
}

/// The request queue in front of one [`BlockDevice`].
///
/// Users [`enqueue`](RequestQueue::enqueue) requests of any length and get a
/// [`Ticket`]. Nothing reaches the device until [`RequestQueue::process`]
/// runs, so requests enqueued together can be merged ("plugging"). Each
/// `process` call collects finished device requests and starts new ones
/// until neither makes progress; a polling kernel calls it in a wait loop,
/// an interrupt-driven one from the device's interrupt handler, and then
/// wakes the owners of [`RequestQueue::finished`] requests (or, when
/// [`QueueStats::finished`] moved, every waiter, each of which collects its
/// own result with [`RequestQueue::take`]). Through [`IoQueue`] a
/// [`crate::BufferCache`] works on either kind of kernel.
///
/// # What goes to the device, in which order
///
/// - **Ordering.** The device may run and complete requests in any order,
///   so the queue holds a request back while it conflicts with an earlier
///   one that has not finished: a read or write overlapping an earlier write,
///   a write overlapping an earlier read, a write after an earlier flush, a
///   flush after an earlier write or flush. The outcome is the same as if
///   every request ran alone, in submission order. Reads pass reads and
///   flushes freely.
/// - **Flushes** are barriers for writes: a flush starts once every earlier
///   write has completed, and later writes wait for it. That is what a
///   journal needs to order its commit record after its data. On a device
///   without a write cache a flush finishes as soon as it may start.
/// - **Elevator.** Of the requests free to go, flushes go first, then the
///   one with the lowest start block at or after the last block dispatched,
///   wrapping around to the lowest one ("C-LOOK"): the disk sweeps one way.
///   A stream of requests just ahead of the sweep can delay others for a
///   while; nothing bounds that wait yet.
/// - **Merging.** A device request keeps taking the next queued request of
///   the same kind that starts where it ends, up to the device's limits.
///   Segments that follow each other in memory become one segment.
/// - **Splitting.** A request the device cannot take in one piece (too many
///   blocks, segments or bytes per segment) goes out in pieces, on block
///   boundaries, and finishes when the last piece is back. If a piece fails,
///   the rest is not sent and the request fails with the first error.
///
/// Every decision scans the request table (and conflict checks scan it
/// again), so the cost grows with the square of its size. That is fine for
/// the dozens of slots a disk needs; a larger queue would want sorted
/// indices.
///
/// # Memory
///
/// Buffers handed to the queue must be contiguous in the device's address
/// space as well as the CPU's (memory from the kernel's direct map is), and
/// two buffers that are adjacent for the CPU may be handed to the device as
/// one segment. A driver whose address translation fails a segment fails
/// the request with [`IoError::BadBuffer`] rather than corrupting anything.
pub struct RequestQueue<'a, D> {
    dev: D,
    info: DeviceInfo,
    requests: &'a mut [RequestSlot],
    device: &'a mut [DeviceSlot],
    segments: &'a mut [IoBuf],
    parts: &'a mut [Part],
    /// Entries of `segments` and `parts` per device slot.
    chunk: usize,
    /// Segments one device request may use: the chunk or the device's
    /// limit, whichever is smaller.
    max_segments: usize,
    next_seq: u64,
    /// The elevator's position: the block after the last one dispatched.
    head: u64,
    in_flight: usize,
    stats: QueueStats,
}

impl<D> core::fmt::Debug for RequestQueue<'_, D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RequestQueue")
            .field("info", &self.info)
            .field("in_flight", &self.in_flight)
            .field("head", &self.head)
            .field("stats", &self.stats)
            .finish()
    }
}

impl<'a, D: BlockDevice> RequestQueue<'a, D> {
    /// A queue for `dev`, which must have nothing in flight and is used
    /// only through the queue from now on.
    pub fn new(dev: D, storage: QueueStorage<'a>) -> Result<RequestQueue<'a, D>, SetupError> {
        let info = dev.info();
        if !info.is_valid() {
            return Err(SetupError::BadDevice);
        }
        let QueueStorage { requests, device, segments, parts } = storage;
        if requests.is_empty() || requests.len() > u16::MAX as usize {
            return Err(SetupError::Requests);
        }
        if device.is_empty() || device.len() > u16::MAX as usize {
            return Err(SetupError::DeviceSlots);
        }
        let chunk = segments.len() / device.len();
        let max_segments = chunk.min(info.max_segments as usize);
        if parts.len() < segments.len()
            || (max_segments as u64) * (info.max_segment_len as u64) < info.block_size as u64
        {
            return Err(SetupError::Segments);
        }
        requests.fill(RequestSlot::EMPTY);
        device.fill(DeviceSlot::EMPTY);
        Ok(RequestQueue {
            dev,
            info,
            requests,
            device,
            segments,
            parts,
            chunk,
            max_segments,
            next_seq: 0,
            head: 0,
            in_flight: 0,
            stats: QueueStats::default(),
        })
    }

    pub const fn info(&self) -> &DeviceInfo {
        &self.info
    }

    pub const fn stats(&self) -> &QueueStats {
        &self.stats
    }

    pub const fn device(&self) -> &D {
        &self.dev
    }

    /// The device, for driver-specific calls (interrupt control, say).
    /// Submitting or polling through it would confuse the queue.
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.dev
    }

    /// Requests at the device right now.
    pub const fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// No request is queued or at the device (finished ones may still wait
    /// to be collected).
    pub fn is_idle(&self) -> bool {
        self.requests.iter().all(|r| r.state != State::Queued)
    }

    /// Queue a read or write of the whole blocks in `buf`, starting at block
    /// `lba`, or a flush (with [`IoBuf::EMPTY`]; see also
    /// [`RequestQueue::enqueue_flush`]). Requests the device could never do
    /// are refused here. [`SubmitError::Busy`] means every request slot is
    /// taken.
    ///
    /// # Safety
    ///
    /// Until the request's result has been collected (or the queue is
    /// dropped without that ever happening, which leaks the request), `buf`
    /// must stay valid, for writes for a read and for reads for a write, and
    /// nothing but the queue and the device may write it (nor, for a read,
    /// read it).
    pub unsafe fn enqueue(&mut self, op: Op, lba: u64, buf: IoBuf) -> Result<Ticket, SubmitError> {
        let units = self.check(op, lba, buf).map_err(SubmitError::Rejected)?;
        let Some(i) = self.requests.iter().position(|r| r.state == State::Free) else {
            return Err(SubmitError::Busy);
        };
        let r = &mut self.requests[i];
        let lba = if op == Op::Flush { 0 } else { lba };
        *r = RequestSlot {
            state: State::Queued,
            generation: r.generation,
            op,
            lba,
            units,
            dispatched: 0,
            outstanding: 0,
            buf,
            seq: self.next_seq,
            result: Ok(()),
        };
        self.next_seq += 1;
        self.stats.requests += 1;
        Ok(Ticket { slot: i as u16, generation: r.generation })
    }

    /// Queue a flush: it finishes once every write enqueued before it is
    /// durable.
    pub fn enqueue_flush(&mut self) -> Result<Ticket, SubmitError> {
        // SAFETY: a flush transfers no memory.
        unsafe { self.enqueue(Op::Flush, 0, IoBuf::EMPTY) }
    }

    /// Units the request takes, if the device can ever do it.
    fn check(&self, op: Op, lba: u64, buf: IoBuf) -> Result<u32, IoError> {
        if op == Op::Flush {
            return if buf.len == 0 { Ok(1) } else { Err(IoError::BadRequest) };
        }
        if op == Op::Write && self.info.read_only {
            return Err(IoError::ReadOnly);
        }
        let bs = self.info.block_size as usize;
        if buf.len == 0 || !buf.len.is_multiple_of(bs) {
            return Err(IoError::BadRequest);
        }
        let blocks = u32::try_from(buf.len / bs).map_err(|_| IoError::BadRequest)?;
        match lba.checked_add(blocks as u64) {
            Some(end) if end <= self.info.blocks => Ok(blocks),
            _ => Err(IoError::OutOfRange),
        }
    }

    /// Collect finished device requests and start queued ones, until
    /// neither makes progress. Returns whether anything happened.
    pub fn process(&mut self) -> bool {
        let mut any = false;
        loop {
            let harvested = self.harvest();
            let dispatched = self.dispatch();
            if harvested + dispatched == 0 {
                return any;
            }
            any = true;
        }
    }

    /// The result of the request, if it has finished; the ticket is then
    /// spent. Does not run the queue.
    ///
    /// # Panics
    ///
    /// If the ticket was already spent.
    pub fn take(&mut self, t: Ticket) -> Option<Result<(), IoError>> {
        let r = &mut self.requests[t.slot as usize];
        assert!(r.generation == t.generation && r.state != State::Free, "spent ticket {t:?}");
        if r.state != State::Done {
            return None;
        }
        let result = r.result;
        r.state = State::Free;
        r.generation = r.generation.wrapping_add(1);
        Some(result)
    }

    /// The oldest finished request, with its result; the ticket is then
    /// spent. For whoever wakes waiters after [`RequestQueue::process`].
    pub fn finished(&mut self) -> Option<(Ticket, Result<(), IoError>)> {
        let (i, _) =
            self.requests.iter().enumerate().filter(|(_, r)| r.state == State::Done).min_by_key(|(_, r)| r.seq)?;
        let t = Ticket { slot: i as u16, generation: self.requests[i].generation };
        self.take(t).map(|r| (t, r))
    }

    /// Run the queue until the request has finished, and return its result.
    /// Spins while the device works.
    ///
    /// # Panics
    ///
    /// If the ticket was already spent.
    pub fn wait(&mut self, t: Ticket) -> Result<(), IoError> {
        loop {
            if let Some(r) = self.take(t) {
                return r;
            }
            if !self.process() {
                core::hint::spin_loop();
            }
        }
    }

    /// Read whole blocks starting at `lba` into `buf`, waiting for the
    /// result. Requests from others queued meanwhile are served too.
    pub fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), IoError> {
        // SAFETY: `buf` stays borrowed until `wait` has collected the result.
        let t = unsafe { self.enqueue(Op::Read, lba, IoBuf::from_mut(buf)) }.map_err(user_error)?;
        self.wait(t)
    }

    /// Write whole blocks starting at `lba` from `buf`, waiting for the
    /// result.
    pub fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), IoError> {
        // SAFETY: `buf` stays borrowed until `wait` has collected the result,
        // and the device only reads it.
        let t = unsafe { self.enqueue(Op::Write, lba, IoBuf::from_ref(buf)) }.map_err(user_error)?;
        self.wait(t)
    }

    /// Make every write enqueued so far durable, waiting for the result.
    pub fn flush(&mut self) -> Result<(), IoError> {
        let t = self.enqueue_flush().map_err(user_error)?;
        self.wait(t)
    }

    /// The device back.
    ///
    /// # Panics
    ///
    /// If requests are queued or in flight: they would be lost with their
    /// memory still lent out.
    pub fn into_device(self) -> D {
        assert!(self.is_idle(), "request queue dropped with requests in flight");
        self.dev
    }

    /// Hand completions from the device to the requests they belong to.
    fn harvest(&mut self) -> usize {
        let mut n = 0;
        while let Some(c) = self.dev.poll() {
            let Some(d) = usize::try_from(c.id.0).ok().filter(|&d| self.device.get(d).is_some_and(|s| s.busy)) else {
                // Not ours, or already back: the driver broke its contract.
                // There is nothing to complete.
                continue;
            };
            let slot = &mut self.device[d];
            slot.busy = false;
            let base = d * self.chunk;
            for k in base..base + slot.parts as usize {
                let r = &mut self.requests[self.parts[k].owner as usize];
                r.outstanding -= 1;
                if r.settle(c.result) {
                    self.stats.finished += 1;
                }
            }
            self.in_flight -= 1;
            n += 1;
        }
        n
    }

    /// Start whatever may start, as long as the device takes it. Returns
    /// how many user requests made progress.
    fn dispatch(&mut self) -> usize {
        let mut progress = 0;
        let mut submitted = false;
        while let Some(first) = self.pick() {
            if self.requests[first].op == Op::Flush && !self.info.write_cache {
                // Every earlier write is complete, which on this device
                // means durable.
                let r = &mut self.requests[first];
                r.dispatched = 1;
                if r.settle(Ok(())) {
                    self.stats.finished += 1;
                }
                progress += 1;
                continue;
            }
            let Some(d) = self.device.iter().position(|s| !s.busy) else { break };
            let base = d * self.chunk;
            let (op, lba, segments, parts) = if self.requests[first].op == Op::Flush {
                self.parts[base] = Part { owner: first as u16, units: 1 };
                (Op::Flush, 0, 0, 1)
            } else {
                self.build(first, base)
            };
            let req = Request { id: RequestId(d as u64), op, lba, segments: &self.segments[base..base + segments] };
            // SAFETY: each segment lies inside the buffer of a queued user
            // request, whose submitter promised (`enqueue`) to leave it to
            // us until that request finishes, which is not before this
            // device request is back. No block is dispatched twice, so no
            // two device requests share memory.
            let result = unsafe { self.dev.submit(&req) };
            progress += parts;
            match result {
                Ok(()) => {
                    submitted = true;
                    let mut units = 0;
                    for p in &self.parts[base..base + parts] {
                        let r = &mut self.requests[p.owner as usize];
                        r.dispatched += p.units;
                        r.outstanding += 1;
                        units += p.units as u64;
                    }
                    let last = &self.requests[self.parts[base + parts - 1].owner as usize];
                    if last.dispatched < last.units {
                        self.stats.split += 1;
                    }
                    if op != Op::Flush {
                        self.head = lba + units;
                    }
                    self.device[d] = DeviceSlot { busy: true, parts: parts as u16 };
                    self.in_flight += 1;
                    self.stats.device_requests += 1;
                    self.stats.merged += parts as u64 - 1;
                }
                Err(SubmitError::Busy) if self.in_flight > 0 => {
                    progress -= parts;
                    break;
                }
                // Busy with nothing of ours in flight would never change, so
                // it counts as a refusal.
                Err(SubmitError::Busy) => self.fail(base, parts, IoError::Busy),
                Err(SubmitError::Rejected(e)) => self.fail(base, parts, e),
            }
        }
        if submitted {
            self.dev.kick();
        }
        progress
    }

    /// The device refused the request built at `base`: fail its owners.
    fn fail(&mut self, base: usize, parts: usize, e: IoError) {
        for k in base..base + parts {
            if self.requests[self.parts[k].owner as usize].settle(Err(e)) {
                self.stats.finished += 1;
            }
        }
    }

    /// The next request to start, if any may.
    fn pick(&self) -> Option<usize> {
        let flush = (0..self.requests.len())
            .filter(|&i| self.requests[i].pending() && self.requests[i].op == Op::Flush && !self.blocked(i))
            .min_by_key(|&i| self.requests[i].seq);
        if flush.is_some() {
            return flush;
        }
        let mut best: Option<(usize, (bool, u64, u64))> = None;
        for (i, r) in self.requests.iter().enumerate() {
            if !r.pending() || r.op == Op::Flush {
                continue;
            }
            // Ahead of the elevator first, then from the bottom again.
            let start = r.next_lba();
            let key = (start < self.head, start, r.seq);
            if best.is_some_and(|(_, k)| k <= key) || self.blocked(i) {
                continue;
            }
            best = Some((i, key));
        }
        best.map(|(i, _)| i)
    }

    /// Must request `i` wait for an earlier one?
    fn blocked(&self, i: usize) -> bool {
        let r = &self.requests[i];
        self.requests.iter().any(|e| e.state == State::Queued && e.seq < r.seq && conflicts(e, r))
    }

    /// A queued read or write that may start and begins at block `at`.
    fn find_next(&self, op: Op, at: u64) -> Option<usize> {
        (0..self.requests.len()).find(|&i| {
            let r = &self.requests[i];
            r.pending() && r.op == op && r.next_lba() == at && !self.blocked(i)
        })
    }

    /// Lay out a device request in the slot whose tables start at `base`:
    /// as much of `first` as fits, then whatever follows it on disk.
    /// Returns its operation, first block, segment count and part count.
    /// Changes nothing but the slot's tables, so a refused submission
    /// leaves no trace.
    fn build(&mut self, first: usize, base: usize) -> (Op, u64, usize, usize) {
        let bs = self.info.block_size as usize;
        let max_len = self.info.max_segment_len as usize;
        let op = self.requests[first].op;
        let lba = self.requests[first].next_lba();
        let (mut nsegs, mut nparts, mut total) = (0, 0, 0u32);
        let mut next = Some(first);
        while let Some(c) = next {
            // Every part needs a segment of its own, except when it shares
            // one with its predecessor; bound the parts the same way.
            if nparts == self.max_segments {
                break;
            }
            let r = self.requests[c];
            let mut taken = 0;
            while r.dispatched + taken < r.units && total < self.info.max_blocks {
                // SAFETY: the block lies inside the buffer (`check` made its
                // length `units` blocks), which `enqueue`'s caller promised
                // is valid memory, so the offset stays in one allocation.
                let ptr = unsafe { r.buf.ptr.add((r.dispatched + taken) as usize * bs) };
                let segs = &mut self.segments[base..base + self.max_segments];
                // SAFETY: as just said, `ptr` starts a block in the buffer.
                if !unsafe { push_block(segs, &mut nsegs, ptr, bs, max_len) } {
                    break;
                }
                taken += 1;
                total += 1;
            }
            if taken == 0 {
                break;
            }
            self.parts[base + nparts] = Part { owner: c as u16, units: taken };
            nparts += 1;
            if r.dispatched + taken < r.units {
                break;
            }
            next = self.find_next(op, lba + total as u64);
        }
        (op, lba, nsegs, nparts)
    }
}

/// Append one block at `ptr` to the segment list `segs[..*n]`: grow the last
/// segment if the block follows it in memory and it has room, else start
/// new segments of at most `max_len` bytes. False, with nothing changed, if
/// that would take more segments than `segs` has.
///
/// # Safety
///
/// `ptr` must start `bs` bytes of one allocation.
unsafe fn push_block(segs: &mut [IoBuf], n: &mut usize, ptr: NonNull<u8>, bs: usize, max_len: usize) -> bool {
    if let Some(last) = n.checked_sub(1).map(|i| &mut segs[i])
        && last.end() == ptr.as_ptr() as usize
        && last.len + bs <= max_len
    {
        last.len += bs;
        return true;
    }
    let need = bs.div_ceil(max_len);
    if *n + need > segs.len() {
        return false;
    }
    for off in (0..bs).step_by(max_len) {
        // SAFETY: `off < bs`, so this stays in the allocation (our
        // caller's promise).
        let p = unsafe { ptr.add(off) };
        segs[*n] = IoBuf { ptr: p, len: max_len.min(bs - off) };
        *n += 1;
    }
    true
}

/// Must `later` wait until `earlier` (submitted before it) has finished?
fn conflicts(earlier: &RequestSlot, later: &RequestSlot) -> bool {
    match (earlier.op, later.op) {
        (Op::Read, Op::Read) | (Op::Read, Op::Flush) | (Op::Flush, Op::Read) => false,
        (Op::Flush, _) | (_, Op::Flush) => true,
        _ => earlier.lba < later.lba + later.units as u64 && later.lba < earlier.lba + earlier.units as u64,
    }
}

/// The queue as its own user sees it: waiting runs it, spinning.
impl<D: BlockDevice> IoQueue for RequestQueue<'_, D> {
    fn info(&self) -> DeviceInfo {
        self.info
    }

    unsafe fn enqueue(&mut self, op: Op, lba: u64, buf: IoBuf) -> Result<Ticket, SubmitError> {
        // SAFETY: our caller's promise, which is the same.
        unsafe { RequestQueue::enqueue(self, op, lba, buf) }
    }

    fn wait(&mut self, t: Ticket) -> Result<(), IoError> {
        RequestQueue::wait(self, t)
    }
}

#[cfg(test)]
mod tests;
