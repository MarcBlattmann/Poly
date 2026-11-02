//! One Wayland connection ("The Wayland Protocol", chapter "Wire Format"):
//! requests are encoded into an output buffer and sent together, with at
//! most one handle (a `fd` argument travels out of band); events are cut
//! from the input buffer, and handles that came along wait in a queue for
//! the event whose `fd` argument they are.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use poly_rt::abi::Handle;
use poly_rt::net::UnixStream;
use poly_rt::poll;

/// `wl_display`, always object 1.
pub const DISPLAY: u32 = 1;

/// A request argument.
#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    /// `int`, `uint`, `object`, `new_id`, `fixed`: one word.
    U(u32),
    /// `string`: its length with the NUL, the bytes, the NUL, padding.
    S(&'a str),
}

/// A request's bytes: the header (sender, size << 16 | opcode), then the
/// arguments, each a whole number of words.
pub fn encode(object: u32, opcode: u16, args: &[Arg<'_>]) -> Vec<u8> {
    let mut body = Vec::new();
    for a in args {
        match *a {
            Arg::U(v) => body.extend_from_slice(&v.to_ne_bytes()),
            Arg::S(s) => {
                let n = s.len() + 1;
                body.extend_from_slice(&(n as u32).to_ne_bytes());
                body.extend_from_slice(s.as_bytes());
                body.resize(body.len() + 1 + (4 - n % 4) % 4, 0);
            }
        }
    }
    let size = 8 + body.len() as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&object.to_ne_bytes());
    out.extend_from_slice(&(size << 16 | u32::from(opcode)).to_ne_bytes());
    out.extend_from_slice(&body);
    out
}

/// An event: its sender, opcode and argument words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub object: u32,
    pub opcode: u16,
    pub args: Vec<u32>,
}

impl Event {
    /// Word `at`, 0 past the end.
    pub fn arg(&self, at: usize) -> u32 {
        self.args.get(at).copied().unwrap_or(0)
    }

    /// The string whose length word is at `at` ("" for a null or broken one).
    pub fn string(&self, at: usize) -> String {
        let n = self.arg(at) as usize;
        let bytes: Vec<u8> = self.args.iter().skip(at + 1).flat_map(|w| w.to_ne_bytes()).collect();
        if n == 0 || n > bytes.len() {
            return String::new();
        }
        String::from_utf8_lossy(&bytes[..n - 1]).into_owned()
    }
}

/// Cut the first whole event off `buf`, if there is one. A header whose
/// size is below the header's own is a broken stream: the rest is dropped.
pub fn take_event(buf: &mut Vec<u8>) -> Option<Event> {
    if buf.len() < 8 {
        return None;
    }
    let word = |i: usize| u32::from_ne_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
    let (object, second) = (word(0), word(4));
    let size = (second >> 16) as usize;
    if size < 8 {
        buf.clear();
        return None;
    }
    if buf.len() < size {
        return None;
    }
    let args = (8..size).step_by(4).filter(|i| i + 4 <= size).map(word).collect();
    buf.drain(..size);
    Some(Event { object, opcode: second as u16, args })
}

/// A connection to the compositor.
#[derive(Debug)]
pub struct Conn {
    sock: UnixStream,
    out: Vec<u8>,
    out_handle: Option<Handle>,
    input: Vec<u8>,
    handles: VecDeque<Handle>,
    next_id: u32,
}

impl Conn {
    /// Connect to `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` (`wayland-0` without it).
    pub fn connect() -> Result<Conn, String> {
        let dir = poly_rt::env::var("XDG_RUNTIME_DIR").ok_or_else(|| String::from("no XDG_RUNTIME_DIR"))?;
        let name = poly_rt::env::var("WAYLAND_DISPLAY").unwrap_or("wayland-0");
        let path = alloc::format!("{dir}/{name}");
        let sock = UnixStream::connect(&path).map_err(|e| alloc::format!("connect to {path}: {e:?}"))?;
        Ok(Conn { sock, out: Vec::new(), out_handle: None, input: Vec::new(), handles: VecDeque::new(), next_id: 2 })
    }

