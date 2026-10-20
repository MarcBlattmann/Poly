use super::*;
use crate::sim::{Event, Rng, SimDisk, Tables};
extern crate std;
use std::collections::HashMap;
use std::{vec, vec::Vec};

/// One device request as the log saw it: when it went out and came back
/// (positions in the log).
#[derive(Clone, Copy, Debug)]
struct Span {
    op: Op,
    lba: u64,
    blocks: u32,
    segments: usize,
    submit: usize,
    complete: usize,
}

fn spans(log: &[Event]) -> Vec<Span> {
    let mut open: HashMap<u64, usize> = HashMap::new();
    let mut out: Vec<Span> = Vec::new();
    for (at, e) in log.iter().enumerate() {
        match *e {
            Event::Submit { id, op, lba, blocks, segments } => {
                open.insert(id, out.len());
                out.push(Span { op, lba, blocks, segments, submit: at, complete: usize::MAX });
            }
            Event::Complete { id } => out[open.remove(&id).unwrap()].complete = at,
        }
    }
    out
}

/// The (only) device request of kind `op` starting at block `lba`.
fn span_of(spans: &[Span], op: Op, lba: u64) -> Span {
    let mut it = spans.iter().filter(|s| s.op == op && s.lba == lba);
    let s = *it.next().unwrap();
    assert!(it.next().is_none());
    s
}

fn fill(buf: &mut [u8], tag: u8) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = tag ^ (i as u8).wrapping_mul(7);
    }
}

fn run_until_idle<D: BlockDevice>(q: &mut RequestQueue<'_, D>) {
    for _ in 0..10_000 {
        q.process();
        if q.is_idle() {
            return;
        }
    }
    panic!("queue never went idle");
}

#[test]
fn merges_neighbours_into_one_request() {
    let mut t = Tables::new(16, 4, 16);
    let mut disk = SimDisk::new(64, 512, false, 1);
    disk.in_order = true;
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    // Eight one-block writes, one buffer each, laid out in memory in disk
    // order but enqueued shuffled: one device request with one segment.
    let mut mem = vec![0u8; 8 * 512];
    fill(&mut mem, 0x5a);
    let mut tickets = Vec::new();
    for i in [3, 0, 7, 1, 6, 2, 5, 4] {
        let buf = IoBuf::from_ref(&mem[i * 512..(i + 1) * 512]);
        // SAFETY: `mem` outlives the queue's use of it (we wait below).
        tickets.push(unsafe { q.enqueue(Op::Write, 10 + i as u64, buf) }.unwrap());
    }
    run_until_idle(&mut q);
    for t in tickets {
        assert_eq!(q.take(t), Some(Ok(())));
    }
    let s = spans(&q.device().log);
    assert_eq!(s.len(), 1);
    assert_eq!((s[0].op, s[0].lba, s[0].blocks, s[0].segments), (Op::Write, 10, 8, 1));
    assert_eq!(q.stats().merged, 7);
    assert_eq!(&q.device().durable[10 * 512..18 * 512], &mem[..]);

    // Reads into blocks with gaps between them: still one request, but
    // one segment per block.
    let mut back = vec![0u8; 16 * 512];
    let mut tickets = Vec::new();
    for (i, chunk) in back.chunks_mut(1024).enumerate() {
        // SAFETY: as above.
        tickets.push(unsafe { q.enqueue(Op::Read, 10 + i as u64, IoBuf::from_mut(&mut chunk[..512])) }.unwrap());
    }
    run_until_idle(&mut q);
    for t in tickets {
        assert_eq!(q.take(t), Some(Ok(())));
    }
    let s = spans(&q.device().log);
    assert_eq!((s.len(), s[1].op, s[1].blocks, s[1].segments), (2, Op::Read, 8, 8));
    for i in 0..8 {
        assert_eq!(&back[i * 1024..i * 1024 + 512], &mem[i * 512..(i + 1) * 512]);
    }
}

