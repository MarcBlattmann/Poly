//! Tests: named pipes and I/O completion ports (ADR 0169).

use super::*;

const PIPE: &str = "\\??\\pipe\\";
const MSG: u32 = FILE_PIPE_MESSAGE_TYPE;
const UNLIMITED: u32 = 0xFFFF_FFFF;

impl T {
    /// `NtCreateNamedPipeFile`: (status, handle, information).
    #[allow(clippy::too_many_arguments)]
    fn np_create(
        &mut self,
        name: &str,
        access: u32,
        share: u32,
        disp: u32,
        options: u32,
        ty: u32,
        read_mode: u32,
        max: u32,
        quota: u32,
    ) -> (NtStatus, u64, u64) {
        let out = self.f.alloc(8);
        let iosb = self.f.alloc(16);
        let oa = self.f.attrs(&alloc::format!("{PIPE}{name}"), CI);
        let timeout = self.f.put_u64((-500_000i64) as u64);
        let s = self.f.call(
            Service::CreateNamedPipeFile,
            &[
                out,
                u64::from(access),
                oa,
                iosb,
                u64::from(share),
                u64::from(disp),
                u64::from(options),
                u64::from(ty),
                u64::from(read_mode),
                0,
                u64::from(max),
                u64::from(quota),
                u64::from(quota),
                timeout,
            ],
        );
        (s, self.f.u64_at(out), self.f.u64_at(iosb + 8))
    }

    /// A duplex message or byte pipe server with 4096-byte quotas.
    fn np_server(&mut self, name: &str, message: bool, options: u32) -> u64 {
        let ty = u32::from(message);
        let (s, h, _) = self.np_create(name, RW | SYNCHRONIZE, 3, FILE_CREATE, options, ty, ty, UNLIMITED, 4096);
        assert_eq!(s, STATUS_SUCCESS, "{name}");
        h
    }

    fn np_client(&mut self, name: &str, options: u32) -> u64 {
        let (s, h, _) =
            self.create(&alloc::format!("{PIPE}{name}"), RW | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, options, 0);
        assert_eq!(s, STATUS_SUCCESS, "{name}");
        h
    }

    fn np_open(&mut self, name: &str, access: u32) -> NtStatus {
        self.create(&alloc::format!("{PIPE}{name}"), access, SHARE_ALL, FILE_OPEN, SYNC, 0).0
    }

    /// `NtFsControlFile` on a handle: (status, output, iosb information).
    fn fsctl(&mut self, h: u64, code: u32, input: &[u8], out_len: usize) -> (NtStatus, Vec<u8>, u64) {
        let (s, out, info, _) = self.fsctl_ev(h, 0, code, input, out_len);
        (s, out, info)
    }

    fn fsctl_ev(
        &mut self,
        h: u64,
        event: u64,
        code: u32,
        input: &[u8],
        out_len: usize,
    ) -> (NtStatus, Vec<u8>, u64, u64) {
        let iosb = self.f.alloc(16);
        let inp = if input.is_empty() { 0 } else { self.f.put(input) };
        let out = self.f.alloc(out_len.max(1));
        let s = self.f.call(
            Service::FsControlFile,
            &[h, event, 0, 0, iosb, u64::from(code), inp, input.len() as u64, out, out_len as u64],
        );
        let n = self.f.u64_at(iosb + 8) as usize;
        (s, self.f.bytes_at(out, n.min(out_len)), n as u64, iosb)
    }

    fn event(&mut self) -> u64 {
        let out = self.f.alloc(8);
        assert_eq!(self.f.call(Service::CreateEvent, &[out, 0x1F0003, 0, 1, 0]), STATUS_SUCCESS);
        self.f.u64_at(out)
    }

    fn event_set(&mut self, e: u64) -> bool {
        let id = self.f.sys.objects.object_by_handle(&self.f.caller(), poly_ntobj::Handle(e), 0, None).unwrap();
        self.f.sys.objects.is_ready_for(id, crate::tests::TID)
    }

    /// An asynchronous read request with `iosb` as its APC context: (status, buffer, iosb).
    fn aread(&mut self, h: u64, len: usize) -> (NtStatus, u64, u64) {
        let buf = self.f.alloc(len.max(1));
        let iosb = self.f.alloc(16);
        let s = self.f.call(Service::ReadFile, &[h, 0, 0, iosb, iosb, buf, len as u64, 0, 0]);
        (s, buf, iosb)
    }

