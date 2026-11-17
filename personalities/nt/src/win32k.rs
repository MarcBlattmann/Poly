//! win32k: table 1 of the NT system calls (ADR 0128).
//!
//! Windows GUI apps reach win32k through Poly's own services
//! (`poly_win32k::services`), whose stubs are in our `win32u.dll` and which
//! our `user32.dll` calls. `PolyUserCall` carries one
//! `poly_win32k::proto::Request` and returns its `Reply`; the session's
//! window manager is `poly_win32k::wm::Wm`. Blocking happens in `user32`,
//! on a per-thread event it registers here (`RegisterEvent`), which this
//! module sets whenever the window manager says a thread has something new
//! (a posted or sent message, a reply). Before dispatch, a process under
//! the win32k lockdown (`DisallowWin32kSystemCalls`,
//! [`NtProcess::win32k_disabled`]) is refused.
//!
//! The desktop link (ADR 0131): after every call (and when a thread or
//! process ends) the surfaces of top-level windows are brought in line
//! with the window manager (`poly_win32k::surfaces`); `PolyUserCommitSurface`
//! copies a window's pixels from the app into its surface buffer, and
//! `PolyUserPresentBuffer` hands the compositor a section the app drew a
//! frame in (ADR 0224). The kernel's `/dev/win32k` device reaches
//! [`Win32k`]'s link methods.
//!
//! [`NtProcess::win32k_disabled`]: crate::system::NtProcess::win32k_disabled

use crate::kernel::NtKernel;
use crate::{STATUS_NOT_IMPLEMENTED, Table, abi, decode};
use alloc::collections::BTreeMap;
use alloc::vec;
use poly_abi::{Outcome, SyscallRegs};
use poly_ntobj::status::{STATUS_BUFFER_TOO_SMALL, STATUS_INVALID_PARAMETER, STATUS_SUCCESS};
use poly_ntobj::{Handle, NtStatus, ObjectId, ObjectType};
use poly_win32k::desktop::Desktop;
use poly_win32k::frame::Rect;
use poly_win32k::layers::{Layers, Presented};
use poly_win32k::link::{FromCompositor, Request as DesktopRequest};
use poly_win32k::proto::{Reply, Request};
use poly_win32k::services::Win32kService;
use poly_win32k::surfaces::{BAND_BYTES, MAX_SIDE, Surfaces};
use poly_win32k::wire;
use poly_win32k::wm::{Ctx, Effects, Key, Wm};

/// What a process under the win32k lockdown gets for every table-1 call.
/// Provisional: Learn says only that such a process "is not permitted to
/// perform GUI system calls"; what Windows returns is to be confirmed by
/// the spec agent against real Windows (ADR 0128).
pub const LOCKDOWN_STATUS: NtStatus = poly_ntobj::status::STATUS_ACCESS_DENIED;

/// The longest request accepted.
const MAX_REQUEST: u64 = 1 << 20;

/// win32k's machine-wide state, in [`crate::NtSystem`].
#[derive(Debug, Default)]
pub struct Win32k {
    /// The window manager of the one session.
    pub wm: Wm,
    /// The desktop link's win32k end: the surfaces of top-level windows
    /// as the compositor is told about them.
    pub desktop: Desktop,
    /// The surfaces' windows and buffers.
    pub surfaces: Surfaces,
    /// Pictures presented as sections (ADR 0224); each adopted section object holds a
    /// reference of ours.
    pub layers: Layers,
    /// Each thread's queue event (registered by `user32`) and the handle
    /// value it registered.
    events: BTreeMap<Key, (ObjectId, u64)>,
    /// The window whose move or size the compositor drags (`Request::MoveSize`): it gets
    /// `WM_EXITSIZEMOVE` when the left button comes up (ADR 0212).
    dragging: Option<u32>,
}

impl Win32k {
    /// Forget a thread: its window manager state and its event.
    pub fn thread_exited(&mut self, objects: &mut poly_ntobj::ObjectManager, pid: u32, tid: u32) {
        let mut fx = Effects::default();
        self.wm.thread_exited(pid, tid, &mut fx);
        if let Some((id, _)) = self.events.remove(&(pid, tid)) {
            let _ = objects.dereference(id);
        }
        self.wake(objects, &fx);
        self.sync();
    }

