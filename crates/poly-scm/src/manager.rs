//! The service control manager's state (ADR 0221): the database, the handles, the status of each
//! service and the life of its process. Pure logic: the NT personality calls it with who is
//! calling and the time, and carries out the [`Effect`]s it asks for (set an event, start a
//! process through the host, end a process) and the registry writes ([`Scm::sync`]).
//!
//! Behaviour from the spec sheets of the 20 service entries and `SERVICES.md`
//! (`specs/windows/advapi32.dll/`).

use crate::config::{self, Config, eq_fold, fold};
use crate::consts::*;
use crate::wire::{Config2, ConfigReply, FailureActions, Status};
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;
use poly_registry::{CreateOptions, KeyId, Registry};
use poly_security::access::{self, GenericMapping};
use poly_security::sd::Parts;
use poly_security::token::Token;

/// `GENERIC_MAPPING` of a service (Learn: "Service Security and Access Rights").
pub const SERVICE_MAPPING: GenericMapping = GenericMapping {
    read: STANDARD_RIGHTS_READ
        | SERVICE_QUERY_CONFIG
        | SERVICE_QUERY_STATUS
        | SERVICE_INTERROGATE
        | SERVICE_ENUMERATE_DEPENDENTS,
    write: STANDARD_RIGHTS_WRITE | SERVICE_CHANGE_CONFIG,
    execute: STANDARD_RIGHTS_EXECUTE
        | SERVICE_START
        | SERVICE_STOP
        | SERVICE_PAUSE_CONTINUE
        | SERVICE_USER_DEFINED_CONTROL,
    all: SERVICE_ALL_ACCESS,
};

/// `GENERIC_MAPPING` of the manager (the same Learn page).
pub const MANAGER_MAPPING: GenericMapping = GenericMapping {
    read: STANDARD_RIGHTS_READ | SC_MANAGER_ENUMERATE_SERVICE | SC_MANAGER_QUERY_LOCK_STATUS,
    write: STANDARD_RIGHTS_WRITE | SC_MANAGER_CREATE_SERVICE | SC_MANAGER_MODIFY_BOOT_CONFIG,
    execute: STANDARD_RIGHTS_EXECUTE | SC_MANAGER_CONNECT | SC_MANAGER_LOCK,
    all: SC_MANAGER_ALL_ACCESS,
};

/// The descriptor of a new service (SERVICES.md section 2).
pub const SERVICE_SDDL: &str = "O:SYG:SYD:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)S:(AU;FA;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;WD)";

/// The descriptor of the manager (SERVICES.md section 2).
pub const MANAGER_SDDL: &str = "O:SYG:SYD:(A;;CC;;;AU)(A;;CCLCRPRC;;;IU)(A;;CCLCRPRC;;;SU)(A;;CCLCRPWPRC;;;SY)(A;;KA;;;BA)(A;;CC;;;AC)(A;;CC;;;S-1-15-3-1024-528118966-3876874398-709513571-1907873084-3598227634-3698730060-278077788-3990600205)S:(AU;FA;KA;;;WD)(AU;OIIOFA;GA;;;WD)";

/// The services Poly's manager provides itself, always running: `RpcSs`, which Google's services
/// depend on (SERVICES.md section 3, item 10). Its values are Windows' defaults for it.
const BUILTIN: &[(&str, &str, u32, &str, &str)] = &[(
    "RpcSs",
    "Remote Procedure Call (RPC)",
    SERVICE_WIN32_SHARE_PROCESS,
    "%SystemRoot%\\system32\\svchost.exe -k rpcss -p",
    "NT AUTHORITY\\NetworkService",
)];

/// The first handle value; handles go up in steps of 4.
const FIRST_HANDLE: u32 = 0x00CA_0004;

/// Something the caller of the manager has to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Signal a waiter's one-shot event and drop the manager's reference to it.
    Signal(u64),
    /// Set a lasting event (a dispatcher's or the host's), keeping the reference.
    Set(u64),
    /// Drop the manager's reference to an event without signalling it.
    Release(u64),
    /// The host has work: start it if it is not running, else set its event.
    WakeHost,
    /// End a process (a service that did not connect in time).
    Terminate(u32),
}

/// Who is calling: the process and its effective token (`None`: the kernel, every right).
#[derive(Clone, Copy, Debug)]
pub struct Caller<'a> {
    pub pid: u32,
    pub token: Option<&'a Token>,
    /// Milliseconds since boot.
    pub now: u64,
}

/// A service process to start (the host does it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawn {
    /// The service's name (its key in the database).
    pub name: Vec<u16>,
    /// `ImagePath` with its variables expanded: the command line; its first word is the program.
    pub command_line: Vec<u16>,
    /// `ObjectName`.
    pub account: Vec<u16>,
    /// Whether its token has its service SID (`ServiceSidType` 1 or 3).
    pub service_sid: bool,
}

/// What the dispatcher of a service process gets next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Nothing,
    Control(u32),
    End,
}

/// What a connecting dispatcher gets: the service's name, its arguments (the name first) and what
/// to do.
pub type Connected = (Vec<u16>, Vec<Vec<u16>>, Vec<Effect>);

/// One service of a listing: its name, display name and status.
pub type Listed = (Vec<u16>, Vec<u16>, Status);

/// How `ControlService` goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Controlled {
    /// Answered at once with this status.
    Done(Status),
    /// Delivered to the service: wait for the waiter event, then ask for the status.
    Pending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Manager,
    Service(Vec<u16>),
}

#[derive(Clone, Debug)]
struct ScHandle {
    pid: u32,
    kind: Kind,
    granted: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// No process.
    Idle,
    /// Started, waiting for its dependencies to run.
    Deps,
    /// Waiting for the host to start its process.
    Spawn,
    /// The host is starting it.
    Spawning,
    /// Its process runs; it has not called the dispatcher yet.
    Connect,
    /// Its dispatcher is connected.
    Connected,
}

#[derive(Clone, Copy, Debug)]
struct Ctl {
    code: u32,
    waiter: Option<u64>,
}

/// One service.
#[derive(Clone, Debug)]
pub struct Service {
    pub config: Config,
    /// Its security descriptor, self-relative.
    pub sd: Vec<u8>,
    pub status: Status,
    /// Provided by the manager itself (always running).
    pub builtin: bool,
    process: Option<u32>,
    phase: Phase,
    starter: Option<u64>,
    start_result: Option<u32>,
    args: Vec<Vec<u16>>,
    deadline: Option<u64>,
    timed_out: bool,
    control_event: Option<u64>,
    controls: VecDeque<Ctl>,
    in_flight: Option<Ctl>,
    reported_stopped: bool,
    failures: u32,
    last_failure: u64,
    restart_at: Option<u64>,
    handles: u32,
}

impl Service {
    fn new(config: Config, sd: Vec<u8>) -> Service {
        Service {
            status: Status {
                ty: config.ty,
                state: SERVICE_STOPPED,
                win32: ERROR_SERVICE_NEVER_STARTED,
                ..Status::default()
            },
            config,
            sd,
            builtin: false,
            process: None,
            phase: Phase::Idle,
            starter: None,
            start_result: None,
            args: Vec::new(),
            deadline: None,
            timed_out: false,
            control_event: None,
            controls: VecDeque::new(),
            in_flight: None,
            reported_stopped: false,
            failures: 0,
            last_failure: 0,
            restart_at: None,
            handles: 0,
        }
    }

