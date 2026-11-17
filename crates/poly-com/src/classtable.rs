//! A process's class registrations (`CoRegisterClassObject` and its relatives, ADR 0220) and its
//! server count (`CoAddRefServerProcess`, `CoReleaseServerProcess`).
//!
//! The rules are the B2a sheets' (`specs/windows/ole32.dll/CoRegisterClassObject.yaml` and the
//! eight others) and Learn's ("REGCLS", "CLSCTX"). The table holds object pointers as numbers;
//! `ole32` adds and releases the references around the calls (never while its lock is held), and
//! publishes to the activation process what [`ClassTable::sync`] says.

use crate::git::Owner;
use crate::guid::Guid;
use crate::hresult::{E_INVALIDARG, Hresult, S_OK, from_win32};
use alloc::vec::Vec;

/// `CLSCTX` bits (`wtypes.h`).
pub const CLSCTX_INPROC_SERVER: u32 = 0x1;
pub const CLSCTX_INPROC_HANDLER: u32 = 0x2;
pub const CLSCTX_LOCAL_SERVER: u32 = 0x4;
pub const CLSCTX_REMOTE_SERVER: u32 = 0x10;
pub const CLSCTX_ACTIVATE_32_BIT_SERVER: u32 = 0x4_0000;
pub const CLSCTX_ACTIVATE_64_BIT_SERVER: u32 = 0x8_0000;
/// `CLSCTX_PS_DLL`: not a context a class object can be registered for (sheet: `E_INVALIDARG`).
pub const CLSCTX_PS_DLL: u32 = 0x8000_0000;

/// `REGCLS` values (`combaseapi.h`).
pub const REGCLS_SINGLEUSE: u32 = 0;
pub const REGCLS_MULTIPLEUSE: u32 = 1;
pub const REGCLS_MULTI_SEPARATE: u32 = 2;
pub const REGCLS_SUSPENDED: u32 = 4;
pub const REGCLS_SURROGATE: u32 = 8;
pub const REGCLS_AGILE: u32 = 0x10;
/// Every defined `REGCLS` bit; another one is `E_INVALIDARG` (sheet: flags 0x100).
const REGCLS_VALID: u32 = 0x1F;

/// `HRESULT_FROM_WIN32(RPC_S_NO_INTERFACES)` (1734, `winerror.h`): what `CoResumeClassObjects`
/// returns when nothing was waiting (sheet `CoResumeClassObjects`).
pub const RESUME_NOTHING_WAITING: Hresult = from_win32(1734);

/// One registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registration {
    pub cookie: u32,
    pub clsid: Guid,
    /// The registered object (an `IUnknown` pointer), as a number.
    pub object: usize,
    /// The context it was registered for.
    pub ctx: u32,
    pub flags: u32,
    pub owner: Owner,
    /// Registered `REGCLS_SUSPENDED` and not resumed yet: found by nobody.
    pub pending: bool,
    /// Hidden by `CoSuspendClassObjects`.
    pub hidden: bool,
    /// Hidden for good: the server count reached 0 while it existed.
    pub dead: bool,
    /// A `REGCLS_SINGLEUSE` registration that served its request.
    pub used: bool,
    /// Whether the activation process has it (so that it has to be withdrawn).
    pub published: bool,
}

impl Registration {
    fn has_local(&self) -> bool {
        self.ctx & CLSCTX_LOCAL_SERVER != 0
    }

    fn visible(&self) -> bool {
        !self.pending && !self.hidden && !self.dead && !self.used
    }

    /// The contexts lookups find it for: its own; `REGCLS_MULTIPLEUSE` with the local-server
    /// context also serves in-process requests (Learn, "REGCLS"); `MULTI_SEPARATE` does not.
    fn serves(&self) -> u32 {
        let mut c = self.ctx & (CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER);
        if self.has_local() && self.flags & REGCLS_MULTI_SEPARATE == 0 && self.flags & REGCLS_MULTIPLEUSE != 0 {
            c |= CLSCTX_INPROC_SERVER;
        }
        c
    }

    fn single(&self) -> bool {
        self.flags & (REGCLS_MULTIPLEUSE | REGCLS_MULTI_SEPARATE) == 0
    }
}

/// What a lookup found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// In the caller's apartment: the registered pointer itself.
    Here(usize),
    /// A local-server registration of another apartment of this process: a proxy to it.
    Elsewhere {
        owner: Owner,
        object: usize,
    },
    Nothing,
}

