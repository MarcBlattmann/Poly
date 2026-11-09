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
//! copies a window's pixels from the app into its surface buffer. The
//! kernel's `/dev/win32k` device reaches [`Win32k`]'s link methods.
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
use poly_win32k::link::{FromCompositor, Request as DesktopRequest};
use poly_win32k::proto::{Reply, Request};
use poly_win32k::services::Win32kService;
use poly_win32k::surfaces::{MAX_SIDE, Surfaces};
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
    /// Each thread's queue event (registered by `user32`) and the handle
    /// value it registered.
    events: BTreeMap<Key, (ObjectId, u64)>,
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
                FromCompositor::Request { surface, request: DesktopRequest::Close } => {
                    if let Some(hwnd) = self.surfaces.hwnd(surface) {
                        self.wm.post_system_message(hwnd, poly_win32k::user::WM_CLOSE, 0, &mut fx);
                    }
                }
                FromCompositor::Configure { surface, x, y, width, height } => {
                    if let Some(hwnd) = self.surfaces.hwnd(surface) {
                        let (width, height) = (width.min(i32::MAX as u32) as i32, height.min(i32::MAX as u32) as i32);
                        let client = Rect::new(x, y, x.saturating_add(width), y.saturating_add(height));
                        self.wm.configure_from_desktop(hwnd, client, frame);
                    }
                }
                _ => {}
            }
        }
        self.wake(objects, &fx);
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

    /// The device's `READ_BUFFER`: rows of a buffer.
    pub fn link_rows(&self, buffer: u32, row: u32, rows: u32) -> Option<&[u8]> {
        self.surfaces.rows(buffer, row, rows)
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
    let reply = k.with_state(|sys, _, _| {
        match req {
            Request::RegisterEvent { handle } => return register_event(sys, &caller, ctx, handle),
            Request::Surface { hwnd } => {
                let w = &sys.win32k;
                let size = w.surfaces.size(&w.desktop, hwnd).map_or(0, |(x, y)| u64::from(x) | (u64::from(y) << 32));
                return Reply::Value { value: size };
            }
            _ => {}
        }
        let mut fx = Effects::default();
        let reply = sys.win32k.wm.handle(ctx, req, &mut fx);
        sys.win32k.wake(&mut sys.objects, &fx);
        sys.win32k.sync();
        reply
    });
    let out = wire::encode(&reply);
    abi::write_u64(k, out_len_ptr, out.len() as u64)?;
    if out.len() as u64 > out_cap {
        return Ok(STATUS_BUFFER_TOO_SMALL);
    }
    abi::write(k, out_ptr, &out)?;
    Ok(STATUS_SUCCESS)
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
    if size != Some((width, height)) || width > MAX_SIDE || height > MAX_SIDE || stride < u64::from(width) * 4 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let row = width as usize * 4;
    let mut bytes = vec![0u8; row * height as usize];
    for (y, chunk) in bytes.chunks_mut(row).enumerate() {
        abi::read(k, pixels + y as u64 * stride, chunk)?;
    }
    let ok = k.with_state(|sys, _, _| {
        let w = &mut sys.win32k;
        w.surfaces.commit(&mut w.desktop, hwnd, width, height, &bytes)
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
