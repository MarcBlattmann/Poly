//! Tests: byte-range locks, asynchronous I/O, pipes and cancellation.

use super::*;

#[test]
fn byte_range_locks_against_io_and_each_other() {
    let mut t = setup();
    let a = t.file("\\??\\C:\\locked");
    let b = t.file("\\??\\C:\\locked");
    t.write(a, &[1u8; 100], Some(0));
    assert_eq!(t.lock(a, 0, 10, 0, true, true), STATUS_SUCCESS);
    // Another handle cannot read or write the region, the owner can.
    assert_eq!(t.read(b, 5, Some(0)).0, STATUS_FILE_LOCK_CONFLICT);
    assert_eq!(t.write(b, b"x", Some(9)).0, STATUS_FILE_LOCK_CONFLICT);
    assert_eq!(t.read(b, 5, Some(10)).0, STATUS_SUCCESS);
    assert_eq!(t.read(a, 5, Some(0)).0, STATUS_SUCCESS);
    assert_eq!(t.write(a, b"y", Some(0)).0, STATUS_SUCCESS);
    // The same handle with another key is another owner.
    let iosb = t.f.alloc(16);
    let buf = t.f.alloc(4);
    let off = t.f.put_u64(0);
    let key = t.f.put(&5u32.to_le_bytes());
    assert_eq!(t.f.call(Service::ReadFile, &[a, 0, 0, 0, iosb, buf, 4, off, key]), STATUS_FILE_LOCK_CONFLICT);
    // Conflicting lock requests.
    assert_eq!(t.lock(b, 5, 10, 0, true, false), STATUS_LOCK_NOT_GRANTED);
    assert_eq!(t.lock(a, 0, 10, 0, true, true), STATUS_LOCK_NOT_GRANTED);
    assert_eq!(t.unlock(a, 0, 5, 0), STATUS_RANGE_NOT_LOCKED);
    assert_eq!(t.unlock(b, 0, 10, 0), STATUS_RANGE_NOT_LOCKED);
    assert_eq!(t.unlock(a, 0, 10, 0), STATUS_SUCCESS);
    assert_eq!(t.lock(b, 5, 10, 0, true, false), STATUS_SUCCESS);
    // A shared lock: everyone reads, nobody writes.
    assert_eq!(t.read(a, 5, Some(5)).0, STATUS_SUCCESS);
    assert_eq!(t.write(b, b"z", Some(6)).0, STATUS_FILE_LOCK_CONFLICT);
    assert_eq!(t.lock(a, 8, 2, 0, true, false), STATUS_SUCCESS);
    // The whole file, as LockFileEx(0, 0, MAXDWORD, MAXDWORD).
    assert_eq!(t.lock(a, 0, u64::MAX, 0, true, true), STATUS_LOCK_NOT_GRANTED);
    assert_eq!(t.lock(a, 1, u64::MAX, 0, true, true), STATUS_INVALID_LOCK_RANGE);
    // Closing b drops its lock.
    t.close(b);
    assert_eq!(t.write(a, b"w", Some(12)).0, STATUS_SUCCESS);
    // Locks need read or write access.
    let (_, c, _) = t.create("\\??\\C:\\locked", FILE_READ_ATTRIBUTES | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.lock(c, 50, 1, 0, true, true), STATUS_ACCESS_DENIED);
    t.close(c);
    t.close(a);
}

#[test]
fn append_only_handles_write_at_the_end() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\log.txt");
    t.write(h, b"one,", None);
    t.close(h);
    let (s, h, _) = t.create("\\??\\C:\\log.txt", FILE_APPEND_DATA | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.write(h, b"two", Some(0)), (STATUS_SUCCESS, 3));
    t.close(h);
    let h = t.file("\\??\\C:\\log.txt");
    assert_eq!(t.write(h, b"!", Some(FILE_WRITE_TO_END_OF_FILE)), (STATUS_SUCCESS, 1));
    assert_eq!(t.read(h, 100, Some(0)).1, b"one,two!");
    t.close(h);
    // Writes need write access.
    let (_, h, _) = t.create("\\??\\C:\\log.txt", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.write(h, b"x", Some(0)).0, STATUS_ACCESS_DENIED);
    t.close(h);
}