/// What a revoke did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Revoked {
    /// The registration is gone: release `object`; withdraw it when `published`.
    Removed { object: usize, published: bool },
    /// A registration of another apartment's STA: it stays until that apartment revokes it or ends
    /// (the visible test finds the class still served right after such a revoke).
    Deferred,
    /// No registration has this cookie: nothing happens (`S_OK` all the same).
    Unknown,
}

/// What the activation process has to be told after a change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Publications {
    /// Registrations to publish: cookie, class id, object, single use.
    pub publish: Vec<(u32, Guid, usize, bool)>,
    /// Cookies to withdraw.
    pub withdraw: Vec<u32>,
}

impl Publications {
    pub fn is_empty(&self) -> bool {
        self.publish.is_empty() && self.withdraw.is_empty()
    }
}

/// The first cookie (any nonzero start would do; this one is easy to spot in a log).
const FIRST_COOKIE: u32 = 0x0000_1001;

/// The process's registrations.
#[derive(Debug)]
pub struct ClassTable {
    regs: Vec<Registration>,
    next_cookie: u32,
    /// A registration was ever made in this process (`CoResumeClassObjects` with none is `S_OK`).
    ever_registered: bool,
    /// The server count reached 0: no local-server request is served any more.
    local_closed: bool,
    /// Cookies removed while published, to withdraw at the next [`ClassTable::sync`].
    withdrawn: Vec<u32>,
}

impl Default for ClassTable {
    fn default() -> Self {
        ClassTable::new()
    }
}

impl ClassTable {
    pub const fn new() -> ClassTable {
        ClassTable {
            regs: Vec::new(),
            next_cookie: FIRST_COOKIE,
            ever_registered: false,
            local_closed: false,
            withdrawn: Vec::new(),
        }
    }

    /// Check `CoRegisterClassObject`'s arguments (sheet: an undefined flag bit, a NULL object and
    /// `CLSCTX_PS_DLL` are `E_INVALIDARG`; a context of 0 is accepted).
    pub fn check(object: usize, ctx: u32, flags: u32) -> Hresult {
        if object == 0 || flags & !REGCLS_VALID != 0 || ctx & CLSCTX_PS_DLL != 0 {
            return E_INVALIDARG;
        }
        S_OK
    }

    /// Add a registration (arguments already checked); its cookie.
    pub fn register(&mut self, clsid: Guid, object: usize, ctx: u32, flags: u32, owner: Owner) -> u32 {
        let cookie = self.next_cookie;
        self.next_cookie = match self.next_cookie.wrapping_add(1) {
            0 => 1,
            n => n,
        };
        self.ever_registered = true;
        self.regs.push(Registration {
            cookie,
            clsid,
            object,
            ctx,
            flags,
            owner,
            pending: flags & REGCLS_SUSPENDED != 0,
            hidden: false,
            dead: false,
            used: false,
            published: false,
        });
        cookie
    }

    /// `CoRevokeClassObject(cookie)` from `caller`'s apartment.
    pub fn revoke(&mut self, cookie: u32, caller: Owner) -> Revoked {
        let Some(i) = self.regs.iter().position(|r| r.cookie == cookie) else { return Revoked::Unknown };
        let r = self.regs[i];
        if matches!(r.owner, Owner::Sta(_)) && r.owner != caller {
            return Revoked::Deferred;
        }
        self.regs.remove(i);
        if r.published {
            self.withdrawn.push(cookie);
        }
        Revoked::Removed { object: r.object, published: r.published }
    }

    /// The apartment `owner` ended: its registrations go; the objects to release.
    pub fn end_apartment(&mut self, owner: Owner) -> Vec<usize> {
        let mut gone = Vec::new();
        let mut i = 0;
        while i < self.regs.len() {
            if self.regs[i].owner == owner {
                let r = self.regs.remove(i);
                if r.published {
                    self.withdrawn.push(r.cookie);
                }
                gone.push(r.object);
            } else {
                i += 1;
            }
        }
        gone
    }

    /// Find the class object of `clsid` for a request with context `ctx` from `caller`'s
    /// apartment. A single-use registration is used up by the lookup that finds it.
    pub fn lookup(&mut self, clsid: &Guid, ctx: u32, caller: Owner) -> Found {
        let mut want = ctx & (CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER);
        if self.local_closed {
            want &= !CLSCTX_LOCAL_SERVER;
        }
        if want == 0 {
            return Found::Nothing;
        }
        let candidates = |r: &Registration| r.clsid == *clsid && r.visible() && r.serves() & want != 0;
        // Newest first, the caller's apartment before the others.
        let here = self.regs.iter().rposition(|r| candidates(r) && r.owner == caller);
        let found = here.or_else(|| self.regs.iter().rposition(|r| candidates(r) && r.has_local()));
        let Some(i) = found else { return Found::Nothing };
        let r = &mut self.regs[i];
        if r.single() {
            r.used = true;
        }
        if r.owner == caller { Found::Here(r.object) } else { Found::Elsewhere { owner: r.owner, object: r.object } }
    }