    /// The process that runs it, if any.
    pub fn process(&self) -> Option<u32> {
        self.process
    }
}

/// What `CreateServiceW` asks for (`None`: a NULL pointer).
#[derive(Clone, Debug, Default)]
pub struct Create {
    pub name: Option<Vec<u16>>,
    pub display: Option<Vec<u16>>,
    pub access: u32,
    pub ty: u32,
    pub start: u32,
    pub error: u32,
    pub path: Option<Vec<u16>>,
    pub group: Option<Vec<u16>>,
    pub deps: Option<Vec<Vec<u16>>>,
    pub account: Option<Vec<u16>>,
    pub wow64: bool,
}

/// What `ChangeServiceConfigW` asks for (`SERVICE_NO_CHANGE` and `None` leave a field).
#[derive(Clone, Debug, Default)]
pub struct Change {
    pub ty: u32,
    pub start: u32,
    pub error: u32,
    pub path: Option<Vec<u16>>,
    pub group: Option<Vec<u16>>,
    pub deps: Option<Vec<Vec<u16>>>,
    pub account: Option<Vec<u16>>,
    pub display: Option<Vec<u16>>,
}

/// The manager.
#[derive(Debug, Default)]
pub struct Scm {
    services: BTreeMap<Vec<u16>, Service>,
    handles: BTreeMap<u32, ScHandle>,
    next_handle: u32,
    /// Processes that called the dispatcher (a second call is 1056).
    tried: BTreeSet<u32>,
    /// The host process (`services.exe`) and its work event.
    host: Option<(u32, Option<u64>)>,
    manager_sd: Vec<u8>,
    loaded: bool,
    autostarted: bool,
    /// Services whose registry key has to be written or removed ([`Scm::sync`]).
    dirty: BTreeSet<Vec<u16>>,
    removed: Vec<Vec<u16>>,
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn sd_of(sddl: &str) -> Vec<u8> {
    poly_security::sdstr::parse(&w(sddl)).unwrap_or_default()
}

/// The access check of `token` against `sd` for `desired` (generic bits mapped): what is granted,
/// or 5.
fn check(token: Option<&Token>, sd: &[u8], desired: u32, mapping: &GenericMapping) -> Result<u32, u32> {
    let Some(t) = token else {
        return Ok(
            mapping.map(desired & !MAXIMUM_ALLOWED) | if desired & MAXIMUM_ALLOWED != 0 { mapping.all } else { 0 }
        );
    };
    let Some(parts) = Parts::from_relative(sd) else { return Err(ERROR_ACCESS_DENIED) };
    let out = access::check(&parts, &t.check_view(), mapping.map(desired), mapping);
    match out.status {
        Ok(()) => Ok(out.granted),
        Err(_) => Err(ERROR_ACCESS_DENIED),
    }
}

/// Where the database lives: `HKLM\SYSTEM\ControlSet001\Services` (what `CurrentControlSet` links
/// to).
pub const SERVICES_PATH: &str = "SYSTEM\\ControlSet001\\Services";

/// Whether the registry holds a service to start at boot (`Start` 2, not a driver, not marked
/// for deletion, not one the manager provides itself), without changing anything.
pub fn autostart_in_registry(reg: &Registry) -> bool {
    let Some(key) = reg.find(reg.machine(), SERVICES_PATH) else { return false };
    let mut index = 0;
    while let Ok(sub) = reg.subkey_at(key, index) {
        index += 1;
        let Ok(k) = reg.key(sub) else { continue };
        let values: Vec<_> = k.values().iter().map(|v| (v.name.clone(), v.kind, v.data.clone())).collect();
        let builtin = BUILTIN.iter().any(|b| eq_fold(&w(b.0), k.name()));
        if let Some(c) = Config::from_values(k.name(), &values)
            && c.start == SERVICE_AUTO_START
            && !c.delete_flag
            && !builtin
        {
            return true;
        }
    }
    false
}

/// The `Services` key, made if needed.
fn services_key(reg: &mut Registry, now: u64) -> Option<KeyId> {
    let mut cur = reg.machine();
    for part in SERVICES_PATH.split('\\') {
        // Under a volatile parent (only in a registry without hives) a key has to be volatile too.
        let volatile = reg.key(cur).ok()?.is_volatile();
        let opts = CreateOptions { volatile, link: false, class: &[] };
        cur = reg.create_key(cur, &w(part), opts, now).ok()?.0;
    }
    Some(cur)
}

impl Scm {
    pub fn new() -> Scm {
        Scm { next_handle: FIRST_HANDLE, manager_sd: sd_of(MANAGER_SDDL), ..Scm::default() }
    }

    /// Read the database from the registry once: every key under `Services` that is a service,
    /// without the ones marked for deletion (they go at boot), and the built-in services.
    pub fn load(&mut self, reg: &mut Registry, now_ft: u64) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if self.manager_sd.is_empty() {
            self.manager_sd = sd_of(MANAGER_SDDL);
        }
        if self.next_handle == 0 {
            self.next_handle = FIRST_HANDLE;
        }
        let Some(key) = services_key(reg, now_ft) else { return };
        let mut index = 0;
        while let Ok(sub) = reg.subkey_at(key, index) {
            index += 1;
            let Ok(k) = reg.key(sub) else { continue };
            let name = k.name().to_vec();
            let values: Vec<_> = k.values().iter().map(|v| (v.name.clone(), v.kind, v.data.clone())).collect();
            let Some(c) = Config::from_values(&name, &values) else { continue };
            if c.delete_flag {
                self.removed.push(name);
                continue;
            }
            self.services.insert(fold(&name), Service::new(c, sd_of(SERVICE_SDDL)));
        }
        for &(name, display, ty, path, account) in BUILTIN {
            let key = fold(&w(name));
            let s = self.services.entry(key.clone()).or_insert_with(|| {
                let c = Config {
                    name: w(name),
                    display: Some(w(display)),
                    ty,
                    start: SERVICE_AUTO_START,
                    error: 1,
                    path: w(path),
                    account: w(account),
                    ..Config::default()
                };
                Service::new(c, sd_of(SERVICE_SDDL))
            });
            s.builtin = true;
            s.status = Status { ty: s.config.ty, state: SERVICE_RUNNING, ..Status::default() };
            self.dirty.insert(key);
        }
        self.sync(reg, now_ft);
    }

    /// Write the keys of the services that changed and remove the deleted ones.
    pub fn sync(&mut self, reg: &mut Registry, now_ft: u64) {
        if self.dirty.is_empty() && self.removed.is_empty() {
            return;
        }
        let Some(root) = services_key(reg, now_ft) else { return };
        for name in core::mem::take(&mut self.removed) {
            if let Some(k) = reg.child(root, &name) {
                remove_tree(reg, k, now_ft);
            }
        }
        for key in core::mem::take(&mut self.dirty) {
            let Some(s) = self.services.get(&key) else { continue };
            let volatile = reg.key(root).is_ok_and(|k| k.is_volatile());
            let opts = CreateOptions { volatile, link: false, class: &[] };
            let Ok((k, _)) = reg.create_key(root, &s.config.name, opts, now_ft) else { continue };
            let values = s.config.values();
            for n in Config::managed_names() {
                if !values.iter().any(|(v, _, _)| eq_fold(v, &n)) && reg.value(k, &n).is_ok() {
                    let _ = reg.delete_value(k, &n, now_ft);
                }
            }
            for (n, kind, data) in &values {
                let _ = reg.set_value(k, n, *kind, data, now_ft);
            }
        }
    }