    /// A new client object ID (1 is `wl_display`).
    pub fn new_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Queue a request.
    pub fn request(&mut self, object: u32, opcode: u16, args: &[Arg<'_>]) {
        self.out.extend_from_slice(&encode(object, opcode, args));
    }

    /// Queue a request whose `fd` argument is `handle` (sent with the next
    /// flush, which comes at once so it goes with this request's bytes).
    pub fn request_with_handle(&mut self, object: u32, opcode: u16, args: &[Arg<'_>], handle: Handle) {
        self.flush_quiet();
        self.request(object, opcode, args);
        self.out_handle = Some(handle);
        self.flush_quiet();
    }

    /// Send what is queued.
    pub fn flush(&mut self) -> Result<(), String> {
        let mut at = 0;
        while at < self.out.len() {
            let handles: Vec<Handle> = self.out_handle.take().into_iter().collect();
            match self.sock.send_with_handles(&self.out[at..], &handles) {
                Ok(n) => at += n,
                Err(poly_rt::Error::Interrupted) => {
                    self.out_handle = handles.first().copied();
                }
                Err(e) => {
                    self.out.clear();
                    return Err(alloc::format!("send: {e:?}"));
                }
            }
        }
        self.out.clear();
        self.out_handle = None;
        Ok(())
    }

    fn flush_quiet(&mut self) {
        if let Err(e) = self.flush() {
            poly_rt::println!("wire: {e}");
        }
    }

    /// The next event, waiting up to `timeout` (`None`: as long as it takes)
    /// for more bytes; `None` on timeout or when the compositor went away.
    pub fn next_event(&mut self, timeout: Option<Duration>) -> Option<Event> {
        loop {
            if let Some(e) = take_event(&mut self.input) {
                return Some(e);
            }
            if let Some(t) = timeout {
                let mut p = [poll::entry(self.sock.handle(), poll::READABLE)];
                match poll::poll(&mut p, Some(t)) {
                    Ok(0) => return None,
                    Ok(_) => {}
                    Err(poly_rt::Error::Interrupted) => continue,
                    Err(_) => return None,
                }
            }
            let mut chunk = [0u8; 16384];
            let mut got = Vec::new();
            match self.sock.recv_with_handles(&mut chunk, &mut got) {
                Ok(0) => return None,
                Ok(n) => {
                    self.input.extend_from_slice(&chunk[..n]);
                    self.handles.extend(got);
                }
                Err(poly_rt::Error::Interrupted) => {}
                Err(_) => return None,
            }
        }
    }

    /// The oldest handle that came with the events (an `fd` argument).
    pub fn take_handle(&mut self) -> Option<Handle> {
        self.handles.pop_front()
    }
}

/// `wl_registry.bind`'s arguments for a global: its name, then the new ID
/// with its interface and version (a `new_id` without an interface in the
/// XML).
pub fn bind_args<'a>(name: u32, iface: &'a str, version: u32, id: u32) -> [Arg<'a>; 4] {
    [Arg::U(name), Arg::S(iface), Arg::U(version), Arg::U(id)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(b: &[u8]) -> Vec<u32> {
        b.chunks(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    #[test]
    fn encodes_a_request_with_a_string() {
        // wl_registry.bind(5, "wl_shm", 1, 7) on object 2.
        let b = encode(2, 0, &bind_args(5, "wl_shm", 1, 7));
        // "wl_shm" + NUL is 7 bytes, padded to 8.
        assert_eq!(b.len(), 8 + 4 + 4 + 8 + 4 + 4);
        let w = words(&b);
        assert_eq!(w[0], 2);
        assert_eq!(w[1], (b.len() as u32) << 16);
        assert_eq!(w[2], 5);
        assert_eq!(w[3], 7);
        assert_eq!(&b[16..23], b"wl_shm\0");
        assert_eq!(b[23], 0);
        assert_eq!(&w[6..], [1, 7]);
    }

    #[test]
    fn a_string_of_a_whole_word_gets_a_padding_word() {
        // "abc" + NUL is exactly 4 bytes: no more padding.
        assert_eq!(encode(1, 3, &[Arg::S("abc")]).len(), 8 + 4 + 4);
        assert_eq!(encode(1, 3, &[Arg::S("abcd")]).len(), 8 + 4 + 8);
        assert_eq!(encode(1, 3, &[Arg::S("")]).len(), 8 + 4 + 4);
    }

    #[test]
    fn events_are_cut_whole() {
        let mut buf = encode(3, 0, &[Arg::U(9), Arg::S("wl_seat"), Arg::U(5)]);
        let second = encode(4, 2, &[]);
        buf.extend_from_slice(&second[..4]);
        let e = take_event(&mut buf).unwrap();
        assert_eq!((e.object, e.opcode, e.arg(0)), (3, 0, 9));
        assert_eq!(e.string(1), "wl_seat");
        assert_eq!(e.arg(4), 5);
        assert_eq!(e.arg(99), 0);
        // Half a header: nothing yet.
        assert_eq!(take_event(&mut buf), None);
        buf.extend_from_slice(&second[4..]);
        assert_eq!(take_event(&mut buf), Some(Event { object: 4, opcode: 2, args: Vec::new() }));
        assert!(buf.is_empty());
    }

    #[test]
    fn a_broken_header_drops_the_stream() {
        let mut buf = [1u32, 4 << 16].iter().flat_map(|w| w.to_ne_bytes()).collect::<Vec<u8>>();
        assert_eq!(take_event(&mut buf), None);
        assert!(buf.is_empty());
    }

    #[test]
    fn broken_strings_are_empty() {
        let e = Event { object: 1, opcode: 0, args: alloc::vec![40, 0x41] };
        assert_eq!(e.string(0), "");
        assert_eq!(Event::default().string(0), "");
    }
}
