//! A minimal VNC (RFB) client that asks the server to resize its desktop:
//! how a boot test makes QEMU's display tell the guest "the window is now
//! this big" without a window to drag (ADR 0145).
//!
//! QEMU's VNC server hands a client's `SetDesktopSize` request on to the
//! display device as the size the display should have (the same thing its
//! windows do when they are resized), so a virtio GPU raises its display
//! event and reports the new size. The protocol is RFC 6143 (RFB 3.8) plus
//! the ExtendedDesktopSize extension (the protocol's own registry of
//! encodings and messages): the handshake with no authentication, a
//! `SetEncodings` naming the extension (without it the server may not
//! offer resizing), then the request. Nothing is drawn; the framebuffer
//! updates the server sends after an update request are never asked for.

// Used by the Unix-only boot test (a VNC socket); the encoding and the
// handshake are tested everywhere.
#![cfg_attr(not(unix), allow(dead_code))]

use std::io::{Read, Write};

/// `SetEncodings` (RFC 6143 §7.5.2).
const SET_ENCODINGS: u8 = 2;
/// `SetDesktopSize` (the ExtendedDesktopSize extension's client message).
const SET_DESKTOP_SIZE: u8 = 251;
/// The pseudo-encodings that say a client takes desktop size changes:
/// `DesktopSize` (-223) and `ExtendedDesktopSize` (-308).
const ENCODINGS: [i32; 3] = [0, -223, -308];

/// `SetEncodings` for [`ENCODINGS`].
pub fn set_encodings() -> Vec<u8> {
    let mut m = vec![SET_ENCODINGS, 0];
    m.extend((ENCODINGS.len() as u16).to_be_bytes());
    for e in ENCODINGS {
        m.extend(e.to_be_bytes());
    }
    m
}

/// `SetDesktopSize` for a desktop of `width` x `height` with one screen
/// (id 0, at the origin, no flags).
pub fn set_desktop_size(width: u16, height: u16) -> Vec<u8> {
    let mut m = vec![SET_DESKTOP_SIZE, 0];
    m.extend(width.to_be_bytes());
    m.extend(height.to_be_bytes());
    // Number of screens, padding, then the screen: id, x, y, width, height,
    // flags.
    m.extend([1, 0]);
    m.extend(0u32.to_be_bytes());
    m.extend(0u16.to_be_bytes());
    m.extend(0u16.to_be_bytes());
    m.extend(width.to_be_bytes());
    m.extend(height.to_be_bytes());
    m.extend(0u32.to_be_bytes());
    m
}

fn read_exact<S: Read>(s: &mut S, n: usize) -> Result<Vec<u8>, String> {
    let mut b = vec![0; n];
    s.read_exact(&mut b).map_err(|e| format!("VNC: {e}"))?;
    Ok(b)
}

/// The handshake of RFB 3.8 with security type None; returns the desktop's
/// size now.
pub fn handshake<S: Read + Write>(s: &mut S) -> Result<(u16, u16), String> {
    let hello = read_exact(s, 12)?;
    if !hello.starts_with(b"RFB 003.") {
        return Err(format!("VNC: not an RFB server ({:?})", String::from_utf8_lossy(&hello)));
    }
    s.write_all(b"RFB 003.008\n").map_err(|e| format!("VNC: {e}"))?;
    let n = read_exact(s, 1)?[0] as usize;
    let types = read_exact(s, n)?;
    if !types.contains(&1) {
        return Err(format!("VNC: the server wants authentication (security types {types:?})"));
    }
    s.write_all(&[1]).map_err(|e| format!("VNC: {e}"))?;
    let result = read_exact(s, 4)?;
    if result != [0; 4] {
        return Err("VNC: the server refused the connection".into());
    }
    // ClientInit: shared. ServerInit: size, pixel format (16 bytes), name.
    s.write_all(&[1]).map_err(|e| format!("VNC: {e}"))?;
    let init = read_exact(s, 4)?;
    let size = (u16::from_be_bytes([init[0], init[1]]), u16::from_be_bytes([init[2], init[3]]));
    read_exact(s, 16)?;
    let name_len = u32::from_be_bytes(read_exact(s, 4)?.try_into().unwrap_or([0; 4]));
    read_exact(s, name_len as usize)?;
    Ok(size)
}

/// `FramebufferUpdateRequest` (RFC 6143 §7.5.3) for the whole `width` x
/// `height` desktop, not incremental: the server answers a client that has
/// asked for something, and a client that goes away at once may have its
/// last messages dropped.
pub fn update_request(width: u16, height: u16) -> Vec<u8> {
    let mut m = vec![3, 0, 0, 0, 0, 0];
    m.extend(width.to_be_bytes());
    m.extend(height.to_be_bytes());
    m
}

/// The result code of the server's answer to a client's `SetDesktopSize` in
/// `data` (what the server sent so far): an `ExtendedDesktopSize` rectangle
/// whose x is the reason 1 ("the client asked", RFB's registry of encodings)
/// and whose y is the result. `None` if it is not there (yet).
pub fn resize_reply(data: &[u8]) -> Option<u16> {
    const ENCODING: [u8; 4] = (-308i32).to_be_bytes();
    // A rectangle: x, y, width, height (two bytes each), the encoding.
    (8..data.len().saturating_sub(3))
        .filter(|&i| data[i..i + 4] == ENCODING)
        .find(|&i| data[i - 8..i - 6] == [0, 1])
        .map(|i| u16::from_be_bytes([data[i - 6], data[i - 5]]))
}