    /// Forget a process.
    pub fn process_exited(&mut self, objects: &mut poly_ntobj::ObjectManager, pid: u32) {
        let mut fx = Effects::default();
        self.wm.process_exited(pid, &mut fx);
        for object in self.layers.remove_process(&mut self.desktop, pid) {
            let _ = objects.dereference(ObjectId::from_raw(object));
        }
        let gone: alloc::vec::Vec<Key> = self.events.keys().filter(|k| k.0 == pid).copied().collect();
        for k in gone {
            if let Some((id, _)) = self.events.remove(&k) {
                let _ = objects.dereference(id);
            }
        }
        self.wake(objects, &fx);
        self.sync();
    }

    /// Bring the desktop link's surfaces in line with the windows.
    pub fn sync(&mut self) {
        self.surfaces.sync(&self.wm, &mut self.desktop);
    }

    /// The device: bytes the compositor wrote (its greeting brings the
    /// surfaces of the windows there are already).
    pub fn link_receive(&mut self, objects: &mut poly_ntobj::ObjectManager, bytes: &[u8]) {
        let events = self.desktop.receive(bytes);
        let mut fx = Effects::default();
        let frame = self.desktop.frame();
        for event in events {
            match event {
                // Input from the desktop: the pointer over a surface
                // (relative to its top left, the window's client area),
                // buttons and keys (ADR 0136).
                FromCompositor::Pointer { surface, x, y, .. } => {
                    if let Some(i) = self.surfaces.hwnd_of(surface).and_then(|h| self.wm.info(h)) {
                        self.wm.link_pointer(i.client.left + x, i.client.top - i.menu_bar + y, &mut fx);
                    }
                }
                FromCompositor::Button { button, pressed, .. } => {
                    self.wm.link_button(button, pressed, &mut fx);
                    // The compositor's drag ends with the button: the window leaves its sizing and
                    // moving loop (`WM_EXITSIZEMOVE`).
                    if !pressed
                        && button == 0x110
                        && let Some(hwnd) = self.dragging.take()
                    {
                        self.wm.post_system_message(hwnd, 0x0232, 0, &mut fx);
                    }
                }
                FromCompositor::Axis { vertical, horizontal, .. } => {
                    self.wm.link_wheel(vertical, &mut fx);
                    self.wm.link_hwheel(horizontal, &mut fx);
                }
                FromCompositor::Key { code, pressed, .. } => self.wm.link_key(code, pressed, &mut fx),
                // Windows apps live in output pixels: the system DPI follows (ADR 0219).
                FromCompositor::Units { units } => self.wm.set_system_dpi(96 * units.clamp(1, 8)),
                // The shell's request to close a window: its thread gets
                // `WM_CLOSE`, as from a click on a caption close button.
                FromCompositor::Request { surface, request: DesktopRequest::Close } => {
                    if let Some(hwnd) = self.surfaces.hwnd_of(surface) {
                        self.wm.post_system_message(hwnd, poly_win32k::user::WM_CLOSE, 0, &mut fx);
                    }
                }
                // The shell minimizes, maximizes or restores the window (a minimized window
                // picked in the window switcher): the window command, as from its caption.
                FromCompositor::Request { surface, request } => {
                    let sc = match request {
                        DesktopRequest::Minimize => poly_win32k::user::SC_MINIMIZE,
                        DesktopRequest::Maximize => poly_win32k::user::SC_MAXIMIZE,
                        _ => poly_win32k::user::SC_RESTORE,
                    };
                    if let Some(hwnd) = self.surfaces.hwnd_of(surface) {
                        self.wm.post_system_message_with(hwnd, poly_win32k::user::WM_SYSCOMMAND, sc, 0, &mut fx);
                    }
                }
                // The compositor moved or resized the window's surface (a new size: its
                // thread applies it with `SetWindowPos`).
                FromCompositor::Configure { surface, x, y, width, height } => {
                    if let Some(hwnd) = self.surfaces.hwnd_of(surface) {
                        let (width, height) = (width.min(i32::MAX as u32) as i32, height.min(i32::MAX as u32) as i32);
                        let client = Rect::new(x, y, x.saturating_add(width), y.saturating_add(height));
                        self.wm.configure_from_desktop(hwnd, client, frame, &mut fx);
                    }
                }
                _ => {}
            }
        }
        self.wake(objects, &fx);
        // Windows programs type with the desktop's keyboard layout (ADR 0209).
        if let Some(name) = self.desktop.layout() {
            self.wm.set_layout(poly_win32k::kbd::Layout::for_desktop(name));
        }
        // With a compositor the session is interactive, its screen the
        // output, the work area below Poly's 24-pixel ridge.
        if let Some(o) = self.desktop.output()
            && !self.wm.display().interactive
        {
            // The ridge in Windows pixels (ADR 0219: `units` of them a logical pixel).
            let ridge = 24 * (self.wm.system_dpi() / 96).max(1) as i32;
            self.wm.set_display(poly_win32k::metrics::Display::desktop(o.width as i32, o.height as i32, ridge));
        }
        self.sync();
    }