    /// The service called `name` (any case).
    pub fn service(&self, name: &[u16]) -> Option<&Service> {
        self.services.get(&fold(name))
    }

    /// Every service, in the database's order.
    pub fn services(&self) -> impl Iterator<Item = &Service> {
        self.services.values()
    }

    /// The host process, once it called in.
    pub fn host_pid(&self) -> Option<u32> {
        self.host.map(|(p, _)| p)
    }

    /// Whether there is anything to start at boot (an auto-start service that is not built in).
    pub fn has_autostart(&self) -> bool {
        self.services.values().any(|s| !s.builtin && s.config.start == SERVICE_AUTO_START && !s.config.delete_flag)
    }

    // ----- handles

    fn new_handle(&mut self, pid: u32, kind: Kind, granted: u32) -> u32 {
        if self.next_handle == 0 {
            self.next_handle = FIRST_HANDLE;
        }
        let h = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(4);
        if let Kind::Service(k) = &kind
            && let Some(s) = self.services.get_mut(k)
        {
            s.handles += 1;
        }
        self.handles.insert(h, ScHandle { pid, kind, granted });
        h
    }

    fn handle(&self, c: &Caller<'_>, h: u64) -> Result<&ScHandle, u32> {
        let h = u32::try_from(h).map_err(|_| ERROR_INVALID_HANDLE)?;
        self.handles.get(&h).filter(|x| x.pid == c.pid).ok_or(ERROR_INVALID_HANDLE)
    }

    fn manager(&self, c: &Caller<'_>, h: u64) -> Result<u32, u32> {
        match self.handle(c, h)? {
            ScHandle { kind: Kind::Manager, granted, .. } => Ok(*granted),
            _ => Err(ERROR_INVALID_HANDLE),
        }
    }

    /// A service handle: the service's key and the rights of the handle.
    fn service_of(&self, c: &Caller<'_>, h: u64) -> Result<(Vec<u16>, u32), u32> {
        match self.handle(c, h)? {
            ScHandle { kind: Kind::Service(k), granted, .. } if self.services.contains_key(k) => {
                Ok((k.clone(), *granted))
            }
            _ => Err(ERROR_INVALID_HANDLE),
        }
    }

    fn service_with(&self, c: &Caller<'_>, h: u64, right: u32) -> Result<Vec<u16>, u32> {
        let (k, granted) = self.service_of(c, h)?;
        if granted & right != right {
            return Err(ERROR_ACCESS_DENIED);
        }
        Ok(k)
    }

    // ----- the calls

    /// `OpenSCManagerW` (the names are checked by `advapi32`).
    pub fn open_manager(&mut self, c: &Caller<'_>, access: u32) -> Result<u32, u32> {
        let desired = if access & MAXIMUM_ALLOWED != 0 { access } else { access | SC_MANAGER_CONNECT };
        let granted = check(c.token, &self.manager_sd, desired, &MANAGER_MAPPING)?;
        Ok(self.new_handle(c.pid, Kind::Manager, granted))
    }

    /// `CreateServiceW`.
    pub fn create(&mut self, c: &Caller<'_>, mgr: u64, req: Create) -> Result<u32, u32> {
        let granted = self.manager(c, mgr)?;
        let name = req.name.ok_or(ERROR_INVALID_ADDRESS)?;
        let path = req.path.ok_or(ERROR_INVALID_ADDRESS)?;
        if granted & SC_MANAGER_CREATE_SERVICE == 0 {
            return Err(ERROR_ACCESS_DENIED);
        }
        config::check_name(&name)?;
        if let Some(d) = &req.display
            && d.len() > MAX_NAME
        {
            return Err(RPC_X_BAD_STUB_DATA);
        }
        if path.is_empty() || !config::type_valid(req.ty) || !config::start_valid(req.ty, req.start) || req.error > 3 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let deps = req.deps.unwrap_or_default();
        if deps.iter().any(|d| eq_fold(d, &name)) {
            return Err(ERROR_CIRCULAR_DEPENDENCY);
        }
        let account = match &req.account {
            None => w("LocalSystem"),
            Some(a) => {
                let acct = poly_security::service::account_of(a).ok_or(ERROR_INVALID_SERVICE_ACCOUNT)?;
                if acct == poly_security::service::Account::LocalSystem && a.is_empty() {
                    w("LocalSystem")
                } else {
                    a.clone()
                }
            }
        };
        let key = fold(&name);
        if let Some(old) = self.services.get(&key) {
            return Err(if old.config.delete_flag { ERROR_SERVICE_MARKED_FOR_DELETE } else { ERROR_SERVICE_EXISTS });
        }
        self.check_display_unique(&key, &name, req.display.as_deref())?;
        let cfg = Config {
            name,
            display: req.display,
            ty: req.ty,
            start: req.start,
            error: req.error,
            path,
            group: req.group.unwrap_or_default(),
            deps,
            account,
            wow64: req.wow64,
            ..Config::default()
        };
        let sd = sd_of(SERVICE_SDDL);
        let rights = SERVICE_MAPPING.map(req.access & !MAXIMUM_ALLOWED)
            | if req.access & MAXIMUM_ALLOWED != 0 { SERVICE_ALL_ACCESS } else { 0 };
        self.services.insert(key.clone(), Service::new(cfg, sd));
        self.dirty.insert(key.clone());
        Ok(self.new_handle(c.pid, Kind::Service(key), rights))
    }

    /// A display name may not be another service's name, nor a name another service's display name.
    fn check_display_unique(&self, key: &[u16], name: &[u16], display: Option<&[u16]>) -> Result<(), u32> {
        for (k, s) in &self.services {
            if k.as_slice() == key {
                continue;
            }
            if let Some(d) = display
                && eq_fold(d, &s.config.name)
            {
                return Err(ERROR_DUPLICATE_SERVICE_NAME);
            }
            if let Some(d) = &s.config.display
                && eq_fold(d, name)
            {
                return Err(ERROR_DUPLICATE_SERVICE_NAME);
            }
        }
        Ok(())
    }

    /// `OpenServiceW`.
    pub fn open_service(&mut self, c: &Caller<'_>, mgr: u64, name: Option<&[u16]>, access: u32) -> Result<u32, u32> {
        self.manager(c, mgr)?;
        let name = name.ok_or(ERROR_INVALID_ADDRESS)?;
        config::check_name(name)?;
        let key = fold(name);
        let s = self.services.get(&key).ok_or(ERROR_SERVICE_DOES_NOT_EXIST)?;
        let granted = check(c.token, &s.sd, access, &SERVICE_MAPPING)?;
        Ok(self.new_handle(c.pid, Kind::Service(key), granted))
    }