/// On `s`: the handshake, say we take size changes, and ask for a `width` x
/// `height` desktop. Returns the size the server had. Waits for the server's
/// answer to the request (a server that is slow to take it, a busy emulator,
/// may lose a request from a client that goes away at once). QEMU's result
/// code in it is not a reliable yes or no (a request it carried out came back
/// with a nonzero one), so only that it came counts; a server that never
/// answers (a canned one in a test, the stream ending) is not an error.
pub fn request_size<S: Read + Write>(mut s: S, width: u16, height: u16) -> Result<(u16, u16), String> {
    let before = handshake(&mut s)?;
    s.write_all(&set_encodings()).map_err(|e| format!("VNC: {e}"))?;
    // A small region: the server answers a client that has asked for
    // something, and the answer is not to be a whole screen of pixels.
    s.write_all(&update_request(1, 1)).map_err(|e| format!("VNC: {e}"))?;
    s.write_all(&set_desktop_size(width, height)).map_err(|e| format!("VNC: {e}"))?;
    s.flush().map_err(|e| format!("VNC: {e}"))?;
    let mut seen = Vec::new();
    let mut chunk = [0u8; 4096];
    while seen.len() < 1 << 20 {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => seen.extend_from_slice(&chunk[..n]),
        }
        if resize_reply(&seen).is_some() {
            break;
        }
    }
    Ok(before)
}

/// [`request_size`] to the VNC server on the Unix socket `path`.
#[cfg(unix)]
pub fn request_size_at(path: &std::path::Path, width: u16, height: u16) -> Result<(u16, u16), String> {
    let s = std::os::unix::net::UnixStream::connect(path).map_err(|e| format!("VNC {}: {e}", path.display()))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(10))).map_err(|e| format!("VNC: {e}"))?;
    request_size(s, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn set_encodings_names_the_extension() {
        assert_eq!(set_encodings(), [2, 0, 0, 3, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0x21, 0xff, 0xff, 0xfe, 0xcc]);
    }

    #[test]
    fn set_desktop_size_has_one_screen_of_the_size() {
        let m = set_desktop_size(1313, 877);
        assert_eq!(m.len(), 24);
        assert_eq!(&m[..8], [251, 0, 0x05, 0x21, 0x03, 0x6d, 1, 0]);
        // id 0 at 0,0, the size, no flags.
        assert_eq!(&m[8..], [0, 0, 0, 0, 0, 0, 0, 0, 0x05, 0x21, 0x03, 0x6d, 0, 0, 0, 0]);
    }

    /// A stream that answers with a canned server and records what is sent.
    struct Pipe {
        input: Cursor<Vec<u8>>,
        sent: Vec<u8>,
    }

    impl Read for Pipe {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Pipe {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.sent.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn server(security: &[u8], result: u32) -> Vec<u8> {
        let mut s = b"RFB 003.008\n".to_vec();
        s.push(security.len() as u8);
        s.extend(security);
        s.extend(result.to_be_bytes());
        s.extend(1280u16.to_be_bytes());
        s.extend(800u16.to_be_bytes());
        s.extend([0u8; 16]);
        s.extend(4u32.to_be_bytes());
        s.extend(b"QEMU");
        s
    }

    #[test]
    fn a_resize_request_is_the_handshake_encodings_and_the_size() {
        let mut p = Pipe { input: Cursor::new(server(&[1], 0)), sent: Vec::new() };
        let before = request_size(&mut p, 1024, 640).unwrap();
        assert_eq!(before, (1280, 800));
        let mut want = b"RFB 003.008\n".to_vec();
        want.extend([1, 1]);
        want.extend(set_encodings());
        want.extend(update_request(1, 1));
        want.extend(set_desktop_size(1024, 640));
        assert_eq!(p.sent, want);
    }

    #[test]
    fn the_servers_answer_to_a_resize_is_found_in_what_it_sent() {
        let rect = |reason: u16, result: u16| {
            let mut r = reason.to_be_bytes().to_vec();
            r.extend(result.to_be_bytes());
            r.extend(1024u16.to_be_bytes());
            r.extend(640u16.to_be_bytes());
            r.extend((-308i32).to_be_bytes());
            r
        };
        assert_eq!(resize_reply(&[]), None);
        assert_eq!(resize_reply(&rect(0, 0)), None, "the server's own report is not the answer");
        assert_eq!(resize_reply(&rect(1, 0)), Some(0));
        assert_eq!(resize_reply(&rect(1, 3)), Some(3));
        let mut both = vec![0u8; 40];
        both.extend(rect(0, 0));
        both.extend(rect(1, 0));
        assert_eq!(resize_reply(&both), Some(0));
        // Any answer to the request is the end of waiting.
        let mut input = server(&[1], 0);
        input.extend(rect(1, 3));
        let mut p = Pipe { input: Cursor::new(input), sent: Vec::new() };
        assert_eq!(request_size(&mut p, 1024, 640), Ok((1280, 800)));
        let mut input = server(&[1], 0);
        input.extend(rect(1, 0));
        let mut p = Pipe { input: Cursor::new(input), sent: Vec::new() };
        assert_eq!(request_size(&mut p, 1024, 640), Ok((1280, 800)));
    }

    #[test]
    fn a_server_that_wants_a_password_or_refuses_is_an_error() {
        let mut p = Pipe { input: Cursor::new(server(&[2], 0)), sent: Vec::new() };
        assert!(handshake(&mut p).unwrap_err().contains("authentication"));
        let mut p = Pipe { input: Cursor::new(server(&[1], 1)), sent: Vec::new() };
        assert!(handshake(&mut p).unwrap_err().contains("refused"));
        let mut p = Pipe { input: Cursor::new(b"HTTP/1.1 400\n".to_vec()), sent: Vec::new() };
        assert!(handshake(&mut p).unwrap_err().contains("not an RFB server"));
    }
}