    /// A synchronous read, with the kernel's delivery of the data: (status, bytes).
    fn npread(&mut self, h: u64, len: usize, _offset: Option<i64>) -> (NtStatus, Vec<u8>) {
        let buf = self.f.alloc(len.max(1));
        let iosb = self.f.alloc(16);
        let s = self.f.call(Service::ReadFile, &[h, 0, 0, 0, iosb, buf, len as u64, 0, 0]);
        self.drain();
        let n = self.f.u64_at(iosb + 8) as usize;
        (s, self.f.bytes_at(buf, n.min(len)))
    }

    fn pipe_state(&mut self, h: u64) -> u32 {
        let (s, b) = self.query(h, FILE_PIPE_LOCAL_INFORMATION, 36);
        assert_eq!(s, STATUS_SUCCESS);
        u32::from_le_bytes(b[28..32].try_into().unwrap())
    }

    fn pipe_mode(&mut self, h: u64, message: bool) -> NtStatus {
        let mut b = u32::from(message).to_le_bytes().to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        self.set(h, FILE_PIPE_INFORMATION, &b)
    }
}

#[test]
fn byte_pipe_connects_and_carries_data_both_ways() {
    let mut t = setup();
    let s = t.np_server("echo", false, SYNC);
    assert_eq!(t.pipe_state(s), FILE_PIPE_LISTENING_STATE);
    // An open of a missing pipe, and a read on an instance nobody connected to.
    assert_eq!(t.np_open("nope", RW | SYNCHRONIZE), STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(t.npread(s, 4, None).0, STATUS_PIPE_LISTENING);
    assert_eq!(t.write(s, b"x", None).0, STATUS_PIPE_LISTENING);
    let c = t.np_client("ECHO", SYNC);
    assert_eq!(t.pipe_state(s), FILE_PIPE_CONNECTED_STATE);
    // A second client finds nothing listening.
    assert_eq!(t.np_open("echo", RW | SYNCHRONIZE), STATUS_PIPE_NOT_AVAILABLE);
    assert_eq!(t.write(c, b"hello", None), (STATUS_SUCCESS, 5));
    assert_eq!(t.npread(s, 16, None), (STATUS_SUCCESS, b"hello".to_vec()));
    assert_eq!(t.write(s, b"world!", None), (STATUS_SUCCESS, 6));
    assert_eq!(t.npread(c, 3, None), (STATUS_SUCCESS, b"wor".to_vec()));
    assert_eq!(t.npread(c, 16, None), (STATUS_SUCCESS, b"ld!".to_vec()));
    t.drain();
    // The client closes: the instance stays taken, the server's read ends.
    t.close(c);
    assert_eq!(t.pipe_state(s), FILE_PIPE_CLOSING_STATE);
    assert_eq!(t.npread(s, 4, None).0, STATUS_PIPE_BROKEN);
    assert_eq!(t.write(s, b"x", None).0, STATUS_PIPE_CLOSING);
    assert_eq!(t.np_open("echo", RW | SYNCHRONIZE), STATUS_PIPE_NOT_AVAILABLE);
    // Disconnecting makes it free for ConnectNamedPipe, not for clients.
    assert_eq!(t.fsctl(s, FSCTL_PIPE_DISCONNECT, &[], 0).0, STATUS_SUCCESS);
    assert_eq!(t.fsctl(s, FSCTL_PIPE_DISCONNECT, &[], 0).0, STATUS_PIPE_DISCONNECTED);
    assert_eq!(t.npread(s, 4, None).0, STATUS_PIPE_DISCONNECTED);
    assert_eq!(t.np_open("echo", RW | SYNCHRONIZE), STATUS_PIPE_NOT_AVAILABLE);
    t.close(s);
    assert_eq!(t.np_open("echo", RW | SYNCHRONIZE), STATUS_OBJECT_NAME_NOT_FOUND);
}

#[test]
fn creating_pipes_checks_names_instances_and_directions() {
    let mut t = setup();
    let rws = RW | SYNCHRONIZE;
    // FILE_CREATE of an existing name is refused, a further instance is not.
    let (s1, h1, info) = t.np_create("multi", rws, 3, FILE_CREATE, SYNC, 0, 0, 2, 4096);
    assert_eq!((s1, info), (STATUS_SUCCESS, FILE_CREATED));
    assert_eq!(t.np_create("multi", rws, 3, FILE_CREATE, SYNC, 0, 0, 2, 4096).0, STATUS_ACCESS_DENIED);
    let (s2, h2, info) = t.np_create("multi", rws, 3, FILE_OPEN_IF, SYNC, 0, 0, 2, 4096);
    assert_eq!((s2, info), (STATUS_SUCCESS, FILE_OPENED));
    // The limit, another direction, and room made by closing an instance.
    assert_eq!(t.np_create("multi", rws, 3, FILE_OPEN_IF, SYNC, 0, 0, 2, 4096).0, STATUS_INSTANCE_NOT_AVAILABLE);
    assert_eq!(t.np_create("multi", rws, 1, FILE_OPEN_IF, SYNC, 0, 0, 2, 4096).0, STATUS_ACCESS_DENIED);
    t.close(h2);
    assert_eq!(t.np_create("multi", rws, 3, FILE_OPEN_IF, SYNC, 0, 0, 2, 4096).0, STATUS_SUCCESS);
    // A message read mode on a byte pipe is refused for the first instance
    // of a name only: a further instance is created.
    t.close(h1);
    assert_eq!(t.np_create("multi", rws, 3, FILE_OPEN_IF, SYNC, 0, 1, 2, 4096).0, STATUS_SUCCESS);
    // Bad parameters.
    assert_eq!(t.np_create("bad", rws, 3, FILE_CREATE, SYNC, 0, 1, 2, 4096).0, STATUS_INVALID_PARAMETER);
    assert_eq!(t.np_create("bad", rws, 3, FILE_CREATE, SYNC, 0, 0, 0, 4096).0, STATUS_INVALID_PARAMETER);
    assert_eq!(t.np_create("bad", rws, 3, FILE_OPEN, SYNC, 0, 0, 1, 4096).0, STATUS_OBJECT_NAME_NOT_FOUND);
    // Outside the pipe device.
    let out = t.f.alloc(8);
    let iosb = t.f.alloc(16);
    let oa = t.f.attrs("\\??\\C:\\pipe", CI);
    let args = [out, u64::from(rws), oa, iosb, 3, 2, u64::from(SYNC), 0, 0, 0, 1, 0, 0, 0];
    assert_eq!(t.f.call(Service::CreateNamedPipeFile, &args), STATUS_INVALID_DEVICE_REQUEST);
    // An inbound pipe refuses a reading client and a writing one opens.
    let (_, inb, _) = t.np_create("inb", FILE_GENERIC_READ, 2, FILE_CREATE, SYNC, 0, 0, 1, 4096);
    assert_eq!(t.np_open("inb", FILE_GENERIC_READ), STATUS_ACCESS_DENIED);
    let (st, w, _) = t.create(&alloc::format!("{PIPE}inb"), FILE_GENERIC_WRITE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(st, STATUS_SUCCESS);
    // The server end of an inbound pipe cannot write.
    assert_eq!(t.write(inb, b"x", None).0, STATUS_ACCESS_DENIED);
    assert_eq!(t.write(w, b"abc", None), (STATUS_SUCCESS, 3));
    assert_eq!(t.npread(inb, 8, None), (STATUS_SUCCESS, b"abc".to_vec()));
    assert_eq!(t.npread(w, 8, None).0, STATUS_ACCESS_DENIED);
}

#[test]
fn message_pipes_keep_boundaries_and_peek() {
    let mut t = setup();
    let s = t.np_server("msg", true, SYNC);
    let c = t.np_client("msg", SYNC);
    // The client starts in byte read mode; the server was made in message mode.
    assert_eq!(t.query(c, FILE_PIPE_INFORMATION, 8).1, [0u8; 8]);
    assert_eq!(t.query(s, FILE_PIPE_INFORMATION, 8).1, [1, 0, 0, 0, 0, 0, 0, 0]);
    t.write(c, b"one", None);
    t.write(c, b"second!", None);
    // A peek: state 3, 10 bytes available, 2 messages, first of 3 bytes.
    let (st, out, n) = t.fsctl(s, FSCTL_PIPE_PEEK, &[], 16 + 2);
    assert_eq!(st, STATUS_BUFFER_OVERFLOW);
    assert_eq!(n, 18);
    let h: Vec<u32> = out[..16].chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(h, [3, 10, 2, 3]);
    assert_eq!(&out[16..], b"on");
    // The server reads messages one at a time; a short buffer gets the
    // first bytes and a warning, then the rest.
    assert_eq!(t.npread(s, 2, None), (STATUS_BUFFER_OVERFLOW, b"on".to_vec()));
    assert_eq!(t.npread(s, 16, None), (STATUS_SUCCESS, b"e".to_vec()));
    assert_eq!(t.npread(s, 16, None), (STATUS_SUCCESS, b"second!".to_vec()));
    // The client in byte mode takes across messages; in message mode, one.
    t.write(s, b"ab", None);
    t.write(s, b"cd", None);
    assert_eq!(t.npread(c, 16, None), (STATUS_SUCCESS, b"abcd".to_vec()));
    assert_eq!(t.pipe_mode(c, true), STATUS_SUCCESS);
    t.write(s, b"ab", None);
    t.write(s, b"cd", None);
    assert_eq!(t.npread(c, 16, None), (STATUS_SUCCESS, b"ab".to_vec()));
    // Zero-length messages are messages.
    t.write(s, b"", None);
    assert_eq!(t.npread(c, 16, None), (STATUS_SUCCESS, b"cd".to_vec()));
    assert_eq!(t.npread(c, 16, None), (STATUS_SUCCESS, Vec::new()));
    // Message read mode is refused on a byte pipe.
    let bs = t.np_server("bytes", false, SYNC);
    assert_eq!(t.pipe_mode(bs, true), STATUS_INVALID_PARAMETER);
    let _ = MSG;
}

#[test]
fn asynchronous_requests_listen_read_write_and_cancel() {
    let mut t = setup();
    let s = t.np_server("async", false, 0);
    // A listen waits; a client's open completes it and signals the event.
    let ev = t.event();
    let (st, _, _, iosb) = t.fsctl_ev(s, ev, FSCTL_PIPE_LISTEN, &[], 0);
    assert_eq!(st, STATUS_PENDING);
    assert!(!t.event_set(ev));
    let c = t.np_client("async", 0);
    t.drain();
    assert!(t.event_set(ev));
    assert_eq!(t.f.u32_at(iosb), STATUS_SUCCESS.0);
    // Now connected: a listen says so.
    assert_eq!(t.fsctl(s, FSCTL_PIPE_LISTEN, &[], 0).0, STATUS_PIPE_CONNECTED);
    // A read waits for data and a write completes it.
    let (st, buf, iosb) = t.aread(s, 16);
    assert_eq!(st, STATUS_PENDING);
    let wbuf = t.f.put(b"ping");
    let wiosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::WriteFile, &[c, 0, 0, 0, wiosb, wbuf, 4, 0, 0]), STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.f.u32_at(iosb), STATUS_SUCCESS.0);
    assert_eq!(t.f.u64_at(iosb + 8), 4);
    assert_eq!(t.f.bytes_at(buf, 4), b"ping");
    // A pending read is cancelled; closing the client fails the next one.
    let (st, _, iosb) = t.aread(s, 16);
    assert_eq!(st, STATUS_PENDING);
    let ci = t.f.alloc(16);
    assert_eq!(t.f.call(Service::CancelIoFileEx, &[s, iosb, ci]), STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.f.u32_at(iosb), STATUS_CANCELLED.0);
    let (st, _, iosb) = t.aread(s, 16);
    assert_eq!(st, STATUS_PENDING);
    t.close(c);
    t.drain();
    assert_eq!(t.f.u32_at(iosb), STATUS_PIPE_BROKEN.0);
    // A disconnect fails the client's pending request with "not connected".
    t.fsctl(s, FSCTL_PIPE_DISCONNECT, &[], 0);
    let ev = t.event();
    let (st, _, _, iosb) = t.fsctl_ev(s, ev, FSCTL_PIPE_LISTEN, &[], 0);
    assert_eq!(st, STATUS_PENDING);
    let c2 = t.np_client("async", 0);
    let (st, _, iosb2) = t.aread(c2, 8);
    assert_eq!(st, STATUS_PENDING);
    t.drain();
    assert_eq!(t.f.u32_at(iosb), STATUS_SUCCESS.0);
    t.fsctl(s, FSCTL_PIPE_DISCONNECT, &[], 0);
    t.drain();
    assert_eq!(t.f.u32_at(iosb2), STATUS_PIPE_DISCONNECTED.0);
    assert_eq!(t.npread(c2, 1, None).0, STATUS_PIPE_DISCONNECTED);
}