#[test]
fn splits_what_the_device_cannot_take() {
    let mut t = Tables::new(8, 2, 8);
    let mut disk = SimDisk::new(64, 512, false, 2);
    disk.info.max_blocks = 4;
    disk.info.max_segments = 2;
    disk.info.max_segment_len = 1024;
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut data = vec![0u8; 16 * 512];
    fill(&mut data, 3);
    q.write(5, &data).unwrap();
    let s = spans(&q.device().log);
    assert_eq!(s.len(), 4);
    assert!(s.iter().all(|s| s.blocks == 4 && s.segments == 2));
    assert_eq!(q.stats().split, 3);
    let mut back = vec![0u8; 16 * 512];
    q.read(5, &mut back).unwrap();
    assert_eq!(back, data);

    // Segments shorter than a block: each block takes two.
    let mut t = Tables::new(8, 2, 4);
    let mut disk = SimDisk::new(64, 512, false, 3);
    disk.info.max_segment_len = 256;
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    q.write(0, &data[..3 * 512]).unwrap();
    let s = spans(&q.device().log);
    assert_eq!(s.iter().map(|s| (s.blocks, s.segments)).collect::<Vec<_>>(), [(2, 4), (1, 2)]);
    assert_eq!(&q.device().durable[..3 * 512], &data[..3 * 512]);
}

#[test]
fn elevator_sweeps_one_way() {
    let mut t = Tables::new(8, 1, 4);
    let mut disk = SimDisk::new(100, 512, false, 4);
    disk.in_order = true;
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut buf = vec![0u8; 5 * 512];
    q.read(35, &mut buf).unwrap();
    let mut mem = vec![0u8; 8 * 512];
    let mut tickets = Vec::new();
    for (i, lba) in [50, 10, 70, 30].into_iter().enumerate() {
        let b = IoBuf::from_mut(&mut mem[i * 1024..i * 1024 + 512]);
        // SAFETY: `mem` outlives the requests.
        tickets.push(unsafe { q.enqueue(Op::Read, lba, b) }.unwrap());
    }
    run_until_idle(&mut q);
    let order: Vec<u64> = spans(&q.device().log)[1..].iter().map(|s| s.lba).collect();
    assert_eq!(order, [50, 70, 10, 30]);
    // Results come back oldest first.
    let done: Vec<Ticket> = core::iter::from_fn(|| q.finished().map(|(t, r)| r.map(|_| t).unwrap())).collect();
    assert_eq!(done, tickets);
}

#[test]
fn overlapping_requests_keep_submission_order() {
    for seed in 1..40 {
        let mut t = Tables::new(8, 8, 4);
        let disk = SimDisk::new(32, 512, false, seed);
        let mut q = RequestQueue::new(disk, t.storage()).unwrap();
        let mut a = vec![0u8; 4 * 512];
        fill(&mut a, 1);
        let mut b = vec![0u8; 512];
        fill(&mut b, 2);
        let mut c = vec![0u8; 4 * 512];
        fill(&mut c, 3);
        let mut r1 = vec![0u8; 512];
        let mut r2 = vec![0u8; 4 * 512];
        // SAFETY: every buffer outlives the queue's use of it.
        let tickets = unsafe {
            [
                q.enqueue(Op::Write, 0, IoBuf::from_ref(&a)).unwrap(),
                q.enqueue(Op::Read, 2, IoBuf::from_mut(&mut r1)).unwrap(),
                q.enqueue(Op::Write, 2, IoBuf::from_ref(&b)).unwrap(),
                q.enqueue(Op::Read, 0, IoBuf::from_mut(&mut r2)).unwrap(),
                q.enqueue(Op::Write, 8, IoBuf::from_ref(&c)).unwrap(),
            ]
        };
        run_until_idle(&mut q);
        for t in tickets {
            assert_eq!(q.take(t), Some(Ok(())));
        }
        assert_eq!(r1, a[2 * 512..3 * 512]);
        assert_eq!(r2, [&a[..1024], &b[..], &a[1536..]].concat());
        let s = spans(&q.device().log);
        let (wa, rd1, wb, rd2, wc) = (
            span_of(&s, Op::Write, 0),
            span_of(&s, Op::Read, 2),
            span_of(&s, Op::Write, 2),
            span_of(&s, Op::Read, 0),
            span_of(&s, Op::Write, 8),
        );
        assert!(wa.complete < rd1.submit, "seed {seed}");
        assert!(rd1.complete < wb.submit, "seed {seed}");
        assert!(wb.complete < rd2.submit, "seed {seed}");
        // The unrelated write did not wait.
        assert!(wc.submit < wa.complete, "seed {seed}");
    }
}