#[test]
fn asynchronous_read_completes_through_the_worker_event_and_apc() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\async.bin");
    t.write(h, b"0123456789", None);
    t.close(h);
    // An asynchronous handle (no FILE_SYNCHRONOUS_IO_*).
    let (s, h, _) = t.create("\\??\\C:\\async.bin", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    // Without a ByteOffset: refused.
    assert_eq!(t.read(h, 4, None).0, STATUS_INVALID_PARAMETER);
    // An event, an APC, an offset.
    let ev_out = t.f.alloc(8);
    assert_eq!(t.f.call(Service::CreateEvent, &[ev_out, 0x1F0003, 0, 0, 1]), STATUS_SUCCESS);
    let ev = t.f.u64_at(ev_out);
    let buf = t.f.alloc(8);
    let iosb = t.f.alloc(16);
    let off = t.f.put_u64(2);
    let s = t.f.call(Service::ReadFile, &[h, ev, 0x7000, 0xC0DE, iosb, buf, 4, off, 0]);
    assert_eq!(s, STATUS_PENDING);
    // The event was reset when the request was made.
    let event = t.f.sys.objects.object_by_handle(&t.f.caller(), poly_ntobj::Handle(ev), 0, None).unwrap();
    assert!(!t.f.sys.objects.is_ready_for(event, crate::tests::TID));
    assert_eq!(t.f.sys.io.pending_irps(), 1);
    assert_eq!(t.f.sys.io.take_kicks(), [1]);
    assert_eq!(t.run_worker(1), 1);
    let done = t.drain();
    assert_eq!(done.len(), 1);
    assert_eq!(t.f.u32_at(iosb), STATUS_SUCCESS.0);
    assert_eq!(t.f.u64_at(iosb + 8), 4);
    assert_eq!(t.f.bytes_at(buf, 4), b"2345");
    assert!(t.f.sys.objects.is_ready_for(event, crate::tests::TID));
    let (thread, apc) = *t.f.apcs.last().unwrap();
    assert_eq!(thread, crate::tests::TID);
    assert_eq!((apc.routine, apc.arg1, apc.arg2, apc.arg3), (0x7000, 0xC0DE, iosb, 0));
    // At the end of the file: STATUS_END_OF_FILE through the status block.
    let off = t.f.put_u64(10);
    assert_eq!(t.f.call(Service::ReadFile, &[h, 0, 0, 0, iosb, buf, 4, off, 0]), STATUS_PENDING);
    t.run_worker(1);
    let done = t.drain();
    assert_eq!(done[0].status, STATUS_END_OF_FILE);
    assert_eq!(t.f.u32_at(iosb), STATUS_END_OF_FILE.0);
    // Without an event the file object is signalled.
    let file = t.f.sys.objects.object_by_handle(&t.f.caller(), poly_ntobj::Handle(h), 0, None).unwrap();
    assert!(t.f.sys.objects.is_ready_for(file, crate::tests::TID));
    t.close(h);
    t.close(ev);
}