    /// `CoSuspendClassObjects`: every local-server registration is hidden.
    pub fn suspend(&mut self) {
        for r in &mut self.regs {
            if r.has_local() && !r.pending {
                r.hidden = true;
            }
        }
    }

    /// `CoResumeClassObjects`: the suspended local-server registrations become visible
    /// (in-process-only ones stay as they are: never found); registrations for both contexts that
    /// `CoSuspendClassObjects` hid come back. The result: `S_OK` when something was waiting or the
    /// process never registered anything, else [`RESUME_NOTHING_WAITING`].
    pub fn resume(&mut self) -> Hresult {
        let mut resumed = 0;
        for r in &mut self.regs {
            if r.pending && r.has_local() && !r.dead {
                r.pending = false;
                resumed += 1;
            }
            if r.hidden && r.ctx & CLSCTX_INPROC_SERVER != 0 && !r.dead {
                r.hidden = false;
            }
        }
        if resumed > 0 || !self.ever_registered { S_OK } else { RESUME_NOTHING_WAITING }
    }

    /// The server count reached 0: every local-server registration is dead, and local-server
    /// requests are not served any more (also for registrations made later).
    pub fn close_local(&mut self) {
        self.local_closed = true;
        for r in &mut self.regs {
            if r.has_local() {
                r.dead = true;
            }
        }
    }

    /// What has to change at the activation process: registrations that became servable to other
    /// processes and published ones that stopped being. Marks them as done.
    pub fn sync(&mut self) -> Publications {
        let mut p = Publications { publish: Vec::new(), withdraw: core::mem::take(&mut self.withdrawn) };
        let closed = self.local_closed;
        for r in &mut self.regs {
            let servable = r.has_local() && r.visible() && !closed;
            if servable && !r.published {
                r.published = true;
                p.publish.push((r.cookie, r.clsid, r.object, r.single()));
            } else if !servable && r.published {
                r.published = false;
                p.withdraw.push(r.cookie);
            }
        }
        p
    }

    /// The registration with `cookie`, if any.
    pub fn get(&self, cookie: u32) -> Option<Registration> {
        self.regs.iter().find(|r| r.cookie == cookie).copied()
    }

    /// The activation process served a single-use registration to another process.
    pub fn used_remotely(&mut self, cookie: u32) {
        if let Some(r) = self.regs.iter_mut().find(|r| r.cookie == cookie) {
            r.used = true;
        }
    }

    pub fn len(&self) -> usize {
        self.regs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regs.is_empty()
    }
}

/// The process's server count (sheets `CoAddRefServerProcess`, `CoReleaseServerProcess`): one
/// 32-bit counter that wraps. A release at 0 stays 0 until the counter was raised or COM was
/// initialised in the process ("armed"); after that it wraps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerCount {
    count: u32,
    armed: bool,
}

impl ServerCount {
    pub const fn new() -> ServerCount {
        ServerCount { count: 0, armed: false }
    }

    /// COM was initialised in the process.
    pub fn arm(&mut self) {
        self.armed = true;
    }

    /// `CoAddRefServerProcess`: the new count.
    pub fn add(&mut self) -> u32 {
        self.armed = true;
        self.count = self.count.wrapping_add(1);
        self.count
    }

    /// `CoReleaseServerProcess`: the new count, and whether this release brought it to 0 (then the
    /// caller closes the class table for local-server requests).
    pub fn release(&mut self) -> (u32, bool) {
        if self.count == 0 && !self.armed {
            return (0, false);
        }
        self.count = self.count.wrapping_sub(1);
        (self.count, self.count == 0)
    }

    /// COM's own hold while an activation from another process runs (sheet: the count is already
    /// 1 inside `CreateInstance`); it never hides anything.
    pub fn enter_call(&mut self) {
        self.count = self.count.wrapping_add(1);
    }

    pub fn leave_call(&mut self) {
        self.count = self.count.wrapping_sub(1);
    }