#[test]
fn writes_beyond_the_quota_wait_for_the_reader() {
    let mut t = setup();
    let (st, s, _) = t.np_create("small", RW | SYNCHRONIZE, 3, FILE_CREATE, 0, 0, 0, 1, 16);
    assert_eq!(st, STATUS_SUCCESS);
    let c = t.np_client("small", 0);
    let data = alloc::vec![7u8; 100];
    let buf = t.f.put(&data);
    let iosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::WriteFile, &[c, 0, 0, 0, iosb, buf, 100, 0, 0]), STATUS_PENDING);
    assert!(t.drain().is_empty());
    // Read it in pieces: the write completes with the last.
    let mut got = 0;
    while got < 100 {
        let (st, _, riosb) = t.aread(s, 40);
        assert_eq!(st, STATUS_SUCCESS);
        t.drain();
        got += t.f.u64_at(riosb + 8) as usize;
        assert_eq!(t.f.u32_at(iosb) == STATUS_SUCCESS.0 && t.f.u64_at(iosb + 8) == 100, got == 100);
    }
    // A small write fits at once.
    let small = t.f.put(b"hello");
    assert_eq!(t.f.call(Service::WriteFile, &[c, 0, 0, 0, iosb, small, 5, 0, 0]), STATUS_SUCCESS);
    // A pending write is cancelled and its data is gone.
    assert_eq!(t.f.call(Service::WriteFile, &[c, 0, 0, 0, iosb, buf, 100, 0, 0]), STATUS_PENDING);
    let ci = t.f.alloc(16);
    assert_eq!(t.f.call(Service::CancelIoFile, &[c, ci]), STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.f.u32_at(iosb), STATUS_CANCELLED.0);
    assert_eq!(t.npread(s, 200, None), (STATUS_SUCCESS, b"hello".to_vec()));
}