    /// `CloseServiceHandle`.
    pub fn close(&mut self, c: &Caller<'_>, h: u64) -> Result<(), u32> {
        self.handle(c, h)?;
        let x = self.handles.remove(&(h as u32)).expect("checked");
        if let Kind::Service(k) = x.kind {
            if let Some(s) = self.services.get_mut(&k) {
                s.handles = s.handles.saturating_sub(1);
            }
            self.maybe_remove(&k);
        }
        Ok(())
    }

    /// Remove a service marked for deletion once nothing holds it.
    fn maybe_remove(&mut self, key: &[u16]) {
        let Some(s) = self.services.get(key) else { return };
        if s.config.delete_flag && s.handles == 0 && s.process.is_none() && s.phase == Phase::Idle {
            let s = self.services.remove(key).expect("checked");
            self.dirty.remove(key);
            self.removed.push(s.config.name);
        }
    }

    /// `DeleteService`.
    pub fn delete(&mut self, c: &Caller<'_>, h: u64) -> Result<(), u32> {
        let k = self.service_with(c, h, DELETE)?;
        let s = self.services.get_mut(&k).expect("checked");
        if s.config.delete_flag {
            return Err(ERROR_SERVICE_MARKED_FOR_DELETE);
        }
        s.config.delete_flag = true;
        s.config.start = SERVICE_DISABLED;
        s.restart_at = None;
        self.dirty.insert(k.clone());
        self.maybe_remove(&k);
        Ok(())
    }

    /// `ChangeServiceConfigW`.
    pub fn change(&mut self, c: &Caller<'_>, h: u64, ch: Change) -> Result<(), u32> {
        let k = self.service_with(c, h, SERVICE_CHANGE_CONFIG)?;
        let s = self.services.get(&k).expect("checked");
        if s.config.delete_flag {
            return Err(ERROR_SERVICE_MARKED_FOR_DELETE);
        }
        let ty = if ch.ty == SERVICE_NO_CHANGE { s.config.ty } else { ch.ty };
        let start = if ch.start == SERVICE_NO_CHANGE { s.config.start } else { ch.start };
        let error = if ch.error == SERVICE_NO_CHANGE { s.config.error } else { ch.error };
        if !config::type_valid(ty) || !config::start_valid(ty, start) || error > 3 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if ch.path.as_ref().is_some_and(Vec::is_empty) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if ch.display.as_ref().is_some_and(|d| d.len() > MAX_NAME) {
            return Err(RPC_X_BAD_STUB_DATA);
        }
        if let Some(d) = &ch.deps
            && d.iter().any(|d| eq_fold(d, &s.config.name))
        {
            return Err(ERROR_CIRCULAR_DEPENDENCY);
        }
        let account = match &ch.account {
            None => None,
            Some(a) => {
                poly_security::service::account_of(a).ok_or(ERROR_INVALID_SERVICE_ACCOUNT)?;
                Some(if a.is_empty() { w("LocalSystem") } else { a.clone() })
            }
        };
        let name = s.config.name.clone();
        self.check_display_unique(&k, &name, ch.display.as_deref())?;
        let s = self.services.get_mut(&k).expect("checked");
        s.config.ty = ty;
        s.config.start = start;
        s.config.error = error;
        if let Some(p) = ch.path {
            s.config.path = p;
        }
        if let Some(g) = ch.group {
            s.config.group = g;
        }
        if let Some(d) = ch.deps {
            s.config.deps = d;
        }
        if let Some(a) = account {
            s.config.account = a;
        }
        if let Some(d) = ch.display {
            s.config.display = Some(d);
        }
        if !s.builtin && s.phase == Phase::Idle {
            s.status.ty = ty;
        }
        self.dirty.insert(k);
        Ok(())
    }

    /// `ChangeServiceConfig2W`.
    pub fn change2(&mut self, c: &Caller<'_>, h: u64, level: u32, data: Config2) -> Result<(), u32> {
        let (k, granted) = self.service_of(c, h)?;
        if !matches!(level, 1..=8) {
            return Err(ERROR_INVALID_LEVEL);
        }
        if granted & SERVICE_CHANGE_CONFIG == 0 {
            return Err(ERROR_ACCESS_DENIED);
        }
        let s = self.services.get_mut(&k).expect("checked");
        if s.config.delete_flag {
            return Err(ERROR_SERVICE_MARKED_FOR_DELETE);
        }
        match (level, data) {
            (_, Config2::Nothing) => return Ok(()),
            (SERVICE_CONFIG_DESCRIPTION, Config2::Description(d)) => {
                s.config.description = d.filter(|d| !d.is_empty());
            }
            (SERVICE_CONFIG_FAILURE_ACTIONS, Config2::Failure { reset, actions, .. }) => {
                if let Some(a) = actions {
                    if a.iter().any(|&(t, _)| t > SC_ACTION_RUN_COMMAND) {
                        return Err(ERROR_INVALID_PARAMETER);
                    }
                    s.config.failure = Some(FailureActions { reset, reboot_msg: None, command: None, actions: a });
                }
            }
            (SERVICE_CONFIG_DELAYED_AUTO_START_INFO, Config2::Value(_, v)) => {
                s.config.delayed = Some(u32::from(v != 0))
            }
            (SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, Config2::Value(_, v)) => {
                s.config.failure_flag = Some(u32::from(v != 0))
            }
            (SERVICE_CONFIG_SERVICE_SID_INFO, Config2::Value(_, v)) => {
                if !matches!(v, 0 | 1 | 3) {
                    return Err(ERROR_INVALID_PARAMETER);
                }
                s.config.sid_type = Some(v);
            }
            (SERVICE_CONFIG_PRESHUTDOWN_INFO, Config2::Value(_, v)) => s.config.preshutdown = Some(v),
            // Required privileges and triggers are accepted and not kept (not covered).
            (SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO | SERVICE_CONFIG_TRIGGER_INFO, _) => return Ok(()),
            _ => return Err(ERROR_INVALID_PARAMETER),
        }
        self.dirty.insert(k);
        Ok(())
    }

    /// `QueryServiceConfigW`: the configuration, the path expanded with `expand`.
    pub fn query_config(
        &self,
        c: &Caller<'_>,
        h: u64,
        expand: &dyn Fn(&[u16]) -> Option<Vec<u16>>,
    ) -> Result<ConfigReply, u32> {
        let k = self.service_with(c, h, SERVICE_QUERY_CONFIG)?;
        let cfg = &self.services[&k].config;
        Ok(ConfigReply {
            ty: cfg.ty,
            start: cfg.start,
            error: cfg.error,
            path: config::expand(&cfg.path, expand),
            group: cfg.group.clone(),
            tag: 0,
            deps: cfg.deps.clone(),
            account: cfg.account.clone(),
            display: cfg.display_name().to_vec(),
        })
    }