    pub fn value(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hresult::E_INVALIDARG;

    const C1: Guid =
        Guid { data1: 0xB2A0_C001, data2: 0, data3: 0x4000, data4: [0x80, 0, 0x70, 0x6F, 0x6C, 0x79, 0x77, 0x73] };
    const C2: Guid =
        Guid { data1: 0xB2A0_C002, data2: 0, data3: 0x4000, data4: [0x80, 0, 0x70, 0x6F, 0x6C, 0x79, 0x77, 0x73] };
    const F: usize = 0x1000;
    const MTA: Owner = Owner::Mta;
    const STA: Owner = Owner::Sta(7);
    const LOCAL: u32 = CLSCTX_LOCAL_SERVER;
    const INPROC: u32 = CLSCTX_INPROC_SERVER;
    const ALL: u32 = 0x17;

    #[test]
    fn arguments() {
        assert_eq!(ClassTable::check(F, INPROC, 0x100), E_INVALIDARG);
        assert_eq!(ClassTable::check(0, INPROC, 1), E_INVALIDARG);
        assert_eq!(ClassTable::check(F, 0x8000_0000, 1), E_INVALIDARG);
        assert_eq!(ClassTable::check(F, 0, 1), S_OK);
    }

    #[test]
    fn cookies_differ_and_are_not_reused() {
        let mut t = ClassTable::new();
        let a = t.register(C1, F, INPROC, 1, MTA);
        let b = t.register(C1, F, INPROC, 1, MTA);
        assert_ne!(a, b);
        assert!(a != 0 && b != 0);
        assert_eq!(t.revoke(a, MTA), Revoked::Removed { object: F, published: false });
        assert_eq!(t.lookup(&C1, INPROC, MTA), Found::Here(F));
        assert_eq!(t.revoke(a, MTA), Revoked::Unknown);
        let c = t.register(C1, F, INPROC, 1, MTA);
        assert!(c != a && c != b);
        t.revoke(b, MTA);
        t.revoke(c, MTA);
        assert_eq!(t.lookup(&C1, INPROC, MTA), Found::Nothing);
    }

    #[test]
    fn contexts_in_the_registering_apartment() {
        let mut t = ClassTable::new();
        t.register(C1, F, INPROC, 1, STA);
        assert_eq!(t.lookup(&C1, INPROC, STA), Found::Here(F));
        assert_eq!(t.lookup(&C1, ALL, STA), Found::Here(F));
        assert_eq!(t.lookup(&C1, LOCAL, STA), Found::Nothing);
        t.register(C2, F, LOCAL, 1, STA);
        for ctx in [INPROC, LOCAL, ALL] {
            assert_eq!(t.lookup(&C2, ctx, STA), Found::Here(F));
        }
    }

    #[test]
    fn single_use_serves_once() {
        let mut t = ClassTable::new();
        t.register(C1, F, INPROC, REGCLS_SINGLEUSE, MTA);
        assert_eq!(t.lookup(&C1, INPROC, MTA), Found::Here(F));
        assert_eq!(t.lookup(&C1, INPROC, MTA), Found::Nothing);
    }

    #[test]
    fn apartments() {
        let mut t = ClassTable::new();
        t.register(C1, F, INPROC, 1, STA);
        assert_eq!(t.lookup(&C1, INPROC, MTA), Found::Nothing);
        t.register(C2, F, LOCAL, 1, STA);
        assert_eq!(t.lookup(&C2, LOCAL, MTA), Found::Elsewhere { owner: STA, object: F });
        // A revoke from another apartment of an STA's registration waits for the STA.
        let k = t.register(C2, F, LOCAL | INPROC, 1, STA);
        assert_eq!(t.revoke(k, MTA), Revoked::Deferred);
        assert!(matches!(t.revoke(k, STA), Revoked::Removed { .. }));
        assert_eq!(t.end_apartment(STA), [F, F]);
        assert!(t.is_empty());
    }

    #[test]
    fn suspended_registrations_and_resume() {
        let mut t = ClassTable::new();
        assert_eq!(t.resume(), S_OK);
        t.register(C1, F, LOCAL, REGCLS_MULTIPLEUSE | REGCLS_SUSPENDED, MTA);
        t.register(C1, F, LOCAL, REGCLS_MULTIPLEUSE | REGCLS_SUSPENDED, MTA);
        assert_eq!(t.lookup(&C1, LOCAL, MTA), Found::Nothing);
        assert_eq!(t.resume(), S_OK);
        assert_eq!(t.lookup(&C1, LOCAL, MTA), Found::Here(F));
        assert_eq!(t.resume(), RESUME_NOTHING_WAITING);
        // In-process only: never found, nothing to resume.
        t.register(C2, F, INPROC, REGCLS_MULTIPLEUSE | REGCLS_SUSPENDED, MTA);
        assert_eq!(t.resume(), RESUME_NOTHING_WAITING);
        assert_eq!(t.lookup(&C2, INPROC, MTA), Found::Nothing);
    }

    #[test]
    fn suspend_hides_local_registrations() {
        let mut t = ClassTable::new();
        let cx = t.register(C1, F, LOCAL, REGCLS_MULTIPLEUSE, MTA);
        let cy = t.register(C2, F, LOCAL, REGCLS_MULTI_SEPARATE, MTA);
        t.suspend();
        assert_eq!(t.lookup(&C1, LOCAL, MTA), Found::Nothing);
        assert_eq!(t.lookup(&C1, INPROC, MTA), Found::Nothing);
        let again = t.register(C1, F, LOCAL, REGCLS_MULTIPLEUSE, MTA);
        assert_eq!(t.lookup(&C1, LOCAL, MTA), Found::Here(F));
        assert_eq!(t.resume(), RESUME_NOTHING_WAITING);
        assert_eq!(t.lookup(&C2, LOCAL, MTA), Found::Nothing);
        for k in [cx, cy, again] {
            t.revoke(k, MTA);
        }
        // Both contexts: back after the resume.
        t.register(C2, F, LOCAL | INPROC, REGCLS_MULTI_SEPARATE, MTA);
        t.suspend();
        assert_eq!(t.lookup(&C2, LOCAL | INPROC, MTA), Found::Nothing);
        t.resume();
        assert_eq!(t.lookup(&C2, LOCAL | INPROC, MTA), Found::Here(F));
        // In-process only: not touched.
        t.register(C1, F, INPROC, REGCLS_MULTIPLEUSE, MTA);
        t.suspend();
        assert_eq!(t.lookup(&C1, INPROC, MTA), Found::Here(F));
    }

    #[test]
    fn server_count_zero_closes_local() {
        let mut t = ClassTable::new();
        let mut n = ServerCount::new();
        t.register(C1, F, INPROC, 1, MTA);
        assert_eq!(n.add(), 1);
        let (v, zero) = n.release();
        assert_eq!((v, zero), (0, true));
        t.close_local();
        assert_eq!(t.lookup(&C1, INPROC, MTA), Found::Here(F));
        t.register(C2, F, LOCAL | INPROC, 1, MTA);
        assert_eq!(t.lookup(&C2, INPROC, MTA), Found::Here(F));
        assert_eq!(t.lookup(&C2, LOCAL, MTA), Found::Nothing);
        t.close_local();
        assert_eq!(t.lookup(&C2, INPROC, MTA), Found::Nothing);
    }

    #[test]
    fn server_count_wraps_once_armed() {
        let mut n = ServerCount::new();
        assert_eq!(n.release(), (0, false));
        assert_eq!(n.release(), (0, false));
        assert_eq!(n.add(), 1);
        assert_eq!(n.add(), 2);
        assert_eq!(n.release().0, 1);
        assert_eq!(n.release(), (0, true));
        assert_eq!(n.release(), (u32::MAX, false));
        assert_eq!(n.release(), (u32::MAX - 1, false));
        assert_eq!(n.add(), u32::MAX);
        assert_eq!(n.add(), 0);
        assert_eq!(n.add(), 1);
        let mut m = ServerCount::new();
        m.arm();
        assert_eq!(m.release().0, u32::MAX);
    }

    #[test]
    fn calls_raise_the_count_without_hiding() {
        let mut n = ServerCount::new();
        n.enter_call();
        assert_eq!(n.add(), 2);
        n.leave_call();
        assert_eq!(n.release(), (0, true));
    }

    #[test]
    fn publications_follow_the_state() {
        let mut t = ClassTable::new();
        let k = t.register(C1, F, LOCAL, REGCLS_MULTIPLEUSE | REGCLS_SUSPENDED, MTA);
        t.register(C2, F, INPROC, 1, MTA);
        assert!(t.sync().is_empty());
        t.resume();
        assert_eq!(t.sync().publish, [(k, C1, F, false)]);
        assert!(t.sync().is_empty());
        t.suspend();
        assert_eq!(t.sync().withdraw, [k]);
        let k2 = t.register(C1, F, LOCAL, REGCLS_SINGLEUSE, MTA);
        assert_eq!(t.sync().publish, [(k2, C1, F, true)]);
        t.revoke(k2, MTA);
        assert_eq!(t.sync().withdraw, [k2]);
    }
}