#[test]
fn transceive_writes_and_reads_a_reply() {
    let mut t = setup();
    let s = t.np_server("trans", true, 0);
    let c = t.np_client("trans", 0);
    // A byte-mode client is refused.
    assert_eq!(t.fsctl(c, FSCTL_PIPE_TRANSCEIVE, b"abcd", 16).0, STATUS_INVALID_PIPE_STATE);
    assert_eq!(t.pipe_mode(c, true), STATUS_SUCCESS);
    // The reply is not there yet: pending; the server reads and answers.
    let ev = t.event();
    let (st, _, _, iosb) = t.fsctl_ev(c, ev, FSCTL_PIPE_TRANSCEIVE, b"abcd", 8);
    assert_eq!(st, STATUS_PENDING);
    let (st, buf, _) = t.aread(s, 16);
    assert_eq!(st, STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.f.bytes_at(buf, 4), b"abcd");
    let reply = t.f.put(b"R:abcd");
    let wiosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::WriteFile, &[s, 0, 0, 0, wiosb, reply, 6, 0, 0]), STATUS_SUCCESS);
    t.drain();
    assert!(t.event_set(ev));
    assert_eq!(t.f.u32_at(iosb), STATUS_SUCCESS.0);
    assert_eq!(t.f.u64_at(iosb + 8), 6);
    // The server end is gone: no transaction (not the write's status).
    t.close(s);
    assert_eq!(t.fsctl(c, FSCTL_PIPE_TRANSCEIVE, b"abcd", 16).0, STATUS_INVALID_PIPE_STATE);
    // Not a pipe: an event.
    let e = t.event();
    assert_eq!(t.fsctl(e, FSCTL_PIPE_LISTEN, &[], 0).0, STATUS_OBJECT_TYPE_MISMATCH);
}

