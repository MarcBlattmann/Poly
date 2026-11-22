//! Shared clipboard bridge between Wayland/Linux/native Poly apps and Windows
//! (`win32k`) apps over `/dev/win32k`.
//!
//! - **Windows → Wayland**: when `win32k` reports a materialized clipboard
//!   snapshot (`ClipPayload` with UTF-8 text and/or packed `CF_DIB`), the bridge
//!   installs an external (compositor-managed) selection on `poly_wayland::Server`
//!   offering `text/plain;charset=utf-8`, `text/plain`, `UTF8_STRING` and/or
//!   `image/png`, `image/bmp`. When a Wayland client pastes from that offer, the
//!   bridge lazily encodes BMP or PNG (or serves UTF-8 directly) into the
//!   client's pipe using non-blocking writes so the compositor never stalls.
//! - **Wayland → Windows**: when a Wayland client sets a selection, the bridge
//!   selects the best text and/or image MIME type(s), creates non-blocking pipes,
//!   asks the client for the bytes via `Server::request_client_selection`, reads
//!   them asynchronously in the poll loop, converts `image/png` or `image/bmp`
//!   into packed `CF_DIB`, and sends `FromCompositor::Clip*` records to `win32k`.
//! - **Loop prevention**: installing/clearing an external selection on `Server`
//!   never emits a `SelectionEvent`, and importing bridged data into `win32k`
//!   never marks `Clipboard::bridge_dirty`. Any in-flight Wayland pipe read is
//!   cancelled if a newer selection arrives from either side.

use crate::os::{self, PollFd, RawFd};
use crate::prelude::*;
use crate::win32;
use poly_wayland::server::{ClientId, ExternalPasteRequest, SelectionEvent, Server};
use poly_wayland::wire::{Fd, ObjectId};
use poly_win32k::dib;
use poly_win32k::link::{ClipPayload, MAX_CLIPBOARD_BYTES};

/// Standard Wayland/X11 text MIME types offered for Windows clipboard text.
pub const MIME_UTF8: &str = "text/plain;charset=utf-8";
pub const MIME_TEXT_PLAIN: &str = "text/plain";
pub const MIME_UTF8_STRING: &str = "UTF8_STRING";
pub const MIME_PNG: &str = "image/png";
pub const MIME_BMP: &str = "image/bmp";

/// Maximum number of concurrent non-blocking paste writes to Wayland clients.
const MAX_PENDING_WRITES: usize = 16;

/// Image MIME format selected when reading from a Wayland client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMime {
    Png,
    Bmp,
}

/// MIME types offered on Wayland for a Windows clipboard snapshot.
pub fn win_to_wayland_mimes(has_text: bool, has_dib: bool) -> Vec<String> {
    let mut mimes = Vec::new();
    if has_text {
        mimes.push(String::from(MIME_UTF8));
        mimes.push(String::from(MIME_TEXT_PLAIN));
        mimes.push(String::from(MIME_UTF8_STRING));
    }
    if has_dib {
        mimes.push(String::from(MIME_PNG));
        mimes.push(String::from(MIME_BMP));
    }
    mimes
}

/// Pick the preferred text MIME type and/or image MIME type from what a
/// Wayland client offered.
pub fn pick_wayland_mimes(offered: &[String]) -> (Option<String>, Option<(String, ImageMime)>) {
    let has = |want: &str| offered.iter().any(|m| m == want);
    let text = [MIME_UTF8, MIME_UTF8_STRING, MIME_TEXT_PLAIN].into_iter().find(|m| has(m)).map(String::from);
    let image = if has(MIME_PNG) {
        Some((String::from(MIME_PNG), ImageMime::Png))
    } else if has(MIME_BMP) {
        Some((String::from(MIME_BMP), ImageMime::Bmp))
    } else {
        None
    };
    (text, image)
}

/// Materialize the bytes to send a Wayland client requesting `mime` from
/// bridged Windows clipboard data (`text` UTF-8 and/or `dib` packed `CF_DIB`).
pub fn render_external_paste(mime: &str, text: Option<&[u8]>, dib_bytes: Option<&[u8]>) -> Option<Vec<u8>> {
    match mime {
        MIME_UTF8 | MIME_TEXT_PLAIN | MIME_UTF8_STRING => text.map(|b| b.to_vec()),
        MIME_BMP => dib_bytes.and_then(dib::to_bmp),
        MIME_PNG => dib_bytes.and_then(dib::to_png),
        _ => None,
    }
}