    /// `QueryServiceConfig2W`: the level's data as [`Config2`] (failure actions as given back).
    pub fn query_config2(&self, c: &Caller<'_>, h: u64, level: u32) -> Result<Config2Reply, u32> {
        let (k, granted) = self.service_of(c, h)?;
        if !matches!(level, 1..=8) {
            return Err(ERROR_INVALID_LEVEL);
        }
        if granted & SERVICE_QUERY_CONFIG == 0 {
            return Err(ERROR_ACCESS_DENIED);
        }
        let cfg = &self.services[&k].config;
        Ok(match level {
            SERVICE_CONFIG_DESCRIPTION => Config2Reply::Description(cfg.description.clone()),
            SERVICE_CONFIG_FAILURE_ACTIONS => Config2Reply::Failure(cfg.failure.clone().unwrap_or_default()),
            SERVICE_CONFIG_DELAYED_AUTO_START_INFO => Config2Reply::Value(cfg.delayed.unwrap_or(0)),
            SERVICE_CONFIG_FAILURE_ACTIONS_FLAG => Config2Reply::Value(cfg.failure_flag.unwrap_or(0)),
            SERVICE_CONFIG_SERVICE_SID_INFO => Config2Reply::Value(cfg.sid_type.unwrap_or(0)),
            SERVICE_CONFIG_PRESHUTDOWN_INFO => Config2Reply::Value(cfg.preshutdown.unwrap_or(DEFAULT_PRESHUTDOWN_MS)),
            SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO => Config2Reply::Privileges,
            _ => Config2Reply::Triggers,
        })
    }

    /// `QueryServiceStatus(Ex)`.
    pub fn query_status(&self, c: &Caller<'_>, h: u64) -> Result<Status, u32> {
        let k = self.service_with(c, h, SERVICE_QUERY_STATUS)?;
        Ok(self.services[&k].status)
    }

    /// The status of a service by handle, whatever the handle's rights (`ControlService`'s answer).
    pub fn status_of(&self, c: &Caller<'_>, h: u64) -> Result<Status, u32> {
        let (k, _) = self.service_of(c, h)?;
        Ok(self.services[&k].status)
    }

    /// `StartServiceW`: `waiter` is the event the caller waits on until the start is decided
    /// ([`Scm::start_result`]).
    pub fn start(&mut self, c: &Caller<'_>, h: u64, args: Vec<Vec<u16>>, waiter: u64) -> Result<Vec<Effect>, u32> {
        let k = self.service_with(c, h, SERVICE_START)?;
        let s = &self.services[&k];
        if s.config.start == SERVICE_DISABLED {
            return Err(ERROR_SERVICE_DISABLED);
        }
        if s.builtin || s.status.state != SERVICE_STOPPED || s.phase != Phase::Idle {
            return Err(ERROR_SERVICE_ALREADY_RUNNING);
        }
        let mut order = Vec::new();
        self.dependencies(&k, &mut order, &mut BTreeSet::new())?;
        let mut fx = Vec::new();
        for d in order {
            self.begin(&d, Vec::new(), None, &mut fx);
        }
        self.begin(&k, args, Some(waiter), &mut fx);
        Ok(fx)
    }

    /// Start service `name` for the system itself (ADR 0235): what `StartServiceW` does, without a
    /// handle or a caller's rights, and with nobody waiting on the result. Poly's own programs ask
    /// for this through the kernel when they need one of the system's services and their user
    /// may not start it (on Windows such services run from boot). Already running (or starting)
    /// is not an error; the manager's other errors are the same as `StartServiceW`'s.
    pub fn start_for_system(&mut self, name: &[u16]) -> Result<Vec<Effect>, u32> {
        config::check_name(name)?;
        let k = fold(name);
        let s = self.services.get(&k).ok_or(ERROR_SERVICE_DOES_NOT_EXIST)?;
        if s.config.start == SERVICE_DISABLED {
            return Err(ERROR_SERVICE_DISABLED);
        }
        if s.builtin || s.status.state != SERVICE_STOPPED || s.phase != Phase::Idle {
            return Ok(Vec::new());
        }
        let mut order = Vec::new();
        self.dependencies(&k, &mut order, &mut BTreeSet::new())?;
        let mut fx = Vec::new();
        for d in order {
            self.begin(&d, Vec::new(), None, &mut fx);
        }
        self.begin(&k, Vec::new(), None, &mut fx);
        Ok(fx)
    }

    /// The services `key` depends on that are not running, deepest first; 1075 for one that does
    /// not exist (or is marked for deletion), 1068 for a disabled one.
    fn dependencies(&self, key: &[u16], out: &mut Vec<Vec<u16>>, seen: &mut BTreeSet<Vec<u16>>) -> Result<(), u32> {
        if !seen.insert(key.to_vec()) {
            return Ok(());
        }
        let s = &self.services[key];
        for d in &s.config.deps {
            if d.first() == Some(&u16::from(b'+')) {
                // A load-order group: Poly has none.
                continue;
            }
            let dk = fold(d);
            let Some(ds) = self.services.get(&dk) else { return Err(ERROR_SERVICE_DEPENDENCY_DELETED) };
            if ds.config.delete_flag {
                return Err(ERROR_SERVICE_DEPENDENCY_DELETED);
            }
            if ds.builtin || ds.status.state != SERVICE_STOPPED || ds.phase != Phase::Idle {
                continue;
            }
            if ds.config.start == SERVICE_DISABLED {
                return Err(ERROR_SERVICE_DEPENDENCY_FAIL);
            }
            self.dependencies(&dk, out, seen)?;
            if !out.contains(&dk) {
                out.push(dk);
            }
        }
        Ok(())
    }

    /// Put a stopped service on its way: START_PENDING (checkpoint 0, wait hint 2000), then the
    /// host starts its process once its dependencies run.
    fn begin(&mut self, key: &[u16], args: Vec<Vec<u16>>, starter: Option<u64>, fx: &mut Vec<Effect>) {
        let deps_ready = self.deps_running(key);
        let Some(s) = self.services.get_mut(key) else { return };
        s.status = Status { ty: s.config.ty, state: SERVICE_START_PENDING, hint: START_HINT_MS, ..Status::default() };
        s.args = args;
        if let Some(old) = core::mem::replace(&mut s.starter, starter) {
            fx.push(Effect::Signal(old));
        }
        s.start_result = None;
        s.reported_stopped = false;
        s.timed_out = false;
        s.restart_at = None;
        if deps_ready {
            s.phase = Phase::Spawn;
            fx.push(Effect::WakeHost);
        } else {
            s.phase = Phase::Deps;
        }
    }

    fn deps_running(&self, key: &[u16]) -> bool {
        let Some(s) = self.services.get(key) else { return false };
        s.config
            .deps
            .iter()
            .filter(|d| d.first() != Some(&u16::from(b'+')))
            .all(|d| self.services.get(&fold(d)).is_some_and(|ds| ds.builtin || ds.status.state == SERVICE_RUNNING))
    }

    /// Decide a start: the waiter learns `result` through [`Scm::start_result`].
    fn resolve(&mut self, key: &[u16], result: u32, fx: &mut Vec<Effect>) {
        let Some(s) = self.services.get_mut(key) else { return };
        s.start_result = Some(result);
        if let Some(w) = s.starter.take() {
            fx.push(Effect::Signal(w));
        }
    }

    /// What became of the start the caller waited for (0 for success).
    pub fn start_result(&mut self, c: &Caller<'_>, h: u64) -> Result<(), u32> {
        let (k, _) = self.service_of(c, h)?;
        match self.services.get_mut(&k).expect("checked").start_result.take() {
            Some(0) | None => Ok(()),
            Some(e) => Err(e),
        }
    }