#[test]
fn asynchronous_write_and_cancel_before_it_runs() {
    let mut t = setup();
    let (s, h, _) = t.create("\\??\\C:\\aw.bin", RW, SHARE_ALL, FILE_CREATE, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let data = t.f.put(b"async!");
    let iosb = t.f.alloc(16);
    let off = t.f.put_u64(0);
    assert_eq!(t.f.call(Service::WriteFile, &[h, 0, 0, 0, iosb, data, 6, off, 0]), STATUS_PENDING);
    t.run_worker(1);
    t.drain();
    assert_eq!(t.f.u64_at(iosb + 8), 6);
    // A queued request cancelled before the worker gets to it.
    let iosb2 = t.f.alloc(16);
    assert_eq!(t.f.call(Service::WriteFile, &[h, 0, 0, 0, iosb2, data, 6, off, 0]), STATUS_PENDING);
    let cancel_iosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::CancelIoFile, &[h, cancel_iosb]), STATUS_SUCCESS);
    assert_eq!(t.run_worker(1), 0);
    let done = t.drain();
    assert_eq!(done[0].status, STATUS_CANCELLED);
    assert_eq!(t.f.u32_at(iosb2), STATUS_CANCELLED.0);
    // Nothing left: CancelIoFileEx finds nothing.
    assert_eq!(t.f.call(Service::CancelIoFileEx, &[h, 0, cancel_iosb]), STATUS_NOT_FOUND);
    t.close(h);
}

fn pipe(t: &mut T, name: &str) -> u64 {
    let (s, h, _) = t.create(&alloc::format!("\\Device\\PolyPipe\\{name}"), RW, SHARE_ALL, FILE_OPEN_IF, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    h
}

#[test]
fn pipe_reads_wait_for_writes_and_cancel() {
    let mut t = setup();
    let r = pipe(&mut t, "loop");
    let w = pipe(&mut t, "LOOP");
    let buf = t.f.alloc(16);
    let iosb = t.f.alloc(16);
    // Nothing there: the read waits.
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0x9000, 0x42, iosb, buf, 16, 0, 0]), STATUS_PENDING);
    assert!(t.drain().is_empty());
    // A write completes it.
    assert_eq!(t.write(w, b"ping", None).0, STATUS_SUCCESS);
    let done = t.drain();
    assert_eq!(done.len(), 1);
    assert_eq!((done[0].status, done[0].information), (STATUS_SUCCESS, 4));
    assert_eq!(t.f.bytes_at(buf, 4), b"ping");
    assert_eq!(t.f.apcs.last().unwrap().1.arg1, 0x42);
    // Data waiting: a read completes at once.
    t.write(w, b"pong", None);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb, buf, 2, 0, 0]), STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.f.bytes_at(buf, 2), b"po");
    assert_eq!(t.f.sys.io.pipe_buffered(&crate::io::names::fold(&poly_ntobj::name::wide("loop"))), Some(2));
    t.read(r, 2, None);
    t.drain();
    // A waiting read cancelled by its thread.
    let iosb2 = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb2, buf, 16, 0, 0]), STATUS_PENDING);
    let ci = t.f.alloc(16);
    assert_eq!(t.f.call(Service::CancelIoFile, &[r, ci]), STATUS_SUCCESS);
    let done = t.drain();
    assert_eq!(done[0].status, STATUS_CANCELLED);
    assert_eq!(t.f.u32_at(iosb2), STATUS_CANCELLED.0);
    // CancelIoFileEx by status block.
    let iosb3 = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb3, buf, 16, 0, 0]), STATUS_PENDING);
    assert_eq!(t.f.call(Service::CancelIoFileEx, &[r, iosb2, ci]), STATUS_NOT_FOUND);
    assert_eq!(t.f.call(Service::CancelIoFileEx, &[r, iosb3, ci]), STATUS_SUCCESS);
    assert_eq!(t.drain()[0].status, STATUS_CANCELLED);
    // Closing the handle cancels what waits on it.
    let iosb4 = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb4, buf, 16, 0, 0]), STATUS_PENDING);
    t.close(r);
    assert_eq!(t.drain()[0].status, STATUS_CANCELLED);
    t.close(w);
    assert_eq!(t.f.sys.io.pending_irps(), 0);
}