/// Convert raw image bytes read from a Wayland client into a packed `CF_DIB`.
pub fn convert_wayland_image(kind: ImageMime, bytes: &[u8]) -> Option<Vec<u8>> {
    match kind {
        ImageMime::Png => dib::from_png(bytes),
        ImageMime::Bmp => dib::from_bmp(bytes),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadKind {
    Text,
    Image(ImageMime),
}

#[derive(Debug)]
struct PendingRead {
    fd: RawFd,
    kind: ReadKind,
    buf: Vec<u8>,
    done: bool,
    failed: bool,
}

#[derive(Debug)]
struct PendingWrite {
    fd: RawFd,
    data: Vec<u8>,
    offset: usize,
}

/// Bidirectional clipboard bridge state owned by the compositor loop.
#[derive(Debug, Default)]
pub struct ClipboardBridge {
    /// Active external selection ID installed on `poly_wayland::Server` (for
    /// Windows → Wayland).
    external_id: Option<u64>,
    external_text: Option<Vec<u8>>,
    external_dib: Option<Vec<u8>>,
    /// Monotonic sequence counter for Wayland → Windows clipboard updates.
    out_seq: u32,
    /// Non-blocking pipe reads currently pulling data from the active Wayland
    /// selection owner (empty when idle).
    reads: Vec<PendingRead>,
    /// Non-blocking pipe writes delivering Windows clipboard data to Wayland
    /// clients that requested a paste.
    writes: Vec<PendingWrite>,
}

impl ClipboardBridge {
    pub fn new() -> ClipboardBridge {
        ClipboardBridge::default()
    }

    fn next_seq(&mut self) -> u32 {
        self.out_seq = self.out_seq.wrapping_add(1).max(1);
        self.out_seq
    }

    /// Close and discard any in-flight reads from an earlier Wayland selection.
    fn cancel_reads(&mut self) {
        for r in self.reads.drain(..) {
            if !r.done {
                os::close(r.fd);
            }
        }
    }

    /// Apply one Windows clipboard snapshot to the Wayland server.
    pub fn apply_win32_payload(&mut self, server: &mut Server, payload: ClipPayload) {
        // Windows took clipboard ownership: cancel any stale Wayland read.
        self.cancel_reads();
        let mimes = win_to_wayland_mimes(payload.text.is_some(), payload.dib.is_some());
        if mimes.is_empty() {
            self.external_id = None;
            self.external_text = None;
            self.external_dib = None;
            server.clear_external_selection();
        } else {
            self.external_text = payload.text;
            self.external_dib = payload.dib;
            self.external_id = Some(server.set_external_selection(mimes));
        }
    }

    /// Drain completed Windows clipboard snapshots from `link` and expose the
    /// latest one as the Wayland selection.
    pub fn sync_from_win32(&mut self, server: &mut Server, link: &mut win32::Link) {
        let mut updates = link.take_clipboard();
        if let Some(latest) = updates.pop() {
            self.apply_win32_payload(server, latest);
        }
    }

    /// Drain Wayland selection changes and external paste requests from
    /// `server`. Must be called before flushing client sockets so queued
    /// `wl_data_source.send` events reach the client on the same loop tick.
    pub fn sync_from_wayland(&mut self, server: &mut Server, link: Option<&mut win32::Link>) {
        // 1. Handle Wayland client paste requests targeting our Windows selection.
        for req in server.take_external_paste_requests() {
            self.handle_paste_request(req);
        }

        // 2. Handle Wayland client selection changes (only the latest matters).
        let mut events = server.take_selection_events();
        let Some(latest) = events.pop() else { return };
        // A Wayland client took/cleared selection: clear our external mirror.
        self.external_id = None;
        self.external_text = None;
        self.external_dib = None;
        self.cancel_reads();

        let Some(link) = link else { return };
        match latest {
            SelectionEvent::Cleared => {
                let seq = self.next_seq();
                link.send_clipboard(seq, None, None);
            }
            SelectionEvent::ClientSelection { client, source, mime_types } => {
                self.start_wayland_read(server, link, client, source, &mime_types);
            }
        }
    }

    fn handle_paste_request(&mut self, req: ExternalPasteRequest) {
        if Some(req.id) != self.external_id || self.writes.len() >= MAX_PENDING_WRITES {
            os::close(req.fd.0);
            return;
        }
        let Some(data) =
            render_external_paste(&req.mime_type, self.external_text.as_deref(), self.external_dib.as_deref())
        else {
            os::close(req.fd.0);
            return;
        };
        let fd = req.fd.0;
        let _ = os::set_nonblocking(fd, true);
        let mut offset = 0;
        while offset < data.len() {
            match os::write_nonblock(fd, &data[offset..]) {
                Ok(Some(0)) | Err(_) => {
                    os::close(fd);
                    return;
                }
                Ok(Some(n)) => offset += n,
                Ok(None) => {
                    self.writes.push(PendingWrite { fd, data, offset });
                    return;
                }
            }
        }
        os::close(fd);
    }

    fn start_wayland_read(
        &mut self,
        server: &mut Server,
        link: &mut win32::Link,
        client: ClientId,
        source: ObjectId,
        mime_types: &[String],
    ) {
        let (text_mime, image_mime) = pick_wayland_mimes(mime_types);
        if text_mime.is_none() && image_mime.is_none() {
            let seq = self.next_seq();
            link.send_clipboard(seq, None, None);
            return;
        }
        let mut started = Vec::new();
        if let Some(mime) = text_mime
            && let Some(r) = open_read_pipe(server, client, source, mime, ReadKind::Text)
        {
            started.push(r);
        }
        if let Some((mime, img_kind)) = image_mime
            && let Some(r) = open_read_pipe(server, client, source, mime, ReadKind::Image(img_kind))
        {
            started.push(r);
        }
        if started.is_empty() {
            let seq = self.next_seq();
            link.send_clipboard(seq, None, None);
        } else {
            self.reads = started;
        }
    }

    /// Pipe descriptors to include in the compositor's `poll` call.
    pub fn poll_fds(&self) -> Vec<PollFd> {
        let mut out = Vec::with_capacity(self.reads.len() + self.writes.len());
        for r in &self.reads {
            out.push(if r.done { PollFd::readable(-1) } else { PollFd::readable(r.fd) });
        }
        for w in &self.writes {
            out.push(PollFd::writable(w.fd));
        }
        out
    }

    /// Advance non-blocking pipe reads and writes after `poll`.
    pub fn polled(&mut self, fds: &[PollFd], link: Option<&mut win32::Link>) {
        let read_count = self.reads.len();
        let mut chunk = [0u8; 8192];
        for (i, r) in self.reads.iter_mut().enumerate() {
            if r.done {
                continue;
            }
            let Some(pfd) = fds.get(i) else { continue };
            if pfd.revents == 0 {
                continue;
            }
            loop {
                match os::read_nonblock(r.fd, &mut chunk) {
                    Ok(Some(0)) => {
                        os::close(r.fd);
                        r.done = true;
                        break;
                    }
                    Ok(Some(n)) => {
                        if r.buf.len().saturating_add(n) > MAX_CLIPBOARD_BYTES {
                            os::close(r.fd);
                            r.buf.clear();
                            r.failed = true;
                            r.done = true;
                            break;
                        }
                        r.buf.extend_from_slice(&chunk[..n]);
                    }
                    Ok(None) => break,
                    Err(_) => {
                        os::close(r.fd);
                        r.buf.clear();
                        r.failed = true;
                        r.done = true;
                        break;
                    }
                }
            }
        }
        if !self.reads.is_empty() && self.reads.iter().all(|r| r.done) {
            let finished = core::mem::take(&mut self.reads);
            let mut text = None;
            let mut dib_bytes = None;
            for r in finished {
                if r.failed {
                    continue;
                }
                match r.kind {
                    ReadKind::Text => text = Some(r.buf),
                    ReadKind::Image(img_kind) => dib_bytes = convert_wayland_image(img_kind, &r.buf),
                }
            }
            if let Some(link) = link {
                let seq = self.next_seq();
                link.send_clipboard(seq, text.as_deref(), dib_bytes.as_deref());
            }
        }

        // Advance pending paste writes to Wayland clients.
        let mut idx = 0;
        let mut poll_i = read_count;
        while idx < self.writes.len() {
            let ready = fds.get(poll_i).is_some_and(|p| p.revents != 0);
            poll_i += 1;
            if !ready {
                idx += 1;
                continue;
            }
            let mut finished = false;
            {
                let w = &mut self.writes[idx];
                while w.offset < w.data.len() {
                    match os::write_nonblock(w.fd, &w.data[w.offset..]) {
                        Ok(Some(0)) | Err(_) => {
                            finished = true;
                            break;
                        }
                        Ok(Some(n)) => w.offset += n,
                        Ok(None) => break,
                    }
                }
                if w.offset >= w.data.len() {
                    finished = true;
                }
            }
            if finished {
                let w = self.writes.remove(idx);
                os::close(w.fd);
            } else {
                idx += 1;
            }
        }
    }
}

impl Drop for ClipboardBridge {
    fn drop(&mut self) {
        self.cancel_reads();
        for w in self.writes.drain(..) {
            os::close(w.fd);
        }
    }
}

fn open_read_pipe(
    server: &mut Server,
    client: ClientId,
    source: ObjectId,
    mime: String,
    kind: ReadKind,
) -> Option<PendingRead> {
    let (r, w) = os::pipe().ok()?;
    let _ = os::set_nonblocking(r, true);
    if !server.request_client_selection(client, source, mime, Fd(w)) {
        os::close(r);
        return None;
    }
    Some(PendingRead { fd: r, kind, buf: Vec::new(), done: false, failed: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_wayland::server::{Config, OutputConfig};
    use poly_win32k::clip::{self, CF_DIB, CF_UNICODETEXT, ClipOp, ClipRes, Clipboard};

    #[test]
    fn mime_negotiation_prefers_utf8_and_png() {
        assert_eq!(win_to_wayland_mimes(true, false), vec!["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]);
        assert_eq!(win_to_wayland_mimes(false, true), vec!["image/png", "image/bmp"]);
        assert_eq!(
            win_to_wayland_mimes(true, true),
            vec!["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "image/png", "image/bmp"]
        );
        assert!(win_to_wayland_mimes(false, false).is_empty());

        let mimes: Vec<String> = ["text/plain", "UTF8_STRING", "text/plain;charset=utf-8", "image/bmp", "image/png"]
            .into_iter()
            .map(String::from)
            .collect();
        let (t, i) = pick_wayland_mimes(&mimes);
        assert_eq!(t.as_deref(), Some(MIME_UTF8));
        assert_eq!(i, Some((String::from(MIME_PNG), ImageMime::Png)));

        let fallback: Vec<String> = ["text/plain", "image/bmp"].into_iter().map(String::from).collect();
        let (t, i) = pick_wayland_mimes(&fallback);
        assert_eq!(t.as_deref(), Some(MIME_TEXT_PLAIN));
        assert_eq!(i, Some((String::from(MIME_BMP), ImageMime::Bmp)));

        let unknown: Vec<String> = ["application/x-custom"].into_iter().map(String::from).collect();
        assert_eq!(pick_wayland_mimes(&unknown), (None, None));
    }

    #[test]
    fn paste_rendering_and_image_conversion_round_trip() {
        let px = [0x10u8, 0x20, 0x30, 0xFF, 0x40, 0x50, 0x60, 0xFF];
        let dib = dib::make32(2, 1, &px);
        let text = "Hello 🌍\nWorld".as_bytes();

        // Text MIME types all serve the UTF-8 payload.
        for m in [MIME_UTF8, MIME_TEXT_PLAIN, MIME_UTF8_STRING] {
            assert_eq!(render_external_paste(m, Some(text), Some(&dib)).as_deref(), Some(text));
        }
        // BMP and PNG paste rendering round-trip back to valid DIBs with matching pixels.
        let bmp = render_external_paste(MIME_BMP, Some(text), Some(&dib)).unwrap();
        let back_from_bmp = convert_wayland_image(ImageMime::Bmp, &bmp).unwrap();
        assert_eq!(back_from_bmp, dib);

        let png = render_external_paste(MIME_PNG, Some(text), Some(&dib)).unwrap();
        let back_from_png = convert_wayland_image(ImageMime::Png, &png).unwrap();
        let (w, h, back_px) = dib::to_bgra(&back_from_png).unwrap();
        assert_eq!((w, h, back_px.as_slice()), (2, 1, &px[..]));

        // Unsupported MIME yields None.
        assert!(render_external_paste("application/octet-stream", Some(text), Some(&dib)).is_none());
    }

    #[test]
    fn win32_to_wayland_and_clear_produce_no_feedback_loop() {
        let output = OutputConfig { width: 800, height: 600, ..OutputConfig::default() };
        let mut server = Server::new(Config { output, ..Config::default() }, Box::new(os::UnixHost::default()));
        let mut bridge = ClipboardBridge::new();

        // Windows clipboard -> bridge -> Wayland server.
        let mut win_clip = Clipboard::new();
        let mut notify = false;
        assert_eq!(win_clip.run(1, 1, ClipOp::Open { hwnd: 0x10 }, &mut notify), ClipRes::Ok {});
        win_clip.run(1, 1, ClipOp::Empty {}, &mut notify);
        let dib = dib::make32(1, 1, &[1, 2, 3, 255]);
        win_clip.run(
            1,
            1,
            ClipOp::Set { format: CF_UNICODETEXT, handle: 1, data: Some(clip::from_utf8(b"bridge\ntest")) },
            &mut notify,
        );
        win_clip.run(1, 1, ClipOp::Set { format: CF_DIB, handle: 2, data: Some(dib.clone()) }, &mut notify);
        win_clip.run(1, 1, ClipOp::Close {}, &mut notify);
        assert!(win_clip.take_bridge_dirty());
        let snap = win_clip.export_bridge();

        bridge.apply_win32_payload(
            &mut server,
            ClipPayload { seq: snap.seq, text: snap.text.clone(), dib: snap.dib.clone() },
        );
        assert!(bridge.external_id.is_some());
        assert!(server.take_selection_events().is_empty(), "installing Windows selection must not echo to Wayland");

        // Clearing Windows clipboard clears external selection without echoing.
        bridge.apply_win32_payload(&mut server, ClipPayload { seq: snap.seq + 1, text: None, dib: None });
        assert!(bridge.external_id.is_none());
        assert!(server.take_selection_events().is_empty(), "clearing Windows selection must not echo to Wayland");
    }
}