#[test]
fn flush_is_a_barrier_for_writes() {
    for seed in 1..40 {
        let mut t = Tables::new(8, 8, 4);
        let disk = SimDisk::new(64, 512, true, seed);
        let mut q = RequestQueue::new(disk, t.storage()).unwrap();
        let (mut w1, mut w2, mut w3) = (vec![0u8; 1024], vec![0u8; 512], vec![0u8; 512]);
        fill(&mut w1, 1);
        fill(&mut w2, 2);
        fill(&mut w3, 3);
        let (mut r1, mut r2) = (vec![0u8; 512], vec![0u8; 512]);
        // SAFETY: every buffer outlives the queue's use of it.
        let tickets = unsafe {
            [
                q.enqueue(Op::Write, 0, IoBuf::from_ref(&w1)).unwrap(),
                q.enqueue(Op::Write, 5, IoBuf::from_ref(&w2)).unwrap(),
                q.enqueue(Op::Read, 20, IoBuf::from_mut(&mut r1)).unwrap(),
                q.enqueue_flush().unwrap(),
                q.enqueue(Op::Write, 10, IoBuf::from_ref(&w3)).unwrap(),
                q.enqueue(Op::Read, 30, IoBuf::from_mut(&mut r2)).unwrap(),
            ]
        };
        // Once the flush is back, the writes before it are durable.
        while q.take(tickets[3]).is_none() {
            q.process();
        }
        assert_eq!(&q.device().durable[..1024], &w1[..]);
        assert_eq!(&q.device().durable[5 * 512..6 * 512], &w2[..]);
        run_until_idle(&mut q);
        let s = spans(&q.device().log);
        let flush = *s.iter().find(|s| s.op == Op::Flush).unwrap();
        assert!(span_of(&s, Op::Write, 0).complete < flush.submit, "seed {seed}");
        assert!(span_of(&s, Op::Write, 5).complete < flush.submit, "seed {seed}");
        assert!(flush.complete < span_of(&s, Op::Write, 10).submit, "seed {seed}");
        // Reads do not wait for flushes.
        assert!(span_of(&s, Op::Read, 30).submit < flush.submit, "seed {seed}");
        for t in [0, 1, 2, 4, 5] {
            assert_eq!(q.take(tickets[t]), Some(Ok(())));
        }
    }
}

#[test]
fn flush_without_a_write_cache_needs_no_device_request() {
    let mut t = Tables::new(4, 2, 2);
    let mut q = RequestQueue::new(SimDisk::new(8, 512, false, 5), t.storage()).unwrap();
    let w = vec![9u8; 512];
    // SAFETY: `w` outlives the request.
    let tw = unsafe { q.enqueue(Op::Write, 1, IoBuf::from_ref(&w)) }.unwrap();
    let tf = q.enqueue_flush().unwrap();
    run_until_idle(&mut q);
    assert_eq!(q.finished(), Some((tw, Ok(()))));
    assert_eq!(q.finished(), Some((tf, Ok(()))));
    assert_eq!(q.finished(), None);
    assert_eq!(spans(&q.device().log).len(), 1);
    assert_eq!(q.flush(), Ok(()));
}

#[test]
fn a_failed_piece_fails_its_request_only() {
    let mut t = Tables::new(4, 1, 4);
    let mut disk = SimDisk::new(64, 512, false, 6);
    disk.info.max_blocks = 2;
    disk.in_order = true;
    disk.bad = vec![6];
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let data = vec![1u8; 16 * 512];
    let other = vec![2u8; 2 * 512];
    // SAFETY: both buffers outlive the requests.
    let (big, small) = unsafe {
        (
            q.enqueue(Op::Write, 0, IoBuf::from_ref(&data)).unwrap(),
            q.enqueue(Op::Write, 20, IoBuf::from_ref(&other)).unwrap(),
        )
    };
    run_until_idle(&mut q);
    assert_eq!(q.take(big), Some(Err(IoError::Media)));
    assert_eq!(q.take(small), Some(Ok(())));
    // Pieces 0-1, 2-3, 4-5, then 6-7 fails; nothing after it goes out.
    let s = spans(&q.device().log);
    let lbas: Vec<u64> = s.iter().filter(|s| s.lba < 20).map(|s| s.lba).collect();
    assert_eq!(lbas, [0, 2, 4, 6]);
    assert_eq!(&q.device().durable[20 * 512..22 * 512], &other[..]);
}