    /// A service whose start failed or never reached RUNNING: those that waited for it fail with
    /// 1068.
    fn fail_dependents(&mut self, key: &[u16], fx: &mut Vec<Effect>) {
        let name = self.services.get(key).map(|s| s.config.name.clone()).unwrap_or_default();
        let waiting: Vec<Vec<u16>> = self
            .services
            .iter()
            .filter(|(_, s)| s.phase == Phase::Deps && s.config.deps.iter().any(|d| eq_fold(d, &name)))
            .map(|(k, _)| k.clone())
            .collect();
        for k in waiting {
            if let Some(s) = self.services.get_mut(&k) {
                s.phase = Phase::Idle;
                s.status.state = SERVICE_STOPPED;
            }
            self.resolve(&k, ERROR_SERVICE_DEPENDENCY_FAIL, fx);
            self.fail_dependents(&k, fx);
        }
    }

    /// Services that waited for their dependencies and can go now.
    fn release_dependents(&mut self, fx: &mut Vec<Effect>) {
        let ready: Vec<Vec<u16>> = self
            .services
            .iter()
            .filter(|(k, s)| s.phase == Phase::Deps && self.deps_running(k))
            .map(|(k, _)| k.clone())
            .collect();
        for k in ready {
            if let Some(s) = self.services.get_mut(&k) {
                s.phase = Phase::Spawn;
                fx.push(Effect::WakeHost);
            }
        }
    }

    // ----- the host (services.exe)

    /// The host's loop: what to start now and how long it may wait (milliseconds, `None`: until
    /// woken). `event` is the host's work event (kept from its first call); `pid` becomes the
    /// host. Starts the auto-start services the first time.
    pub fn host_work(&mut self, c: &Caller<'_>, event: Option<u64>) -> (Vec<Spawn>, Option<u64>, Vec<Effect>) {
        let mut fx = Vec::new();
        match &mut self.host {
            Some((p, e)) if *p == c.pid => {
                if e.is_none() {
                    *e = event;
                }
            }
            _ => self.host = Some((c.pid, event)),
        }
        for s in self.services.values_mut().filter(|s| s.builtin) {
            s.status.pid = c.pid;
        }
        if !self.autostarted {
            self.autostarted = true;
            let auto: Vec<Vec<u16>> = self
                .services
                .iter()
                .filter(|(_, s)| {
                    !s.builtin
                        && s.config.start == SERVICE_AUTO_START
                        && s.config.ty & SERVICE_WIN32 != 0
                        && s.phase == Phase::Idle
                        && s.status.state == SERVICE_STOPPED
                })
                .map(|(k, _)| k.clone())
                .collect();
            for k in auto {
                let mut order = Vec::new();
                if self.dependencies(&k, &mut order, &mut BTreeSet::new()).is_err() {
                    continue;
                }
                for d in order {
                    if self.services[&d].phase == Phase::Idle {
                        self.begin(&d, Vec::new(), None, &mut fx);
                    }
                }
                if self.services[&k].phase == Phase::Idle {
                    self.begin(&k, Vec::new(), None, &mut fx);
                }
            }
        }
        // Restarts that are due (failure actions).
        let due: Vec<Vec<u16>> = self
            .services
            .iter()
            .filter(|(_, s)| s.restart_at.is_some_and(|t| t <= c.now))
            .map(|(k, _)| k.clone())
            .collect();
        for k in due {
            let s = self.services.get_mut(&k).expect("listed");
            s.restart_at = None;
            if s.phase == Phase::Idle && s.status.state == SERVICE_STOPPED && !s.config.delete_flag {
                self.begin(&k, Vec::new(), None, &mut fx);
            }
        }
        // Processes that did not call the dispatcher in time.
        for s in self.services.values_mut() {
            if s.phase == Phase::Connect
                && !s.timed_out
                && s.deadline.is_some_and(|d| d <= c.now)
                && let Some(p) = s.process
            {
                s.timed_out = true;
                fx.push(Effect::Terminate(p));
            }
        }
        let mut spawns = Vec::new();
        for s in self.services.values_mut().filter(|s| s.phase == Phase::Spawn) {
            s.phase = Phase::Spawning;
            spawns.push(Spawn {
                name: s.config.name.clone(),
                command_line: s.config.path.clone(),
                account: s.config.account.clone(),
                service_sid: matches!(s.config.sid_type, Some(1 | 3)),
            });
        }
        let next = self.next_wake(c.now);
        fx.retain(|e| *e != Effect::WakeHost);
        (spawns, next, fx)
    }

    /// How long the host may wait (milliseconds from `now`) before something is due: a start
    /// deadline or a restart. `None`: until it is woken. Asked again after the host started the
    /// processes [`Scm::host_work`] returned (their deadlines begin then).
    pub fn next_wake(&self, now: u64) -> Option<u64> {
        self.services
            .values()
            .filter_map(|s| match s.phase {
                Phase::Connect if !s.timed_out => s.deadline,
                _ => s.restart_at,
            })
            .min()
            .map(|t| t.saturating_sub(now))
    }

    /// The host started (`Ok(pid)`) or could not start (`Err(win32)`) the process of service
    /// `name`.
    pub fn spawned(&mut self, name: &[u16], result: Result<u32, u32>, now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        let k = fold(name);
        let Some(s) = self.services.get_mut(&k) else { return fx };
        if s.phase != Phase::Spawning {
            return fx;
        }
        match result {
            Ok(pid) => {
                s.process = Some(pid);
                s.status.pid = pid;
                s.phase = Phase::Connect;
                s.deadline = Some(now + START_TIMEOUT_MS);
            }
            Err(e) => {
                s.phase = Phase::Idle;
                s.status.state = SERVICE_STOPPED;
                s.status.win32 = 0;
                s.status.pid = 0;
                self.resolve(&k, e, &mut fx);
                self.fail_dependents(&k, &mut fx);
                self.maybe_remove(&k);
            }
        }
        fx
    }

    // ----- the service process's side

    /// `StartServiceCtrlDispatcherW` connects: the service's name and arguments. `event` is the
    /// dispatcher's control event. 1063 for a process the manager did not start, 1056 for a second
    /// attempt.
    pub fn connect(&mut self, c: &Caller<'_>, event: u64) -> Result<Connected, u32> {
        let found = self
            .services
            .iter()
            .find(|(_, s)| s.process == Some(c.pid) && s.phase == Phase::Connect)
            .map(|(k, _)| k.clone());
        let first = self.tried.insert(c.pid);
        let Some(k) = found.filter(|_| first) else {
            return Err(if first { ERROR_FAILED_SERVICE_CONTROLLER_CONNECT } else { ERROR_SERVICE_ALREADY_RUNNING });
        };
        let mut fx = Vec::new();
        let s = self.services.get_mut(&k).expect("found");
        s.phase = Phase::Connected;
        s.deadline = None;
        s.control_event = Some(event);
        let mut argv = alloc::vec![s.config.name.clone()];
        argv.extend(core::mem::take(&mut s.args));
        let name = s.config.name.clone();
        self.resolve(&k, 0, &mut fx);
        Ok((name, argv, fx))
    }

    fn connected(&self, pid: u32) -> Option<Vec<u16>> {
        self.services
            .iter()
            .find(|(_, s)| s.process == Some(pid) && s.phase == Phase::Connected)
            .map(|(k, _)| k.clone())
    }