#[test]
fn wait_for_a_pipe_finds_a_listening_instance() {
    let mut t = setup();
    let (s, root, _) = t.create("\\??\\pipe\\", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let wait = |name: &str, timeout: i64| {
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut b = timeout.to_le_bytes().to_vec();
        b.extend_from_slice(&(units.len() as u32).to_le_bytes());
        b.push(1);
        b.extend_from_slice(&[0, 0, 0]);
        b.extend_from_slice(&units);
        b
    };
    assert_eq!(t.fsctl(root, FSCTL_PIPE_WAIT, &wait("w", -10_000), 0).0, STATUS_OBJECT_NAME_NOT_FOUND);
    let _s = t.np_server("w", false, SYNC);
    assert_eq!(t.fsctl(root, FSCTL_PIPE_WAIT, &wait("W", -10_000), 0).0, STATUS_SUCCESS);
    let _c = t.np_client("w", SYNC);
    // Busy: the wait times out (the fake kernel's clock does not advance,
    // so the limit is zero here).
    assert_eq!(t.fsctl(root, FSCTL_PIPE_WAIT, &wait("w", 0), 0).0, STATUS_IO_TIMEOUT);
}

// ----- completion ports

impl T {
    fn port(&mut self) -> u64 {
        let out = self.f.alloc(8);
        assert_eq!(self.f.call(Service::CreateIoCompletion, &[out, 0x1F0003, 0, 0]), STATUS_SUCCESS);
        self.f.u64_at(out)
    }

    fn post(&mut self, port: u64, key: u64, ctx: u64, status: u32, info: u64) -> NtStatus {
        self.f.call(Service::SetIoCompletion, &[port, key, ctx, u64::from(status), info])
    }

    /// `NtRemoveIoCompletion` with a zero timeout: (status, key, ctx, status, info).
    fn take(&mut self, port: u64) -> (NtStatus, u64, u64, u32, u64) {
        let key = self.f.alloc(8);
        let ctx = self.f.alloc(8);
        let iosb = self.f.alloc(16);
        let zero = self.f.put_u64(0);
        let s = self.f.call(Service::RemoveIoCompletion, &[port, key, ctx, iosb, zero]);
        if s != STATUS_SUCCESS {
            return (s, 0, 0, 0, 0);
        }
        (s, self.f.u64_at(key), self.f.u64_at(ctx), self.f.u32_at(iosb), self.f.u64_at(iosb + 8))
    }

    fn depth(&mut self, port: u64) -> u32 {
        let out = self.f.alloc(4);
        assert_eq!(self.f.call(Service::QueryIoCompletion, &[port, 0, out, 4, 0]), STATUS_SUCCESS);
        self.f.u32_at(out)
    }

    fn associate(&mut self, file: u64, port: u64, key: u64) -> NtStatus {
        let mut b = port.to_le_bytes().to_vec();
        b.extend_from_slice(&key.to_le_bytes());
        self.set(file, FILE_COMPLETION_INFORMATION, &b)
    }
}

#[test]
fn ports_queue_packets_in_order_and_check_rights() {
    let mut t = setup();
    let p = t.port();
    assert_eq!(t.take(p).0, STATUS_TIMEOUT);
    assert_eq!(t.depth(p), 0);
    for i in 1..=3u64 {
        assert_eq!(t.post(p, 0x10 + i, 0x20 + i, 0, i), STATUS_SUCCESS);
    }
    assert_eq!(t.depth(p), 3);
    assert_eq!(t.take(p), (STATUS_SUCCESS, 0x11, 0x21, 0, 1));
    assert_eq!(t.depth(p), 2);
    // A failure status comes out as put in.
    assert_eq!(t.post(p, 1, 2, 0xC000_0005, 9), STATUS_SUCCESS);
    assert_eq!(t.take(p), (STATUS_SUCCESS, 0x12, 0x22, 0, 2));
    assert_eq!(t.take(p), (STATUS_SUCCESS, 0x13, 0x23, 0, 3));
    assert_eq!(t.take(p), (STATUS_SUCCESS, 1, 2, 0xC000_0005, 9));
    // Ports are objects of their own type.
    let e = t.event();
    assert_eq!(t.post(e, 0, 0, 0, 0), STATUS_OBJECT_TYPE_MISMATCH);
    assert_eq!(t.post(0xFFFF_0000, 0, 0, 0, 0), STATUS_INVALID_HANDLE);
    // Query: the class and the length.
    let out = t.f.alloc(8);
    assert_eq!(t.f.call(Service::QueryIoCompletion, &[p, 1, out, 4, 0]), STATUS_INVALID_INFO_CLASS);
    assert_eq!(t.f.call(Service::QueryIoCompletion, &[p, 0, out, 8, 0]), STATUS_INFO_LENGTH_MISMATCH);
    // A duplicate with only query rights cannot post or take.
    let dup = t.f.alloc(8);
    assert_eq!(t.f.call(Service::DuplicateObject, &[u64::MAX, p, u64::MAX, dup, 1, 0, 0]), STATUS_SUCCESS);
    let q = t.f.u64_at(dup);
    assert_eq!(t.post(q, 0, 0, 0, 0), STATUS_ACCESS_DENIED);
    assert_eq!(t.take(q).0, STATUS_ACCESS_DENIED);
    assert_eq!(t.depth(q), 0);
}

#[test]
fn files_on_a_port_post_packets_when_requests_complete() {
    let mut t = setup();
    let p = t.port();
    let s = t.np_server("iocp", false, 0);
    let c = t.np_client("iocp", 0);
    // Only asynchronous handles, and only once.
    let sync_s = t.np_server("iocp2", false, SYNC);
    assert_eq!(t.associate(sync_s, p, 1), STATUS_INVALID_PARAMETER);
    assert_eq!(t.associate(s, p, 0x5E), STATUS_SUCCESS);
    assert_eq!(t.associate(s, p, 0x5E), STATUS_INVALID_PARAMETER);
    assert_eq!(t.associate(c, p, 0xC1), STATUS_SUCCESS);
    // A read that waits queues its packet when it completes.
    let ov = t.f.alloc(32);
    let buf = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[s, 0, 0, ov, ov, buf, 16, 0, 0]), STATUS_PENDING);
    assert_eq!(t.depth(p), 0);
    let wbuf = t.f.put(b"data");
    let wiosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::WriteFile, &[c, 0, 0, wiosb, wiosb, wbuf, 4, 0, 0]), STATUS_SUCCESS);
    t.drain();
    // Two packets: the server's read, which the write completed, and the
    // write itself.
    assert_eq!(t.take(p), (STATUS_SUCCESS, 0x5E, ov, 0, 4));
    assert_eq!(t.take(p), (STATUS_SUCCESS, 0xC1, wiosb, 0, 4));
    // A request with a NULL context (the event's low bit) queues nothing.
    assert_eq!(t.f.call(Service::ReadFile, &[s, 0, 0, 0, ov, buf, 16, 0, 0]), STATUS_PENDING);
    let w2 = t.f.alloc(16);
    assert_eq!(t.f.call(Service::WriteFile, &[c, 0, 0, 0, w2, wbuf, 4, 0, 0]), STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.take(p).0, STATUS_TIMEOUT);
    // Skip-on-success: a read that finds data queues nothing.
    assert_eq!(t.set(s, FILE_IO_COMPLETION_NOTIFICATION_INFORMATION, &1u32.to_le_bytes()), STATUS_SUCCESS);
    assert_eq!(t.set(s, FILE_IO_COMPLETION_NOTIFICATION_INFORMATION, &8u32.to_le_bytes()), STATUS_INVALID_PARAMETER);
    assert_eq!(t.f.call(Service::WriteFile, &[c, 0, 0, 0, w2, wbuf, 4, 0, 0]), STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.f.call(Service::ReadFile, &[s, 0, 0, ov, ov, buf, 16, 0, 0]), STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.take(p).0, STATUS_TIMEOUT);
    // A request that waits still queues; a close fails it as a broken pipe.
    assert_eq!(t.f.call(Service::ReadFile, &[s, 0, 0, ov, ov, buf, 16, 0, 0]), STATUS_PENDING);
    t.close(c);
    t.drain();
    assert_eq!(t.take(p), (STATUS_SUCCESS, 0x5E, ov, STATUS_PIPE_BROKEN.0, 0));
    // ReadFileEx on a handle with a port is refused.
    assert_eq!(t.f.call(Service::ReadFile, &[s, 0, 0x7000, ov, ov, buf, 16, 0, 0]), STATUS_INVALID_PARAMETER);
    t.close(s);
    assert_eq!(t.depth(p), 0);
}

#[test]
fn ports_accept_odd_rights_and_refuse_a_name_without_a_leaf() {
    let mut t = setup();
    let out = t.f.alloc(8);
    // A right the type does not have is accepted.
    assert_eq!(t.f.call(Service::CreateIoCompletion, &[out, 0x8000 | 0x3, 0, 0]), STATUS_SUCCESS);
    // GENERIC_READ and GENERIC_WRITE map to the query and modify rights.
    assert_eq!(t.f.call(Service::CreateIoCompletion, &[out, 0x8000_0000, 0, 1]), STATUS_SUCCESS);
    // A directory name with no leaf is no name.
    let oa = t.f.attrs(r"\BaseNamedObjects\", CI);
    let s = t.f.call(Service::CreateIoCompletion, &[out, 0x1F0003, oa, 0]);
    assert_eq!(s, STATUS_OBJECT_NAME_INVALID);
}
