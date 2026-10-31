use super::*;
use crate::protocol::{wl_compositor, wl_display, wl_registry, wl_shm, wl_surface, xdg_toplevel};
use alloc::string::ToString;
use alloc::vec;

fn words(ws: &[u32]) -> Vec<u8> {
    ws.iter().flat_map(|w| w.to_ne_bytes()).collect()
}

fn no_fds() -> Option<Fd> {
    None
}

#[test]
fn headers_round_trip() {
    let h = Header { object: ObjectId(3), opcode: 7, size: 24 };
    assert_eq!(h.encode(), words(&[3, 24 << 16 | 7]).as_slice());
    assert_eq!(Header::parse(&h.encode()), Some(h));
    assert_eq!(Header::parse(&[0; 7]), None);
}

#[test]
fn fixed_point() {
    assert_eq!(Fixed::from_int(3).0, 768);
    assert_eq!(Fixed(-1).floor(), -1);
    assert_eq!(Fixed::from_f64(1.5).0, 384);
    assert_eq!(Fixed(640).to_f64(), 2.5);
}

/// `wl_registry.bind` carries a `new_id` without an interface: interface
/// name, version, then the ID; strings count their NUL and are padded.
#[test]
fn encodes_strings_and_untyped_new_ids_as_the_book_describes() {
    let bind = wl_registry::Request::Bind {
        name: 5,
        id: NewObject { interface: "wl_compositor".into(), version: 4, id: ObjectId(9) },
    };
    let (mut out, mut fds) = (Vec::new(), Vec::new());
    encode(&mut out, &mut fds, ObjectId(2), bind.opcode(), &bind.clone().into_args()).unwrap();
    let mut want = words(&[2, 0, 5, 14]);
    want.extend_from_slice(b"wl_compositor\0\0\0");
    want.extend_from_slice(&words(&[4, 9]));
    let size = want.len() as u32;
    want[4..8].copy_from_slice(&(size << 16).to_ne_bytes());
    assert_eq!(out, want);
    assert!(fds.is_empty());
    // And back.
    let h = Header::parse(&out).unwrap();
    let desc = &wl_registry::INTERFACE.requests[h.opcode as usize];
    let args = decode_args(desc, &out[HEADER_SIZE..], &mut no_fds).unwrap();
    assert_eq!(wl_registry::Request::from_args(h.opcode, args).unwrap(), bind);
}

#[test]
fn round_trips_every_argument_kind() {
    // xdg_toplevel.configure: int, int, array.
    let e = xdg_toplevel::Event::Configure { width: -1, height: 600, states: vec![4, 0, 0, 0, 1, 0, 0, 0] };
    let (mut out, mut fds) = (Vec::new(), Vec::new());
    encode(&mut out, &mut fds, ObjectId(12), e.opcode(), &e.clone().into_args()).unwrap();
    let args = decode_args(&xdg_toplevel::INTERFACE.events[0], &out[8..], &mut no_fds).unwrap();
    assert_eq!(xdg_toplevel::Event::from_args(0, args).unwrap(), e);
    // wl_shm.create_pool: new_id, fd (out of band), int.
    let r = wl_shm::Request::CreatePool { id: ObjectId(10), fd: Fd(7), size: 4096 };
    let (mut out, mut fds) = (Vec::new(), Vec::new());
    encode(&mut out, &mut fds, ObjectId(4), r.opcode(), &r.clone().into_args()).unwrap();
    assert_eq!(fds, [Fd(7)]);
    assert_eq!(out.len(), 16);
    let mut queue = vec![Fd(7)].into_iter();
    let args = decode_args(&wl_shm::INTERFACE.requests[0], &out[8..], &mut || queue.next()).unwrap();
    assert_eq!(wl_shm::Request::from_args(0, args).unwrap(), r);
    // A null object where allowed.
    let attach = wl_surface::Request::Attach { buffer: None, x: 0, y: 0 };
    let args = attach.clone().into_args();
    assert_eq!(args[0], Arg::Object(None));
    assert_eq!(wl_surface::Request::from_args(1, args).unwrap(), attach);
}

#[test]
fn rejects_malformed_messages() {
    let sync = &wl_display::INTERFACE.requests[0];
    assert_eq!(decode_args(sync, &[], &mut no_fds), Err(WireError::Truncated));
    assert_eq!(decode_args(sync, &words(&[0]), &mut no_fds), Err(WireError::Null("callback")));
    assert_eq!(decode_args(sync, &words(&[3, 4]), &mut no_fds), Err(WireError::TrailingBytes));
    let create_pool = &wl_shm::INTERFACE.requests[0];
    assert_eq!(decode_args(create_pool, &words(&[3, 4096]), &mut no_fds), Err(WireError::MissingFd));
    // A string without its NUL, one longer than the message, a null one.
    let title = &xdg_toplevel::INTERFACE.requests[2];
    assert_eq!(title.name, "set_title");
    let mut bad = words(&[4]);
    bad.extend_from_slice(b"abcd");
    assert_eq!(decode_args(title, &bad, &mut no_fds), Err(WireError::BadString));
    assert_eq!(decode_args(title, &words(&[100, 0]), &mut no_fds), Err(WireError::Truncated));
    assert_eq!(decode_args(title, &words(&[0]), &mut no_fds), Err(WireError::Null("title")));
    // An empty string is one NUL, padded.
    let empty = decode_args(title, &words(&[1, 0]), &mut no_fds).unwrap();
    assert_eq!(empty, [Arg::Str(Some(String::new()))]);
    // A wrong opcode or argument list.
    assert_eq!(wl_compositor::Request::from_args(9, Vec::new()), Err(WireError::BadOpcode(9)));
    assert_eq!(wl_compositor::Request::from_args(0, vec![Arg::Int(1)]), Err(WireError::ArgMismatch));
    assert_eq!(WireError::BadOpcode(9).to_string(), "no opcode 9");
}

#[test]
fn refuses_messages_too_long_for_the_header() {
    let long = "x".repeat(MAX_MESSAGE_SIZE);
    let (mut out, mut fds) = (vec![1, 2, 3], Vec::new());
    let r = encode(&mut out, &mut fds, ObjectId(5), 2, &[Arg::Str(Some(long))]);
    assert_eq!(r, Err(WireError::TooLong));
    assert_eq!(out, [1, 2, 3]);
}

#[test]
fn every_generated_interface_is_findable() {
    for i in crate::protocol::INTERFACES {
        assert_eq!(crate::protocol::interface(i.name).map(|f| f.name), Some(i.name));
        for m in i.requests.iter().chain(i.events) {
            assert!(m.since >= 1 && m.since <= i.version, "{}.{}", i.name, m.name);
        }
    }
    assert_eq!(crate::protocol::interface("wl_surface").unwrap().version, 7);
    assert!(crate::protocol::interface("nope").is_none());
}