#[test]
fn refusals() {
    let mut t = Tables::new(2, 1, 2);
    let mut disk = SimDisk::new(8, 512, false, 7);
    disk.info.read_only = true;
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut buf = vec![0u8; 1024];
    assert_eq!(q.write(0, &buf), Err(IoError::ReadOnly));
    assert_eq!(q.read(7, &mut buf), Err(IoError::OutOfRange));
    assert_eq!(q.read(u64::MAX, &mut buf[..512]), Err(IoError::OutOfRange));
    assert_eq!(q.read(0, &mut buf[..100]), Err(IoError::BadRequest));
    assert_eq!(q.read(0, &mut []), Err(IoError::BadRequest));
    let mut b = [0u8; 512];
    // SAFETY: rejected, or never run (the slots fill up first).
    unsafe {
        assert_eq!(q.enqueue(Op::Flush, 0, IoBuf::from_mut(&mut b)), Err(SubmitError::Rejected(IoError::BadRequest)));
        q.enqueue(Op::Read, 0, IoBuf::from_mut(&mut b)).unwrap();
        q.enqueue(Op::Read, 1, IoBuf::from_mut(&mut b)).unwrap();
        assert_eq!(q.enqueue(Op::Read, 2, IoBuf::from_mut(&mut b)), Err(SubmitError::Busy));
    }
    assert_eq!(q.read(3, &mut b), Err(IoError::Busy));
    // `b` is lent to the queue until the requests finish.
    run_until_idle(&mut q);

    // A device that turns read-only behind the queue's back refuses the
    // write at submission; the request fails with its reason.
    let mut t = Tables::new(2, 1, 2);
    let mut q = RequestQueue::new(SimDisk::new(8, 512, false, 8), t.storage()).unwrap();
    q.device_mut().info.read_only = true;
    assert_eq!(q.write(0, &buf), Err(IoError::ReadOnly));
    assert_eq!(q.device().log.len(), 0);
}

#[test]
fn backpressure_from_a_small_device() {
    let mut t = Tables::new(32, 4, 4);
    let mut disk = SimDisk::new(64, 512, false, 9);
    disk.max_pending = 1;
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut mem = vec![0u8; 40 * 512];
    fill(&mut mem, 4);
    let mut tickets = Vec::new();
    for i in 0..20 {
        // Every other block, so nothing merges.
        let b = IoBuf::from_ref(&mem[i * 1024..i * 1024 + 512]);
        // SAFETY: `mem` outlives the requests.
        tickets.push(unsafe { q.enqueue(Op::Write, i as u64 * 3, b) }.unwrap());
    }
    while !q.is_idle() {
        q.process();
        assert!(q.in_flight() <= 1);
    }
    for (i, t) in tickets.into_iter().enumerate() {
        assert_eq!(q.take(t), Some(Ok(())));
        assert_eq!(&q.device().durable[i * 3 * 512..(i * 3 + 1) * 512], &mem[i * 1024..i * 1024 + 512]);
    }

    // A device that never has room fails requests instead of hanging.
    let mut t = Tables::new(4, 2, 2);
    let mut disk = SimDisk::new(8, 512, false, 10);
    disk.max_pending = 0;
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    assert_eq!(q.write(0, &mem[..512]), Err(IoError::Busy));
}

#[test]
#[should_panic(expected = "spent ticket")]
fn spent_tickets_are_caught() {
    let mut t = Tables::new(2, 1, 2);
    let mut q = RequestQueue::new(SimDisk::new(8, 512, false, 11), t.storage()).unwrap();
    let tf = q.enqueue_flush().unwrap();
    assert_eq!(q.wait(tf), Ok(()));
    q.take(tf);
}

#[test]
fn setup_checks() {
    let disk = || SimDisk::new(8, 512, false, 12);
    let mut t = Tables::new(0, 1, 1);
    assert_eq!(RequestQueue::new(disk(), t.storage()).err(), Some(SetupError::Requests));
    let mut t = Tables::new(1, 0, 1);
    assert_eq!(RequestQueue::new(disk(), t.storage()).err(), Some(SetupError::DeviceSlots));
    let mut t = Tables::new(1, 2, 0);
    assert_eq!(RequestQueue::new(disk(), t.storage()).err(), Some(SetupError::Segments));
    let mut t = Tables::new(1, 2, 1);
    t.parts.pop();
    assert_eq!(RequestQueue::new(disk(), t.storage()).err(), Some(SetupError::Segments));
    // Two segments of 256 bytes carry a block; one does not.
    let mut short = disk();
    short.info.max_segment_len = 256;
    let mut t = Tables::new(1, 2, 1);
    assert_eq!(RequestQueue::new(short, t.storage()).err(), Some(SetupError::Segments));
    let mut bad = disk();
    bad.info.max_blocks = 0;
    let mut t = Tables::new(1, 1, 1);
    assert_eq!(RequestQueue::new(bad, t.storage()).err(), Some(SetupError::BadDevice));
    let mut t = Tables::new(1, 1, 1);
    let q = RequestQueue::new(disk(), t.storage()).unwrap();
    q.into_device();
}