#[test]
fn pipe_reads_end_when_the_last_writer_closes() {
    let mut t = setup();
    let name = r"\Device\PolyPipe\anon";
    // A read-only end first: its read waits, as no writer came yet.
    let (s, r, _) = t.create(name, FILE_READ_DATA | SYNCHRONIZE, SHARE_ALL, FILE_CREATE, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let buf = t.f.alloc(16);
    let iosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb, buf, 16, 0, 0]), STATUS_PENDING);
    // Two writers; the data written stays readable after both close.
    let (s, w1, _) = t.create(name, FILE_WRITE_DATA | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (_, w2, _) = t.create(name, FILE_WRITE_DATA | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, 0, 0);
    assert_eq!(t.write(w1, b"ab", None).0, STATUS_SUCCESS);
    assert_eq!(t.drain()[0].status, STATUS_SUCCESS);
    t.write(w2, b"cd", None);
    t.close(w1);
    // A writer is left: an empty pipe still waits.
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb, buf, 2, 0, 0]), STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.f.bytes_at(buf, 2), b"cd");
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb, buf, 16, 0, 0]), STATUS_PENDING);
    // The last writer closes: the waiting read ends, and so do new ones.
    t.close(w2);
    assert_eq!(t.drain()[0].status, STATUS_PIPE_BROKEN);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb, buf, 16, 0, 0]), STATUS_PIPE_BROKEN);
    t.close(r);
    assert_eq!(t.f.sys.io.pending_irps(), 0);
}

#[test]
fn waiting_lock_granted_on_unlock() {
    let mut t = setup();
    let a = t.file("\\??\\C:\\wl");
    let (s, b, _) = t.create("\\??\\C:\\wl", RW, SHARE_ALL, FILE_OPEN, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.lock(a, 0, 4, 0, true, true), STATUS_SUCCESS);
    // Asynchronous handle, wait for the lock.
    assert_eq!(t.lock(b, 2, 4, 0, false, true), STATUS_PENDING);
    assert!(t.drain().is_empty());
    assert_eq!(t.unlock(a, 0, 4, 0), STATUS_SUCCESS);
    let done = t.drain();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].status, STATUS_SUCCESS);
    assert_eq!(t.lock(a, 3, 1, 0, true, false), STATUS_LOCK_NOT_GRANTED);
    // A lock granted because the holder closed.
    let (_, c, _) = t.create("\\??\\C:\\wl", RW, SHARE_ALL, FILE_OPEN, 0, 0);
    assert_eq!(t.lock(c, 2, 1, 0, false, true), STATUS_PENDING);
    t.close(b);
    let done = t.drain();
    assert_eq!(done[0].status, STATUS_SUCCESS);
    t.close(c);
    t.close(a);
}

#[test]
fn thread_exit_cancels_without_apc() {
    let mut t = setup();
    let r = pipe(&mut t, "t");
    let buf = t.f.alloc(16);
    let iosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0x9000, 0, iosb, buf, 16, 0, 0]), STATUS_PENDING);
    let apcs = t.f.apcs.len();
    assert_eq!(t.f.sys.io.cancel_thread(crate::tests::TID), 1);
    assert_eq!(t.drain()[0].status, STATUS_CANCELLED);
    assert_eq!(t.f.apcs.len(), apcs);
    t.close(r);
}

#[test]
fn unattached_and_non_file_handles() {
    let mut f = Fake::new();
    let out = f.alloc(8);
    let iosb = f.alloc(16);
    let oa = f.attrs("\\??\\C:\\x", CI);
    let s = f.call(Service::CreateFile, &[out, u64::from(RW), oa, iosb, 0, 0, 0, u64::from(FILE_CREATE), 0, 0, 0]);
    assert_eq!(s, STATUS_OBJECT_PATH_NOT_FOUND);
    // An event handle is not a file handle.
    let ev = f.alloc(8);
    f.call(Service::CreateEvent, &[ev, 0x1F0003, 0, 0, 0]);
    let h = f.u64_at(ev);
    let s = f.call(Service::ReadFile, &[h, 0, 0, 0, iosb, iosb, 1, 0, 0]);
    assert_eq!(s, STATUS_OBJECT_TYPE_MISMATCH);
    assert!(!IoState::new().is_attached());
}