    /// Whether `pid` runs a connected service (`RegisterServiceCtrlHandler(Ex)W`).
    pub fn is_service_process(&self, pid: u32) -> bool {
        self.connected(pid).is_some()
    }

    /// `SetServiceStatus` of the calling service process (type, state, accepted, Win32 and
    /// specific exit code, checkpoint, wait hint).
    pub fn set_status(&mut self, c: &Caller<'_>, st: Status) -> Result<Vec<Effect>, u32> {
        let k = self.connected(c.pid).ok_or(ERROR_INVALID_HANDLE)?;
        if !(SERVICE_STOPPED..=SERVICE_PAUSED).contains(&st.state) || st.accepted & !SERVICE_ACCEPT_ALL != 0 {
            return Err(ERROR_INVALID_DATA);
        }
        let mut fx = Vec::new();
        let s = self.services.get_mut(&k).expect("found");
        s.status.state = st.state;
        s.status.accepted = st.accepted;
        s.status.win32 = st.win32;
        s.status.specific = st.specific;
        s.status.checkpoint = st.checkpoint;
        s.status.hint = st.hint;
        match st.state {
            SERVICE_RUNNING => self.release_dependents(&mut fx),
            SERVICE_STOPPED => {
                s.status.pid = 0;
                s.reported_stopped = true;
                if let Some(e) = s.control_event {
                    fx.push(Effect::Set(e));
                }
                let failed = st.win32 != 0 && s.config.failure_flag == Some(1);
                self.fail_dependents(&k, &mut fx);
                if failed {
                    self.failure(&k, c.now, &mut fx);
                }
            }
            _ => {}
        }
        Ok(fx)
    }

    /// A failure of service `key` (a crash, or a reported error with the flag on): count it and
    /// run its action (SERVICES.md section 3, item 9).
    fn failure(&mut self, key: &[u16], now: u64, fx: &mut Vec<Effect>) {
        let Some(s) = self.services.get_mut(key) else { return };
        let reset = s.config.failure.as_ref().map_or(0, |f| f.reset);
        if s.failures > 0
            && reset != 0
            && reset != u32::MAX
            && now.saturating_sub(s.last_failure) > u64::from(reset) * 1000
        {
            s.failures = 0;
        }
        s.failures += 1;
        s.last_failure = now;
        let Some(f) = &s.config.failure else { return };
        let Some(&(ty, delay)) = f.actions.get((s.failures as usize - 1).min(f.actions.len().saturating_sub(1))) else {
            return;
        };
        if ty == SC_ACTION_RESTART && !s.config.delete_flag {
            s.restart_at = Some(now + u64::from(delay));
            fx.push(Effect::WakeHost);
        }
    }

    /// `ControlService`: checks the code, the right and the state, then queues the control for the
    /// service's dispatcher; `waiter` is signalled when its handler has returned.
    pub fn control(
        &mut self,
        c: &Caller<'_>,
        h: u64,
        code: u32,
        waiter: u64,
    ) -> Result<(Controlled, Vec<Effect>), u32> {
        let (k, granted) = self.service_of(c, h)?;
        let right = match code {
            SERVICE_CONTROL_STOP => SERVICE_STOP,
            SERVICE_CONTROL_PAUSE | SERVICE_CONTROL_CONTINUE => SERVICE_PAUSE_CONTINUE,
            SERVICE_CONTROL_INTERROGATE => SERVICE_INTERROGATE,
            128..=255 => SERVICE_USER_DEFINED_CONTROL,
            _ => return Err(ERROR_INVALID_PARAMETER),
        };
        if granted & right == 0 {
            return Err(ERROR_ACCESS_DENIED);
        }
        let s = &self.services[&k];
        if s.status.state == SERVICE_STOPPED {
            return Err(ERROR_SERVICE_NOT_ACTIVE);
        }
        if s.status.state == SERVICE_STOP_PENDING {
            return Err(ERROR_SERVICE_CANNOT_ACCEPT_CTRL);
        }
        match code {
            SERVICE_CONTROL_STOP => {
                if s.status.accepted & SERVICE_ACCEPT_STOP == 0 {
                    return Err(ERROR_INVALID_SERVICE_CONTROL);
                }
                let name = &s.config.name;
                let running_dependent = self
                    .services
                    .values()
                    .any(|o| o.status.state != SERVICE_STOPPED && o.config.deps.iter().any(|d| eq_fold(d, name)));
                if running_dependent {
                    return Err(ERROR_DEPENDENT_SERVICES_RUNNING);
                }
            }
            SERVICE_CONTROL_PAUSE | SERVICE_CONTROL_CONTINUE => {
                if s.status.state == SERVICE_START_PENDING {
                    return Err(ERROR_SERVICE_CANNOT_ACCEPT_CTRL);
                }
                if s.status.accepted & SERVICE_ACCEPT_PAUSE_CONTINUE == 0 {
                    return Err(ERROR_INVALID_SERVICE_CONTROL);
                }
            }
            _ => {}
        }
        if s.builtin {
            return Ok((Controlled::Done(s.status), Vec::new()));
        }
        if s.phase != Phase::Connected {
            return Err(ERROR_SERVICE_CANNOT_ACCEPT_CTRL);
        }
        let s = self.services.get_mut(&k).expect("checked");
        s.controls.push_back(Ctl { code, waiter: Some(waiter) });
        let fx = s.control_event.map(Effect::Set).into_iter().collect();
        Ok((Controlled::Pending, fx))
    }

    /// The dispatcher of the calling process asks for its next control.
    pub fn next_control(&mut self, c: &Caller<'_>) -> Next {
        let Some(k) = self.connected(c.pid) else { return Next::End };
        let s = self.services.get_mut(&k).expect("found");
        if s.in_flight.is_some() {
            return Next::Nothing;
        }
        if let Some(ctl) = s.controls.pop_front() {
            s.in_flight = Some(ctl);
            return Next::Control(ctl.code);
        }
        if s.reported_stopped { Next::End } else { Next::Nothing }
    }

    /// The handler returned from the control the dispatcher had.
    pub fn control_done(&mut self, c: &Caller<'_>) -> Vec<Effect> {
        let Some(k) = self.connected(c.pid) else { return Vec::new() };
        let s = self.services.get_mut(&k).expect("found");
        let mut fx: Vec<Effect> = s.in_flight.take().and_then(|x| x.waiter).map(Effect::Signal).into_iter().collect();
        if (!s.controls.is_empty() || s.reported_stopped)
            && let Some(e) = s.control_event
        {
            fx.push(Effect::Set(e));
        }
        fx
    }

