//! Period and buffer bookkeeping for an output stream: which bytes of the
//! ring buffer the writer may fill, which are ready for the device, which
//! are with the device and which it has played.
//!
//! The ring buffer is `buffer_bytes` long, a whole number of periods of
//! `period_bytes` (the sizes given in SET PARAMETERS, §5.14.6.6.3). Three
//! byte counters run from 0 at PREPARE and never wrap in practice (64
//! bits): `written` by the writer, `submitted` to the transmit queue,
//! `played` (the device returned the message, §5.14.6.8: an output
//! message comes back once its data is consumed). Always
//! `played <= submitted <= written <= played + buffer_bytes`.
//!
//! Messages never cross a period boundary or the end of the ring, so each
//! is one contiguous piece of the buffer. A full period goes out as soon as
//! it is written. A partial period goes out only when the device would
//! otherwise run dry (nothing in flight) or the stream drains: then the
//! rest of the period follows in a message of its own, and periods line
//! up again. [`PeriodRing::pad_to_period`] fills the last period of a
//! drain with silence, so the device always gets the period it was set up
//! for at the end, and the pad is not counted as the writer's.

/// A contiguous piece of the ring buffer: byte offset and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub offset: u32,
    pub len: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeriodRing {
    buffer: u32,
    period: u32,
    written: u64,
    submitted: u64,
    played: u64,
    /// Silence added by [`pad_to_period`](Self::pad_to_period).
    padded: u64,
}

impl PeriodRing {
    /// A ring of `buffer_bytes`, a whole number (at least one) of
    /// non-empty periods.
    pub fn new(buffer_bytes: u32, period_bytes: u32) -> Option<PeriodRing> {
        if period_bytes == 0 || buffer_bytes < period_bytes || !buffer_bytes.is_multiple_of(period_bytes) {
            return None;
        }
        Some(PeriodRing { buffer: buffer_bytes, period: period_bytes, written: 0, submitted: 0, played: 0, padded: 0 })
    }

    pub const fn buffer_bytes(&self) -> u32 {
        self.buffer
    }

    pub const fn period_bytes(&self) -> u32 {
        self.period
    }

    /// Back to empty (PREPARE).
    pub fn reset(&mut self) {
        *self = PeriodRing { buffer: self.buffer, period: self.period, ..PeriodRing::default_counts() };
    }

    const fn default_counts() -> PeriodRing {
        PeriodRing { buffer: 0, period: 0, written: 0, submitted: 0, played: 0, padded: 0 }
    }

    /// Bytes the writer may add now.
    pub fn room(&self) -> u32 {
        (self.buffer as u64 - (self.written - self.played)) as u32
    }

    /// Bytes written and not yet played (in the ring or with the device).
    pub fn queued(&self) -> u64 {
        self.written - self.played
    }

    /// Bytes with the device.
    pub fn in_flight(&self) -> u64 {
        self.submitted - self.played
    }

    /// Bytes written but not yet handed to the device.
    pub fn pending(&self) -> u64 {
        self.written - self.submitted
    }

    pub const fn written(&self) -> u64 {
        self.written
    }

    pub const fn submitted(&self) -> u64 {
        self.submitted
    }

    /// Bytes the device has played, of those the writer wrote (silence
    /// padding not counted).
    pub fn played(&self) -> u64 {
        self.played.min(self.written - self.padded)
    }

    /// Where the next `n` bytes of the writer go, as up to two pieces (the
    /// second when the ring wraps); `n` is cut to [`room`](Self::room).
    /// Call [`commit`](Self::commit) once they are copied.
    pub fn write_spans(&self, n: u32) -> [Chunk; 2] {
        let n = n.min(self.room());
        let at = (self.written % self.buffer as u64) as u32;
        let first = n.min(self.buffer - at);
        [Chunk { offset: at, len: first }, Chunk { offset: 0, len: n - first }]
    }

    /// The writer filled `n` more bytes (at most [`room`](Self::room)).
    pub fn commit(&mut self, n: u32) {
        self.written += n.min(self.room()) as u64;
    }

    /// The next piece to hand to the device: the rest of the current
    /// period once it is all written, or (with `partial`, or when nothing
    /// is in flight so the device would run dry) whatever is written of it.
    pub fn next_chunk(&self, partial: bool) -> Option<Chunk> {
        let pending = self.pending();
        if pending == 0 {
            return None;
        }
        let into_period = (self.submitted % self.period as u64) as u32;
        let to_boundary = self.period - into_period;
        let offset = (self.submitted % self.buffer as u64) as u32;
        if pending >= to_boundary as u64 {
            Some(Chunk { offset, len: to_boundary })
        } else if partial || self.in_flight() == 0 {
            Some(Chunk { offset, len: pending as u32 })
        } else {
            None
        }
    }

    /// `chunk` (from [`next_chunk`](Self::next_chunk)) went to the device.
    pub fn mark_submitted(&mut self, chunk: Chunk) {
        self.submitted = (self.submitted + chunk.len as u64).min(self.written);
    }

    /// The device returned a message of `len` bytes. More than is in
    /// flight is not believed.
    pub fn complete(&mut self, len: u32) {
        self.played = (self.played + len as u64).min(self.submitted);
    }

    /// Everything handed over and played: the device has nothing left.
    pub fn is_drained(&self) -> bool {
        self.played == self.written
    }

    /// Fill the rest of the period being written with silence (for a
    /// drain): returns the piece the caller must fill with silence, already
    /// counted as written. `None` if the written data ends on a period
    /// boundary (or, which cannot happen, there is no room).
    pub fn pad_to_period(&mut self) -> Option<Chunk> {
        let into = (self.written % self.period as u64) as u32;
        if into == 0 {
            return None;
        }
        let len = self.period - into;
        if len > self.room() {
            return None;
        }
        let offset = (self.written % self.buffer as u64) as u32;
        self.written += len as u64;
        self.padded += len as u64;
        Some(Chunk { offset, len })
    }

    /// Forget what is with the device (after a RELEASE returned every
    /// message): written data not yet played is dropped too.
    pub fn drop_all(&mut self) {
        self.reset();
    }
}