    /// The device: what to hand the compositor's next `read`.
    pub fn link_output(&mut self) -> alloc::vec::Vec<u8> {
        self.desktop.take_output()
    }

    /// The device: the compositor closed it.
    pub fn link_closed(&mut self) {
        self.desktop.disconnect();
        self.sync();
    }

    /// The kernel's backing id of a layer buffer's pages, for the compositor's mapping of the
    /// device (ADR 0224).
    pub fn link_layer_backing(&self, buffer: u32) -> Option<u64> {
        self.layers.backing(buffer)
    }

    /// The device's `READ_BUFFER`: rows of a buffer.
    pub fn link_rows(&self, buffer: u32, row: u32, rows: u32) -> Option<alloc::vec::Vec<u8>> {
        self.surfaces.rows(buffer, row, rows)
    }

    /// The window manager's thread queues for a wait report (`poly.nt.waits`), each with the
    /// state of the event the thread registered (`set` / `clear` / `none`).
    pub fn queue_debug_lines(&self, objects: &poly_ntobj::ObjectManager) -> alloc::vec::Vec<alloc::string::String> {
        let mut lines = self.wm.queue_debug_lines();
        for (k, (id, handle)) in &self.events {
            let state = objects.event_info(*id).map_or("gone", |e| if e.signaled { "set" } else { "clear" });
            lines.push(alloc::format!("queue event p{} t{} handle {handle:#x} {state}", k.0, k.1));
        }
        lines
    }

    fn wake(&self, objects: &mut poly_ntobj::ObjectManager, fx: &Effects) {
        for k in &fx.wake {
            if let Some(&(id, _)) = self.events.get(k) {
                let _ = objects.set_event(id);
            }
        }
    }
}

/// The name of win32k service `index` (the low 12 bits of its number), for
/// the kernel's log of unimplemented calls.
pub fn service_name(index: u64) -> Option<&'static str> {
    let number = (poly_win32k::services::TABLE_WIN32K << 12) | (index as u32 & 0xfff);
    Win32kService::from_number(number).map(Win32kService::name)
}

/// Run the table-1 call in `regs` against `k`, its `NTSTATUS` into `rax`.
pub fn dispatch<K: NtKernel>(k: &mut K, regs: &mut SyscallRegs) -> Outcome {
    let number = regs.rax as u32;
    let (table, index) = decode(number);
    debug_assert_eq!(table, Table::Win32k);
    if k.with_state(|_, p, _| p.win32k_disabled) {
        regs.rax = u64::from(LOCKDOWN_STATUS.0);
        return Outcome::Handled;
    }
    let status = match Win32kService::from_number(number) {
        Some(Win32kService::Call) => call(k, regs).unwrap_or_else(|e| e),
        Some(Win32kService::CommitSurface) => commit(k, regs).unwrap_or_else(|e| e),
        Some(Win32kService::CommitSurfaceArea) => commit_area(k, regs).unwrap_or_else(|e| e),
        Some(Win32kService::PresentBuffer) => present_buffer(k, regs).unwrap_or_else(|e| e),
        None => {
            regs.rax = u64::from(STATUS_NOT_IMPLEMENTED);
            return Outcome::Unimplemented { table: Table::Win32k.name(), number: u64::from(index) };
        }
    };
    regs.rax = u64::from(status.0);
    Outcome::Handled
}