    /// A process ended: its handles close, and a service it ran is STOPPED (a crash if it had not
    /// said so: 1067 and the failure actions; before its dispatcher connected: the start fails with
    /// 1053).
    pub fn process_ended(&mut self, pid: u32, now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.tried.remove(&pid);
        if self.host.is_some_and(|(p, _)| p == pid) {
            if let Some((_, Some(e))) = self.host.take() {
                fx.push(Effect::Release(e));
            }
            for s in self.services.values_mut().filter(|s| s.builtin) {
                s.status.pid = 0;
            }
        }
        let owned: Vec<u32> = self.handles.iter().filter(|(_, x)| x.pid == pid).map(|(h, _)| *h).collect();
        let mut touched = BTreeSet::new();
        for h in owned {
            if let Some(ScHandle { kind: Kind::Service(k), .. }) = self.handles.remove(&h) {
                if let Some(s) = self.services.get_mut(&k) {
                    s.handles = s.handles.saturating_sub(1);
                }
                touched.insert(k);
            }
        }
        let ran: Vec<Vec<u16>> =
            self.services.iter().filter(|(_, s)| s.process == Some(pid)).map(|(k, _)| k.clone()).collect();
        for k in &ran {
            let s = self.services.get_mut(k).expect("listed");
            s.process = None;
            s.status.pid = 0;
            for ctl in s.in_flight.take().into_iter().chain(s.controls.drain(..)) {
                if let Some(w) = ctl.waiter {
                    fx.push(Effect::Signal(w));
                }
            }
            if let Some(e) = s.control_event.take() {
                fx.push(Effect::Release(e));
            }
            let phase = core::mem::replace(&mut s.phase, Phase::Idle);
            s.deadline = None;
            match phase {
                Phase::Connect => {
                    s.status.state = SERVICE_STOPPED;
                    s.status.win32 = 0;
                    s.status.accepted = 0;
                    self.resolve(k, ERROR_SERVICE_REQUEST_TIMEOUT, &mut fx);
                    self.fail_dependents(k, &mut fx);
                }
                Phase::Connected if !s.reported_stopped => {
                    s.status = Status {
                        ty: s.config.ty,
                        state: SERVICE_STOPPED,
                        win32: ERROR_PROCESS_ABORTED,
                        ..Status::default()
                    };
                    self.fail_dependents(k, &mut fx);
                    self.failure(k, now, &mut fx);
                }
                _ => {}
            }
            if let Some(s) = self.services.get_mut(k) {
                s.reported_stopped = false;
                if s.status.state != SERVICE_STOPPED {
                    s.status.state = SERVICE_STOPPED;
                    s.status.accepted = 0;
                }
            }
            touched.insert(k.clone());
        }
        for k in touched {
            self.maybe_remove(&k);
        }
        fx
    }

    // ----- listing and security

    /// `EnumServicesStatusExW` (`SC_ENUM_PROCESS_INFO`): name, display name and status of each
    /// service of `ty` in `state`, in the database's order.
    pub fn enumerate(
        &self,
        c: &Caller<'_>,
        mgr: u64,
        ty: u32,
        state: u32,
        group: Option<&[u16]>,
    ) -> Result<Vec<Listed>, u32> {
        let granted = self.manager(c, mgr)?;
        if granted & SC_MANAGER_ENUMERATE_SERVICE == 0 {
            return Err(ERROR_ACCESS_DENIED);
        }
        if ty & SERVICE_TYPE_ALL == 0 || !matches!(state, SERVICE_ACTIVE | SERVICE_INACTIVE | SERVICE_STATE_ALL) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if let Some(g) = group
            && !g.is_empty()
            && !self.services.values().any(|s| eq_fold(&s.config.group, g))
        {
            return Err(ERROR_SERVICE_DOES_NOT_EXIST);
        }
        Ok(self
            .services
            .values()
            .filter(|s| s.config.ty & ty != 0)
            .filter(|s| {
                let active = s.status.state != SERVICE_STOPPED;
                (state & SERVICE_ACTIVE != 0 && active) || (state & SERVICE_INACTIVE != 0 && !active)
            })
            .filter(|s| group.is_none_or(|g| eq_fold(&s.config.group, g)))
            .map(|s| (s.config.name.clone(), s.config.display_name().to_vec(), s.status))
            .collect())
    }

    fn sd_slot(&mut self, c: &Caller<'_>, h: u64) -> Result<(u32, &mut Vec<u8>), u32> {
        let x = self.handle(c, h)?.clone();
        match x.kind {
            Kind::Manager => Ok((x.granted, &mut self.manager_sd)),
            Kind::Service(k) => {
                let s = self.services.get_mut(&k).ok_or(ERROR_INVALID_HANDLE)?;
                Ok((x.granted, &mut s.sd))
            }
        }
    }

    /// `QueryServiceObjectSecurity`: the parts of the descriptor `info` selects, self-relative.
    pub fn query_security(&mut self, c: &Caller<'_>, h: u64, info: u32) -> Result<Vec<u8>, u32> {
        use poly_security::sd::{DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};
        let (granted, sd) = self.sd_slot(c, h)?;
        if info & 0xF == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let need =
            if info & (OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION) != 0 {
                READ_CONTROL
            } else {
                0
            } | if info & poly_security::sd::SACL_SECURITY_INFORMATION != 0 { ACCESS_SYSTEM_SECURITY } else { 0 };
        if granted & need != need {
            return Err(ERROR_ACCESS_DENIED);
        }
        let parts = Parts::from_relative(sd).ok_or(ERROR_INVALID_PARAMETER)?;
        Ok(poly_security::sdops::select(&parts, info).to_relative())
    }

    /// `SetServiceObjectSecurity`.
    pub fn set_security(&mut self, c: &Caller<'_>, h: u64, info: u32, new: &[u8]) -> Result<(), u32> {
        use poly_security::sd::{DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};
        let (granted, sd) = self.sd_slot(c, h)?;
        if info & 0xF == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let need = if info & DACL_SECURITY_INFORMATION != 0 { WRITE_DAC } else { 0 }
            | if info & (OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION) != 0 { WRITE_OWNER } else { 0 }
            | if info & poly_security::sd::SACL_SECURITY_INFORMATION != 0 { ACCESS_SYSTEM_SECURITY } else { 0 };
        if granted & need != need {
            return Err(ERROR_ACCESS_DENIED);
        }
        let stored = Parts::from_relative(sd).ok_or(ERROR_INVALID_PARAMETER)?;
        let new = Parts::from_relative(new).ok_or(ERROR_INVALID_PARAMETER)?;
        let merged = poly_security::sdops::merge(&stored, &new, info).map_err(|_| ERROR_INVALID_PARAMETER)?;
        *sd = merged.to_relative();
        Ok(())
    }
}

/// The reply of `QueryServiceConfig2W` by level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Config2Reply {
    Description(Option<Vec<u16>>),
    Failure(FailureActions),
    Value(u32),
    /// Level 6: no privileges (a NULL list).
    Privileges,
    /// Level 8: no triggers.
    Triggers,
}

impl Config2Reply {
    pub fn write(&self, w: &mut crate::wire::Writer) {
        match self {
            Config2Reply::Description(d) => {
                w.opt(d.as_deref());
            }
            Config2Reply::Failure(f) => f.write(w),
            Config2Reply::Value(v) => {
                w.u32(*v);
            }
            Config2Reply::Privileges | Config2Reply::Triggers => {}
        }
    }
}

/// Remove key `k` and everything below it.
fn remove_tree(reg: &mut Registry, k: KeyId, now: u64) {
    while let Ok(sub) = reg.subkey_at(k, 0) {
        remove_tree(reg, sub, now);
        if reg.key(sub).is_ok_and(|x| !x.is_deleted()) {
            break;
        }
    }
    let _ = reg.delete_key(k, now);
}