/// Random requests from several users against a device that completes in
/// random order with random limits. Each read must see exactly the writes
/// enqueued before it, and the disk must end up as the model says.
#[test]
fn randomized_against_a_model() {
    const BLOCKS: u64 = 64;
    for seed in 1..=30u64 {
        let mut rng = Rng(seed * 0x9e37_79b9);
        let bs = if rng.one_in(3) { 4096 } else { 512 };
        let mut disk = SimDisk::new(BLOCKS, bs as u32, rng.one_in(2), seed);
        disk.max_pending = 1 + rng.below(8) as usize;
        disk.info.max_blocks = 1 + rng.below(8) as u32;
        disk.info.max_segments = 1 + rng.below(4) as u16;
        disk.info.max_segment_len = if rng.one_in(2) { u32::MAX } else { (bs as u32 / 2) * (1 + rng.below(3) as u32) };
        while !disk.info.is_valid() {
            disk.info.max_segments += 1;
        }
        let mut t = Tables::new(4 + rng.below(12) as usize, 1 + rng.below(6) as usize, 1 + rng.below(6) as usize);
        let Ok(mut q) = RequestQueue::new(disk, t.storage()) else {
            // Too few segments per slot for this device's limits.
            continue;
        };
        let mut model = vec![0u8; BLOCKS as usize * bs];
        // Buffers lent to the queue, with what reads must find in them.
        let mut lent: HashMap<Ticket, (Vec<u8>, Option<Vec<u8>>)> = HashMap::new();
        let mut checked = 0;
        let mut collect = |q: &mut RequestQueue<'_, SimDisk>, lent: &mut HashMap<_, (Vec<u8>, Option<Vec<u8>>)>| {
            while let Some((t, r)) = q.finished() {
                assert_eq!(r, Ok(()), "seed {seed}");
                let (buf, expect) = lent.remove(&t).unwrap();
                if let Some(expect) = expect {
                    assert_eq!(buf, expect, "seed {seed}");
                    checked += 1;
                }
            }
        };
        for step in 0..400u64 {
            let lba = rng.below(BLOCKS);
            let len = (1 + rng.below(12)).min(BLOCKS - lba) as usize;
            let range = lba as usize * bs..(lba as usize + len) * bs;
            let (op, buf, expect) = match rng.below(10) {
                0 => (Op::Flush, Vec::new(), None),
                1..=4 => {
                    let mut buf = vec![0u8; len * bs];
                    fill(&mut buf, step as u8);
                    (Op::Write, buf, None)
                }
                _ => (Op::Read, vec![0u8; len * bs], Some(model[range.clone()].to_vec())),
            };
            let mut buf = buf;
            let io = if op == Op::Flush { IoBuf::EMPTY } else { IoBuf::from_mut(&mut buf) };
            // SAFETY: the buffer moves into `lent` (its heap memory stays
            // put) and is dropped only after its result is collected.
            match unsafe { q.enqueue(op, lba, io) } {
                Ok(t) => {
                    if op == Op::Write {
                        model[range].copy_from_slice(&buf);
                    }
                    lent.insert(t, (buf, expect));
                }
                Err(SubmitError::Busy) => {
                    q.process();
                    collect(&mut q, &mut lent);
                }
                Err(e) => panic!("seed {seed}: {e:?}"),
            }
            if rng.one_in(3) {
                q.process();
            }
            if rng.one_in(4) {
                collect(&mut q, &mut lent);
            }
        }
        run_until_idle(&mut q);
        collect(&mut q, &mut lent);
        assert!(lent.is_empty());
        assert!(checked > 0);
        let disk = q.into_device();
        for b in 0..BLOCKS {
            assert_eq!(disk.block(b), model[b as usize * bs..(b as usize + 1) * bs], "seed {seed} block {b}");
        }
    }
}