/// `PolyUserCall(request, request_length, reply, reply_capacity,
/// reply_length*)`.
fn call<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus, NtStatus> {
    let [req_ptr, req_len, out_ptr, out_cap, out_len_ptr] = abi::args::<K, 5>(k, regs)?;
    if req_len == 0 || req_len > MAX_REQUEST {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let mut bytes = vec![0u8; req_len as usize];
    abi::read(k, req_ptr, &mut bytes)?;
    let req: Request = wire::decode(&bytes).ok_or(STATUS_INVALID_PARAMETER)?;
    let caller = k.caller();
    let ctx = Ctx { pid: caller.process.0, tid: caller.thread.0 as u32, now: k.monotonic_100ns() / 10_000 };
    // Window stations and desktops make handles and read the caller's descriptors: the NT side's.
    let reply = if let Request::Winsta { op } = &req {
        Reply::Winsta { res: crate::winsta::handle_call(k, op) }
    } else {
        call_wm(k, &caller, ctx, req)
    };
    let out = wire::encode(&reply);
    abi::write_u64(k, out_len_ptr, out.len() as u64)?;
    if out.len() as u64 > out_cap {
        return Ok(STATUS_BUFFER_TOO_SMALL);
    }
    abi::write(k, out_ptr, &out)?;
    Ok(STATUS_SUCCESS)
}

/// One request for the window manager.
fn call_wm<K: NtKernel>(k: &mut K, caller: &poly_ntobj::Caller, ctx: Ctx, req: Request) -> Reply {
    k.with_state(|sys, _, _| {
        match req {
            Request::RegisterEvent { handle } => return register_event(sys, caller, ctx, handle),
            Request::Surface { hwnd } => {
                let w = &sys.win32k;
                // A child draws into its top-level window's surface, through a bitmap of its
                // client size (`commit`).
                let size = w
                    .surfaces
                    .size(&w.desktop, hwnd)
                    .or_else(|| w.surfaces.child_area(&w.wm, &w.desktop, hwnd).map(|a| (a.width, a.height)))
                    .map_or(0, |(x, y)| u64::from(x) | (u64::from(y) << 32));
                return Reply::Value { value: size };
            }
            // A layered window passes the hit through where it shows nothing.
            Request::WindowFromPoint { x, y, below } => {
                let mut below = below;
                let mut fx = Effects::default();
                loop {
                    let r = sys.win32k.wm.handle(ctx, Request::WindowFromPoint { x, y, below }, &mut fx);
                    let Reply::Handle { hwnd } = r else { return r };
                    let w = &sys.win32k;
                    let mut top = hwnd;
                    while let Some(i) = w.wm.info(top) {
                        if i.parent == 0 || w.wm.info(i.parent).is_none_or(|p| p.desktop) {
                            break;
                        }
                        top = i.parent;
                    }
                    if top != 0 && w.surfaces.hit_passes(&w.wm, &w.desktop, top, x, y) {
                        below = top;
                        continue;
                    }
                    return r;
                }
            }
            Request::ScreenPixel { x, y } => {
                let w = &sys.win32k;
                return Reply::Value { value: u64::from(w.surfaces.screen_pixel(&w.wm, &w.desktop, x, y)) };
            }
            // What UpdateLayeredWindow gave a layered window (ADR 0172).
            Request::LayeredContent { hwnd, width, height, pixels } => {
                let w = &mut sys.win32k;
                return if w.surfaces.set_content(&w.wm, hwnd, width, height, pixels) {
                    Reply::Ok {}
                } else {
                    Reply::Error { code: 87 }
                };
            }
            // DefWindowProc's SC_MOVE / SC_SIZE: the compositor drags the surface (ADR 0128).
            Request::MoveSize { hwnd, edges } => {
                let w = &mut sys.win32k;
                // Without a compositor there is no one to drag the surface: the caller runs the
                // sizing and moving loop itself (`user::sizemove`, ADR 0212).
                if !w.desktop.is_connected() {
                    return Reply::Error { code: 50 };
                }
                let Some(id) = w.surfaces.surface(hwnd) else { return Reply::Error { code: 1400 } };
                if edges == 0 {
                    w.desktop.begin_move(id);
                } else {
                    w.desktop.begin_resize(id, poly_win32k::link::Edges(edges));
                }
                w.dragging = Some(hwnd);
                return Reply::Ok {};
            }
            // A system cursor over a top-level window's surface (ADR 0172).
            Request::CursorShape { hwnd, idc } => {
                let w = &mut sys.win32k;
                if let Some(id) = w.surfaces.surface(hwnd) {
                    w.desktop.set_cursor(id, poly_win32k::link::CursorShape::Standard(idc));
                }
                return Reply::Ok {};
            }
            _ => {}
        }
        let mut fx = Effects::default();
        let reply = sys.win32k.wm.handle(ctx, req, &mut fx);
        sys.win32k.wake(&mut sys.objects, &fx);
        sys.win32k.sync();
        reply
    })
}

/// `PolyUserCommitSurface(hwnd, pixels, width, height, stride)`: the app
/// drew window `hwnd`'s surface (32-bit pixels, rows `stride` bytes apart);
/// they go into its buffer for the compositor. `STATUS_INVALID_PARAMETER`
/// when the window has no surface of that size (it was resized or hidden
/// since; `user32` asks for the size again next time).
fn commit<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus, NtStatus> {
    let [hwnd, pixels, width, height, stride] = abi::args::<K, 5>(k, regs)?;
    let (hwnd, width, height) = (hwnd as u32, width as u32, height as u32);
    let size = k.with_state(|sys, _, _| sys.win32k.surfaces.size(&sys.win32k.desktop, hwnd));
    if size.is_none() {
        return commit_child(k, hwnd, pixels, width, height, stride);
    }
    if size != Some((width, height)) || width > MAX_SIDE || height > MAX_SIDE || stride < u64::from(width) * 4 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    // In pieces of whole rows: a maximized window's surface is larger than
    // one kernel allocation may be (`poly_win32k::surfaces::BAND_BYTES`).
    let row = width as usize * 4;
    let piece = (BAND_BYTES / row.max(1)).max(1) as u32;
    let mut bytes = vec![0u8; row * piece.min(height) as usize];
    let mut y = 0u32;
    while y < height {
        let n = piece.min(height - y) as usize;
        let part = &mut bytes[..n * row];
        for (i, chunk) in part.chunks_mut(row).enumerate() {
            abi::read(k, pixels + (u64::from(y) + i as u64) * stride, chunk)?;
        }
        let ok = k.with_state(|sys, _, _| {
            let w = &mut sys.win32k;
            w.surfaces.write_rows(&w.desktop, hwnd, width, height, y, part)
        });
        if !ok {
            return Err(STATUS_INVALID_PARAMETER);
        }
        y += n as u32;
    }
    let ok = k.with_state(|sys, _, _| {
        let w = &mut sys.win32k;
        w.surfaces.committed(&mut w.desktop, hwnd)
    });
    Ok(if ok { STATUS_SUCCESS } else { STATUS_INVALID_PARAMETER })
}

/// `PolyUserCommitSurfaceArea(hwnd, pixels, width, height, stride, area)`: as
/// `PolyUserCommitSurface`, but only the pixels of `area` (a `RECT` in surface pixels: what the
/// calling process drew) go into the buffer and are damaged (ADR 0219). Another process's drawing
/// elsewhere in the window stays. A window without a surface of its own (a child) commits its
/// whole picture into its top-level window's, as `PolyUserCommitSurface` does.
fn commit_area<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus, NtStatus> {
    let [hwnd, pixels, width, height, stride, area] = abi::args::<K, 6>(k, regs)?;
    let (hwnd, width, height) = (hwnd as u32, width as u32, height as u32);
    let size = k.with_state(|sys, _, _| sys.win32k.surfaces.size(&sys.win32k.desktop, hwnd));
    if size.is_none() {
        return commit_child(k, hwnd, pixels, width, height, stride);
    }
    if size != Some((width, height)) || width > MAX_SIDE || height > MAX_SIDE || stride < u64::from(width) * 4 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let mut r = [0u8; 16];
    abi::read(k, area, &mut r)?;
    let edge = |i: usize| i32::from_le_bytes([r[i * 4], r[i * 4 + 1], r[i * 4 + 2], r[i * 4 + 3]]);
    let clamp = |v: i32, most: u32| v.clamp(0, most as i32) as u32;
    let (left, top) = (clamp(edge(0), width), clamp(edge(1), height));
    let (right, bottom) = (clamp(edge(2), width).max(left), clamp(edge(3), height).max(top));
    if left == right || top == bottom {
        return Ok(STATUS_SUCCESS);
    }
    let span = (right - left) as usize * 4;
    let piece = (BAND_BYTES / span.max(1)).max(1) as u32;
    let mut bytes = vec![0u8; span * piece.min(bottom - top) as usize];
    let mut y = top;
    while y < bottom {
        let n = piece.min(bottom - y) as usize;
        let part = &mut bytes[..n * span];
        for (i, chunk) in part.chunks_mut(span).enumerate() {
            abi::read(k, pixels + (u64::from(y) + i as u64) * stride + u64::from(left) * 4, chunk)?;
        }
        let ok = k.with_state(|sys, _, _| {
            let w = &mut sys.win32k;
            w.surfaces.write_area(&w.desktop, hwnd, (width, height), (left, y, right - left), part)
        });
        if !ok {
            return Err(STATUS_INVALID_PARAMETER);
        }
        y += n as u32;
    }
    let ok = k.with_state(|sys, _, _| {
        let w = &mut sys.win32k;
        w.surfaces.committed_area(&mut w.desktop, hwnd, (left, top, right - left, bottom - top))
    });
    Ok(if ok { STATUS_SUCCESS } else { STATUS_INVALID_PARAMETER })
}

/// The most dirty rectangles one present names (more: the whole buffer).
const MAX_PRESENT_RECTS: u32 = 64;

/// `SECTION_MAP_READ` (winnt.h): the access the caller's section handle must allow.
const SECTION_MAP_READ: u32 = 0x0004;

/// `PolyUserPresentBuffer(hwnd, section, width, height, stride, rects, count)` (ADR 0224): the
/// frame in the caller's section is window `hwnd`'s picture from the caller now. The section
/// becomes a buffer of the window's layer (a reference of win32k's keeps it while the layer has
/// it) that the compositor maps, and the rectangles are damage: no pixel is copied. Section 0
/// removes the caller's layer of the window. `STATUS_INVALID_PARAMETER` for a picture larger than
/// the section or a surface may be; success without a record when the window shows nowhere.
fn present_buffer<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus, NtStatus> {
    let [hwnd, section, width, height, stride, rects, count] = abi::args::<K, 7>(k, regs)?;
    let (hwnd, width, height) = (hwnd as u32, width as u32, height as u32);
    let caller = k.caller();
    let pid = caller.process.0;
    if section == 0 {
        return Ok(k.with_state(|sys, _, b| {
            let w = &mut sys.win32k;
            for object in w.layers.remove(&mut w.desktop, hwnd, pid) {
                let _ = sys.objects.dereference(ObjectId::from_raw(object));
            }
            sys.reap(b);
            STATUS_SUCCESS
        }));
    }
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || stride < u64::from(width) * 4
        || stride > u64::from(MAX_SIDE) * 4
        || count > u64::from(MAX_PRESENT_RECTS)
    {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let mut list = alloc::vec::Vec::new();
    if count > 0 && rects != 0 {
        let mut raw = vec![0u8; count as usize * 16];
        abi::read(k, rects, &mut raw)?;
        let edge = |r: &[u8], i: usize| i32::from_le_bytes([r[i * 4], r[i * 4 + 1], r[i * 4 + 2], r[i * 4 + 3]]);
        list.extend(raw.chunks_exact(16).map(|r| (edge(r, 0), edge(r, 1), edge(r, 2), edge(r, 3))));
    }
    k.with_state(|sys, _, b| {
        let object =
            sys.objects.object_by_handle(&caller, Handle(section), SECTION_MAP_READ, Some(ObjectType::Section))?;
        let key = sys.objects.cookie(object).ok_or(STATUS_INVALID_PARAMETER)?;
        let s = sys.section(key).ok_or(STATUS_INVALID_PARAMETER)?;
        if s.image().is_some() || s.size < stride * u64::from(height) {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let presented = Presented { object: object.raw(), backing: s.backing, width, height, stride: stride as u32 };
        let w = &mut sys.win32k;
        let Some(outcome) = w.layers.present(&w.surfaces, &w.wm, &mut w.desktop, hwnd, pid, presented, &list) else {
            return Ok(STATUS_SUCCESS);
        };
        if outcome.adopted {
            let _ = sys.objects.reference(object);
        }
        for o in outcome.dropped {
            let _ = sys.objects.dereference(ObjectId::from_raw(o));
        }
        sys.reap(b);
        Ok(STATUS_SUCCESS)
    })
}

/// A commit of a child window: its picture into its top-level window's surface where the child
/// is (`Surfaces::child_area`), in pieces of whole rows as `commit`.
fn commit_child<K: NtKernel>(
    k: &mut K,
    hwnd: u32,
    pixels: u64,
    width: u32,
    height: u32,
    stride: u64,
) -> Result<NtStatus, NtStatus> {
    let area = k.with_state(|sys, _, _| {
        let w = &sys.win32k;
        w.surfaces.child_area(&w.wm, &w.desktop, hwnd)
    });
    let Some(area) = area else { return Err(STATUS_INVALID_PARAMETER) };
    if (area.width, area.height) != (width, height)
        || width > MAX_SIDE
        || height > MAX_SIDE
        || stride < u64::from(width) * 4
    {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let row = width as usize * 4;
    let piece = (BAND_BYTES / row.max(1)).max(1) as u32;
    let mut bytes = vec![0u8; row * piece.min(height) as usize];
    let mut y = 0u32;
    while y < height {
        let n = piece.min(height - y) as usize;
        let part = &mut bytes[..n * row];
        for (i, chunk) in part.chunks_mut(row).enumerate() {
            abi::read(k, pixels + (u64::from(y) + i as u64) * stride, chunk)?;
        }
        let ok = k.with_state(|sys, _, _| {
            let w = &mut sys.win32k;
            w.surfaces.write_child_rows(&w.desktop, &area, y, part)
        });
        if !ok {
            return Err(STATUS_INVALID_PARAMETER);
        }
        y += n as u32;
    }
    let ok = k.with_state(|sys, _, _| {
        let w = &mut sys.win32k;
        w.surfaces.child_committed(&mut w.desktop, &area)
    });
    Ok(if ok { STATUS_SUCCESS } else { STATUS_INVALID_PARAMETER })
}

/// `RegisterEvent`: handle 0 asks which handle the thread registered (0:
/// none yet); another handle, an event the thread created, becomes its
/// queue event.
fn register_event(sys: &mut crate::NtSystem, caller: &poly_ntobj::Caller, ctx: Ctx, handle: u64) -> Reply {
    let key = (ctx.pid, ctx.tid);
    if handle == 0 {
        return Reply::Value { value: sys.win32k.events.get(&key).map_or(0, |e| e.1) };
    }
    let access = poly_ntobj::access::EVENT_MODIFY_STATE;
    let Ok(id) = sys.objects.object_by_handle(caller, Handle(handle), access, Some(ObjectType::Event)) else {
        return Reply::Error { code: 6 };
    };
    if sys.objects.reference(id).is_err() {
        return Reply::Error { code: 6 };
    }
    if let Some((old, _)) = sys.win32k.events.insert(key, (id, handle)) {
        let _ = sys.objects.dereference(old);
    }
    // What arrived before the event existed must not be lost.
    if sys.win32k.wm.has_pending(key) {
        let _ = sys.objects.set_event(id);
    }
    Reply::Ok {}
}
