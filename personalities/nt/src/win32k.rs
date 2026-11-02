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
//! [`NtProcess::win32k_disabled`]: crate::system::NtProcess::win32k_disabled

use crate::kernel::NtKernel;
use crate::{STATUS_NOT_IMPLEMENTED, Table, abi, decode};
use alloc::collections::BTreeMap;
use alloc::vec;
use poly_abi::{Outcome, SyscallRegs};
use poly_ntobj::status::{STATUS_BUFFER_TOO_SMALL, STATUS_INVALID_PARAMETER, STATUS_SUCCESS};
use poly_ntobj::{Handle, NtStatus, ObjectId, ObjectType};
use poly_win32k::desktop::Desktop;
use poly_win32k::proto::{Reply, Request};
use poly_win32k::services::Win32kService;
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
        // The desktop link's commit comes with its slice; nothing to do yet.
        Some(Win32kService::CommitSurface) => STATUS_SUCCESS,
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
        if let Request::RegisterEvent { handle } = req {
            return register_event(sys, &caller, ctx, handle);
        }
        let mut fx = Effects::default();
        let reply = sys.win32k.wm.handle(ctx, req, &mut fx);
        sys.win32k.wake(&mut sys.objects, &fx);
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
    Reply::Ok {}
}
