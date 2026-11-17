//! The service control manager in the NT personality (ADR 0221).
//!
//! The manager's state and decisions are `poly_scm::Scm`, one per machine in
//! [`NtSystem::scm`]. Programs reach it through `advapi32`, which sends the messages of
//! `poly_scm::wire` through Poly's own service `PolyServiceCall(Operation, In, InLength, Out,
//! OutLength, ReturnLength*)` ([`service_call`]). This module turns them into calls of the
//! manager with the caller's process, effective token and the time, carries out what the manager
//! asks for (signal an event, start a process, end a process) and writes the database back to the
//! registry (`HKLM\SYSTEM\ControlSet001\Services`).
//!
//! Waits happen in user mode on real event objects: `StartServiceW` and `ControlService` get an
//! event in the caller's table that the manager signals when the start or the control is decided;
//! a service process's dispatcher gets one that is set when a control is queued for it or its
//! service reported STOPPED; `services.exe` (the host) gets one that is set when a process is to
//! be started, and waits on it with the timeout of the next due restart or start deadline.
//!
//! The service processes are started by the host, in the host's own call ([`OP_HOST`]): their
//! creator is `services.exe`, their primary token is the account's ([`Token::service`]: LocalSystem
//! in session 0 at System integrity), their window station `Service-0x0-3e7$` with its `Default`
//! desktop, not visible, and their command line the expanded `ImagePath`.

use crate::abi;
use crate::kernel::NtKernel;
use crate::system::NtSystem;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::{Caller, EventType, Handle, NtStatus, ObjectAttributes, ObjectId, ProcessId};
use poly_scm::consts::*;
use poly_scm::manager::{Change, Config2Reply, Controlled, Create, Next, Spawn};
use poly_scm::wire::{Config2, Reader, Writer, op};
use poly_scm::{Effect, Scm};
use poly_security::service::{Account, account_of, service_sid};
use poly_security::token::Token;

type Result<T> = core::result::Result<T, NtStatus>;

const STATUS_SUCCESS: NtStatus = NtStatus(0);
const STATUS_INVALID_PARAMETER: NtStatus = NtStatus(0xC000_000D);
const STATUS_BUFFER_TOO_SMALL: NtStatus = NtStatus(0xC000_0023);
/// `EVENT_ALL_ACCESS` (`winnt.h`).
const EVENT_ALL_ACCESS: u32 = 0x1F_0003;
/// The largest request accepted.
const MAX_REQUEST: u64 = 256 * 1024;

pub use poly_scm::wire::op::HOST as OP_HOST;

/// The manager and what the NT personality keeps for it: the event objects the manager's ids
/// stand for, and the host's.
#[derive(Debug, Default)]
pub struct ScmState {
    pub scm: Scm,
    events: BTreeMap<u64, ObjectId>,
    next_event: u64,
    /// The host: its process, the manager's id of its event and its handle to it.
    host_handle: Option<(u32, u64, u64)>,
    loaded: bool,
    /// The kernel was asked to start the host and it has not called in yet.
    host_requested: bool,
    /// The process the kernel started as `services.exe`.
    expected_host: Option<u32>,
}

impl ScmState {
    /// Load the database from the registry the first time.
    fn ready(sys: &mut NtSystem, now_ft: u64) {
        sys.scm.loaded = true;
        sys.scm.scm.load(&mut sys.registry.reg, now_ft);
    }
}

/// What carrying out the manager's effects leaves to do outside the state lock.
#[derive(Default)]
struct After {
    terminate: Vec<u32>,
    start_host: bool,
}

/// Make an event in the caller's table that the manager knows as the returned id.
fn new_event(sys: &mut NtSystem, caller: &Caller, kind: EventType) -> Result<(Handle, u64)> {
    let attrs = ObjectAttributes::unnamed(0);
    let created = sys.objects.nt_create_event(caller, &attrs, EVENT_ALL_ACCESS, kind, false)?;
    // The manager's own reference: the event lives until the manager is done with it.
    sys.objects.reference(created.object)?;
    let st = &mut sys.scm;
    st.next_event += 1;
    let id = st.next_event;
    st.events.insert(id, created.object);
    Ok((created.handle, id))
}

/// Forget an event the manager did not keep, and close the caller's handle to it.
fn drop_event(sys: &mut NtSystem, caller: &Caller, h: Handle, id: u64) {
    if let Some(o) = sys.scm.events.remove(&id) {
        let _ = sys.objects.dereference(o);
    }
    let _ = sys.objects.close(caller, h);
}

fn apply(sys: &mut NtSystem, fx: Vec<Effect>, after: &mut After) {
    for e in fx {
        match e {
            Effect::Signal(id) => {
                if let Some(o) = sys.scm.events.remove(&id) {
                    let _ = sys.objects.set_event(o);
                    let _ = sys.objects.dereference(o);
                }
            }
            Effect::Set(id) => {
                if let Some(&o) = sys.scm.events.get(&id) {
                    let _ = sys.objects.set_event(o);
                }
            }
            Effect::Release(id) => {
                if let Some(o) = sys.scm.events.remove(&id) {
                    let _ = sys.objects.dereference(o);
                }
            }
            Effect::WakeHost => match sys.scm.host_handle {
                Some((_, id, _)) => {
                    if let Some(&o) = sys.scm.events.get(&id) {
                        let _ = sys.objects.set_event(o);
                    }
                }
                None => {
                    if !sys.scm.host_requested {
                        sys.scm.host_requested = true;
                        after.start_host = true;
                    }
                }
            },
            Effect::Terminate(pid) => after.terminate.push(pid),
        }
    }
}

fn finish<K: NtKernel>(k: &mut K, after: After) {
    for pid in after.terminate {
        let _ = k.terminate_other_process(ProcessId(pid), NtStatus(ERROR_SERVICE_REQUEST_TIMEOUT));
    }
    if after.start_host {
        k.start_service_host();
    }
}

fn ms<K: NtKernel>(k: &K) -> u64 {
    k.monotonic_100ns() / 10_000
}

/// The environment the system's programs get (to expand `%SystemRoot%` and its kind).
fn system_environment(sys: &NtSystem, processors: u32) -> crate::layout::env::Block {
    crate::layout::env::build(&sys.registry.reg, &crate::layout::POLY_USER, crate::layout::env::Machine { processors })
}

fn lookup(env: &crate::layout::env::Block) -> impl Fn(&[u16]) -> Option<Vec<u16>> + '_ {
    move |name: &[u16]| {
        let n = alloc::string::String::from_utf16_lossy(name);
        env.get(&n).map(|v| v.encode_utf16().collect())
    }
}

/// `PolyServiceCall(Operation, In, InLength, Out, OutLength, ReturnLength*)` (ADR 0221): one
/// message of `poly_scm::wire` to the manager and its reply. A reply that does not fit is
/// `STATUS_BUFFER_TOO_SMALL` with its length in `*ReturnLength`.
pub fn service_call<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [operation, in_at, in_len, out_at, out_len, ret_at] = abi::args::<K, 6>(k, regs)?;
    if in_len > MAX_REQUEST {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let mut input = alloc::vec![0u8; in_len as usize];
    if in_len != 0 {
        abi::read(k, in_at, &mut input)?;
    }
    let mut after = After::default();
    let reply = match operation as u32 {
        op::HOST => host(k, &mut after)?,
        op => {
            let caller = k.caller();
            let now = ms(k);
            let ft = k.system_time();
            let processors = k.processor_count();
            k.with_state(|sys, _, _| call(sys, &caller, now, ft, processors, op, &input, &mut after))
        }
    };
    finish(k, after);
    if ret_at != 0 {
        abi::write_u32(k, ret_at, reply.len() as u32)?;
    }
    if (out_len as usize) < reply.len() {
        return Ok(STATUS_BUFFER_TOO_SMALL);
    }
    abi::write(k, out_at, &reply)?;
    Ok(STATUS_SUCCESS)
}

fn err(code: u32) -> Vec<u8> {
    code.to_le_bytes().to_vec()
}

fn ok(w: Writer) -> Vec<u8> {
    let mut out = 0u32.to_le_bytes().to_vec();
    out.extend_from_slice(&w.0);
    out
}

fn reply(r: core::result::Result<Writer, u32>) -> Vec<u8> {
    match r {
        Ok(w) => ok(w),
        Err(e) => err(e),
    }
}

/// One call of a program (not the host's).
#[allow(clippy::too_many_arguments)]
fn call(
    sys: &mut NtSystem,
    caller: &Caller,
    now: u64,
    ft: u64,
    processors: u32,
    operation: u32,
    input: &[u8],
    after: &mut After,
) -> Vec<u8> {
    ScmState::ready(sys, ft);
    let token: Option<Token> = sys
        .objects
        .security
        .effective(caller.process, caller.thread)
        .and_then(|id| sys.objects.security.token(id))
        .cloned();
    let pid = caller.process.0;
    let c = poly_scm::Caller { pid, token: token.as_ref(), now };
    let mut r = Reader::new(input);
    let bad = || err(ERROR_INVALID_PARAMETER);
    let out = match operation {
        op::OPEN_MANAGER => {
            let Some(access) = r.u32() else { return bad() };
            reply(sys.scm.scm.open_manager(&c, access).map(|h| {
                let mut w = Writer::new();
                w.u64(u64::from(h));
                w
            }))
        }
        op::CREATE => {
            let parsed = (|| {
                let mgr = r.u64()?;
                let req = Create {
                    name: r.opt()?,
                    display: r.opt()?,
                    access: r.u32()?,
                    ty: r.u32()?,
                    start: r.u32()?,
                    error: r.u32()?,
                    path: r.opt()?,
                    group: r.opt()?,
                    deps: r.opt_list()?,
                    account: r.opt()?,
                    wow64: {
                        let _password = r.opt()?;
                        r.u32()? != 0
                    },
                };
                Some((mgr, req))
            })();
            let Some((mgr, req)) = parsed else { return bad() };
            reply(sys.scm.scm.create(&c, mgr, req).map(|h| {
                let mut w = Writer::new();
                w.u64(u64::from(h));
                w
            }))
        }
        op::OPEN_SERVICE => {
            let (Some(mgr), Some(name), Some(access)) = (r.u64(), r.opt(), r.u32()) else { return bad() };
            reply(sys.scm.scm.open_service(&c, mgr, name.as_deref(), access).map(|h| {
                let mut w = Writer::new();
                w.u64(u64::from(h));
                w
            }))
        }
        op::CLOSE => {
            let Some(h) = r.u64() else { return bad() };
            reply(sys.scm.scm.close(&c, h).map(|()| Writer::new()))
        }
        op::DELETE => {
            let Some(h) = r.u64() else { return bad() };
            reply(sys.scm.scm.delete(&c, h).map(|()| Writer::new()))
        }
        op::CHANGE => {
            let parsed = (|| {
                let h = r.u64()?;
                let ch = Change {
                    ty: r.u32()?,
                    start: r.u32()?,
                    error: r.u32()?,
                    path: r.opt()?,
                    group: r.opt()?,
                    deps: r.opt_list()?,
                    account: r.opt()?,
                    display: {
                        let _password = r.opt()?;
                        r.opt()?
                    },
                };
                Some((h, ch))
            })();
            let Some((h, ch)) = parsed else { return bad() };
            reply(sys.scm.scm.change(&c, h, ch).map(|()| Writer::new()))
        }
        op::CHANGE2 => {
            let (Some(h), Some(level)) = (r.u64(), r.u32()) else { return bad() };
            let Some(data) = Config2::read(level, &mut r) else { return bad() };
            reply(sys.scm.scm.change2(&c, h, level, data).map(|()| Writer::new()))
        }
        op::QUERY_CONFIG => {
            let Some(h) = r.u64() else { return bad() };
            let env = system_environment(sys, processors);
            let f = lookup(&env);
            reply(sys.scm.scm.query_config(&c, h, &f).map(|cfg| {
                let mut w = Writer::new();
                cfg.write(&mut w);
                w
            }))
        }
        op::QUERY_CONFIG2 => {
            let (Some(h), Some(level)) = (r.u64(), r.u32()) else { return bad() };
            reply(sys.scm.scm.query_config2(&c, h, level).map(|x: Config2Reply| {
                let mut w = Writer::new();
                x.write(&mut w);
                w
            }))
        }
        op::QUERY_STATUS => {
            let Some(h) = r.u64() else { return bad() };
            reply(sys.scm.scm.query_status(&c, h).map(|s| {
                let mut w = Writer::new();
                w.status(&s);
                w
            }))
        }
        op::START => {
            let (Some(h), Some(args)) = (r.u64(), r.list()) else { return bad() };
            let (eh, id) = match new_event(sys, caller, EventType::Notification) {
                Ok(x) => x,
                Err(_) => return err(ERROR_INVALID_PARAMETER),
            };
            match sys.scm.scm.start(&c, h, args, id) {
                Ok(fx) => {
                    apply(sys, fx, after);
                    let mut w = Writer::new();
                    w.u64(eh.0);
                    ok(w)
                }
                Err(e) => {
                    drop_event(sys, caller, eh, id);
                    err(e)
                }
            }
        }
        op::START_RESULT => {
            let Some(h) = r.u64() else { return bad() };
            reply(sys.scm.scm.start_result(&c, h).map(|()| Writer::new()))
        }
        op::CONTROL => {
            let (Some(h), Some(code)) = (r.u64(), r.u32()) else { return bad() };
            let (eh, id) = match new_event(sys, caller, EventType::Notification) {
                Ok(x) => x,
                Err(_) => return err(ERROR_INVALID_PARAMETER),
            };
            match sys.scm.scm.control(&c, h, code, id) {
                Ok((Controlled::Done(s), fx)) => {
                    drop_event(sys, caller, eh, id);
                    apply(sys, fx, after);
                    let mut w = Writer::new();
                    w.u32(0).status(&s);
                    ok(w)
                }
                Ok((Controlled::Pending, fx)) => {
                    apply(sys, fx, after);
                    let mut w = Writer::new();
                    w.u32(1).u64(eh.0);
                    ok(w)
                }
                Err(e) => {
                    drop_event(sys, caller, eh, id);
                    err(e)
                }
            }
        }
        op::CONTROL_RESULT => {
            let Some(h) = r.u64() else { return bad() };
            reply(sys.scm.scm.status_of(&c, h).map(|s| {
                let mut w = Writer::new();
                w.status(&s);
                w
            }))
        }
        op::ENUM => {
            let (Some(mgr), Some(ty), Some(state), Some(group)) = (r.u64(), r.u32(), r.u32(), r.opt()) else {
                return bad();
            };
            reply(sys.scm.scm.enumerate(&c, mgr, ty, state, group.as_deref()).map(|list| {
                let mut w = Writer::new();
                w.u32(list.len() as u32);
                for (name, display, s) in &list {
                    w.units(name).units(display).status(s);
                }
                w
            }))
        }
        op::QUERY_SECURITY => {
            let (Some(h), Some(info)) = (r.u64(), r.u32()) else { return bad() };
            reply(sys.scm.scm.query_security(&c, h, info).map(|sd| {
                let mut w = Writer::new();
                w.bytes(&sd);
                w
            }))
        }
        op::SET_SECURITY => {
            let (Some(h), Some(info), Some(sd)) = (r.u64(), r.u32(), r.bytes()) else { return bad() };
            reply(sys.scm.scm.set_security(&c, h, info, &sd).map(|()| Writer::new()))
        }
        op::CONNECT => {
            let (eh, id) = match new_event(sys, caller, EventType::Synchronization) {
                Ok(x) => x,
                Err(_) => return err(ERROR_INVALID_PARAMETER),
            };
            match sys.scm.scm.connect(&c, id) {
                Ok((name, argv, fx)) => {
                    apply(sys, fx, after);
                    let mut w = Writer::new();
                    w.units(&name).list(&argv).u64(eh.0);
                    ok(w)
                }
                Err(e) => {
                    drop_event(sys, caller, eh, id);
                    err(e)
                }
            }
        }
        op::NEXT_CONTROL => {
            let mut w = Writer::new();
            match sys.scm.scm.next_control(&c) {
                Next::Nothing => w.u32(0),
                Next::Control(code) => w.u32(1).u32(code).u32(0),
                Next::End => w.u32(2),
            };
            ok(w)
        }
        op::CONTROL_DONE => {
            let fx = sys.scm.scm.control_done(&c);
            apply(sys, fx, after);
            ok(Writer::new())
        }
        op::SET_STATUS => {
            let s = (|| {
                Some(poly_scm::wire::Status {
                    ty: r.u32()?,
                    state: r.u32()?,
                    accepted: r.u32()?,
                    win32: r.u32()?,
                    specific: r.u32()?,
                    checkpoint: r.u32()?,
                    hint: r.u32()?,
                    ..Default::default()
                })
            })();
            let Some(s) = s else { return bad() };
            match sys.scm.scm.set_status(&c, s) {
                Ok(fx) => {
                    apply(sys, fx, after);
                    ok(Writer::new())
                }
                Err(e) => err(e),
            }
        }
        op::IS_SERVICE => {
            let mut w = Writer::new();
            w.u32(u32::from(sys.scm.scm.is_service_process(pid)));
            ok(w)
        }
        _ => err(ERROR_INVALID_PARAMETER),
    };
    sys.scm.scm.sync(&mut sys.registry.reg, ft);
    out
}

/// May the calling process be the host? Only the process the kernel started as
/// `services.exe` ([`expect_host`]), and only while no other host lives.
fn may_host(sys: &NtSystem, caller: &Caller) -> bool {
    let other = sys.scm.scm.host_pid().is_some_and(|p| p != caller.process.0);
    sys.scm.expected_host == Some(caller.process.0) && !other
}

/// The kernel made process `pid` to run `services.exe` (before its first thread runs): it may
/// become the host.
pub fn expect_host(sys: &mut NtSystem, pid: ProcessId) {
    sys.scm.expected_host = Some(pid.0);
}

/// `services.exe`'s turn ([`OP_HOST`]): it becomes the host (LocalSystem, the first time), starts
/// the processes the manager asks for and learns how long it may wait on its event.
fn host<K: NtKernel>(k: &mut K, after: &mut After) -> Result<Vec<u8>> {
    let caller = k.caller();
    let pid = caller.process.0;
    let now = ms(k);
    let ft = k.system_time();
    let host = k.with_state(|sys, _, _| -> Result<Option<(u64, u64)>> {
        ScmState::ready(sys, ft);
        if let Some((p, id, h)) = sys.scm.host_handle
            && p == pid
        {
            return Ok(Some((id, h)));
        }
        if !may_host(sys, &caller) {
            return Ok(None);
        }
        // The host is LocalSystem in session 0 (SERVICES.md section 4).
        let luid = sys.objects.security.new_luid();
        crate::security::install_process_token(
            sys,
            caller.process,
            Token::service(Account::LocalSystem, None, luid),
            caller.thread,
            None,
        )?;
        let (eh, id) = new_event(sys, &caller, EventType::Synchronization)?;
        sys.scm.host_handle = Some((pid, id, eh.0));
        sys.scm.host_requested = false;
        Ok(Some((id, eh.0)))
    })?;
    let Some((event_id, handle)) = host else { return Ok(err(ERROR_ACCESS_DENIED)) };
    let (spawns, _, fx) = k.with_state(|sys, _, _| {
        let c = poly_scm::Caller { pid, token: None, now };
        let r = sys.scm.scm.host_work(&c, Some(event_id));
        sys.scm.scm.sync(&mut sys.registry.reg, ft);
        r
    });
    k.with_state(|sys, _, _| apply(sys, fx, after));
    for s in spawns {
        // The process is made suspended: the manager knows its id before it can connect.
        let made = spawn(k, &s);
        let now = ms(k);
        k.with_state(|sys, _, _| {
            let fx = sys.scm.scm.spawned(&s.name, made.map(|(pid, _)| pid), now);
            apply(sys, fx, after);
            sys.scm.scm.sync(&mut sys.registry.reg, ft);
        });
        if let Ok((_, thread)) = made {
            let _ = k.resume_thread(thread);
        }
    }
    // The processes just started have their deadlines now.
    let now = ms(k);
    let wait = k.with_state(|sys, _, _| sys.scm.scm.next_wake(now));
    let mut w = Writer::new();
    w.u64(handle).u64(wait.unwrap_or(u64::MAX));
    Ok(ok(w))
}

/// Make the process of a service from the host's call, suspended: its id and first thread, or a
/// Win32 error (2 for a program that is not there).
fn spawn<K: NtKernel>(k: &mut K, s: &Spawn) -> core::result::Result<(u32, poly_ntobj::ThreadRef), u32> {
    use crate::io::consts::*;
    let processors = k.processor_count();
    let caller = k.caller();
    let (env, command) = k.with_state(|sys, _, _| {
        let env = system_environment(sys, processors);
        let cmd = poly_scm::config::expand(&s.command_line, &lookup(&env));
        (env.to_utf16(), cmd)
    });
    let image = poly_scm::config::image_of(&command);
    let not_found = |e: NtStatus| match e.0 {
        0xC000_0034 | 0xC000_003A | 0xC000_000F | 0xC000_0033 | 0xC000_0039 | 0xC000_003B => ERROR_FILE_NOT_FOUND,
        0xC000_0022 => ERROR_ACCESS_DENIED,
        0xC000_007B | 0xC000_012F | 0xC000_0130 | 0xC000_0131 => ERROR_BAD_EXE_FORMAT,
        _ => ERROR_FILE_NOT_FOUND,
    };
    let name = crate::threads::image_nt_name(&image).map_err(not_found)?;
    let attrs =
        abi::Attributes { root: None, name: Some(name), attributes: poly_ntobj::OBJ_CASE_INSENSITIVE, security: None };
    let section = crate::io::services::with_named(
        k,
        &attrs,
        FILE_EXECUTE | FILE_READ_DATA | SYNCHRONIZE,
        FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
        |sys, cookie| crate::image::ImageSection::from_reader(sys.io.image_file(cookie)?),
    )
    .map_err(not_found)?;
    let image_text = alloc::string::String::from_utf16_lossy(&image);
    let mut params = crate::params::for_program(&image_text, &[], env);
    params.command_line = command;
    params.current_dir = crate::layout::SYSTEM32.encode_utf16().chain("\\".encode_utf16()).collect();
    let account = account_of(&s.account).unwrap_or(Account::LocalSystem);
    let sid = s.service_sid.then(|| service_sid(&s.name));
    let station: Vec<u16> = account.station_name().encode_utf16().collect();
    let (token_id, token_obj, desktop) = k
        .with_state(|sys, _, _| -> Result<(u64, ObjectId, crate::winsta::Start)> {
            let (id, obj) = sys.objects.new_token(Token::service(account, sid, 0), caller.thread)?;
            sys.objects.assign_security(obj, poly_ntobj::ObjectType::Token, Some(id), None);
            let desktop = sys.winsta.service_start(&mut sys.objects, &station, caller.thread);
            Ok((id, obj, desktop))
        })
        .map_err(|_| ERROR_INVALID_PARAMETER)?;
    let req = crate::kernel::ProcessCreate {
        image: section,
        params,
        process_access: poly_ntobj::access::PROCESS_ALL_ACCESS,
        thread_access: poly_ntobj::access::THREAD_ALL_ACCESS,
        inherit_handles: false,
        suspended: true,
        token: Some(token_id),
        inherit_only: None,
        jobs: Vec::new(),
        join_creator_jobs: false,
        mitigation: poly_kernelbase::mitigation::State::new(),
        desktop,
        error_mode: 0,
        process_security: None,
        thread_security: None,
    };
    let created = k.create_process(req);
    k.with_state(|sys, _, _| {
        let _ = sys.objects.dereference(token_obj);
        if let Ok(c) = &created {
            let _ = sys.objects.close(&caller, c.process);
            let _ = sys.objects.close(&caller, c.thread);
        }
    });
    created.map(|c| (c.client.process.0, c.client.thread)).map_err(|e| match e.0 {
        0xC000_009A => ERROR_INVALID_PARAMETER,
        _ => not_found(e),
    })
}

/// A process ended: the manager closes its handles and stops the service it ran. Returns whether
/// the host has to be started (a restart is due and there is none).
pub fn process_ended(sys: &mut NtSystem, pid: ProcessId, now_ms: u64, now_ft: u64) -> bool {
    if !sys.scm.loaded {
        return false;
    }
    let fx = sys.scm.scm.process_ended(pid.0, now_ms);
    if sys.scm.host_handle.is_some_and(|(p, _, _)| p == pid.0) {
        sys.scm.host_handle = None;
    }
    let mut after = After::default();
    apply(sys, fx, &mut after);
    sys.scm.scm.sync(&mut sys.registry.reg, now_ft);
    after.start_host
}

/// Whether the registry holds services to start at boot (`Start` 2): the kernel then starts the
/// host when the NT personality comes up.
pub fn wants_host_at_boot(sys: &mut NtSystem) -> bool {
    poly_scm::manager::autostart_in_registry(&sys.registry.reg) && sys.scm.host_handle.is_none()
}

#[cfg(test)]
mod tests {
    use crate::services::Service;
    use crate::tests::{Fake, PID};
    use poly_ntobj::status::{STATUS_BUFFER_TOO_SMALL, STATUS_SUCCESS};
    use poly_scm::wire::{Writer, op};

    /// Send `req` as `operation`: the status and the reply's error code.
    fn send(f: &mut Fake, operation: u32, req: &Writer) -> (poly_ntobj::NtStatus, u32, u64) {
        let (inp, out, ret) = (f.alloc(req.0.len().max(8)), f.alloc(256), f.alloc(8));
        f.mem.write_bytes(inp, &req.0).unwrap();
        let in_at = if req.0.is_empty() { 0 } else { inp };
        let st = f.call(Service::PolyServiceCall, &[u64::from(operation), in_at, req.0.len() as u64, out, 256, ret]);
        (st, f.u32_at(out), out)
    }

    #[test]
    fn programs_reach_the_manager_through_one_call() {
        let mut f = Fake::new();
        let mut w = Writer::new();
        w.u32(poly_scm::consts::SC_MANAGER_ALL_ACCESS);
        let (st, e, out) = send(&mut f, op::OPEN_MANAGER, &w);
        assert_eq!((st, e), (STATUS_SUCCESS, 0));
        assert_ne!(f.u64_at(out + 4), 0);
        // A reply that does not fit gives its size.
        let ret = f.alloc(8);
        let st = f.call(Service::PolyServiceCall, &[u64::from(op::OPEN_MANAGER), out, 4, out, 2, ret]);
        assert_eq!((st, f.u32_at(ret)), (STATUS_BUFFER_TOO_SMALL, 12));
    }

    #[test]
    fn only_the_process_the_kernel_started_becomes_the_host() {
        let mut f = Fake::new();
        // An empty request at address 0 is fine (services.exe sends one).
        let (st, e, _) = send(&mut f, op::HOST, &Writer::new());
        assert_eq!((st, e), (STATUS_SUCCESS, poly_scm::consts::ERROR_ACCESS_DENIED));
        super::expect_host(&mut f.sys, PID);
        let (st, e, out) = send(&mut f, op::HOST, &Writer::new());
        assert_eq!((st, e), (STATUS_SUCCESS, 0));
        assert_ne!(f.u64_at(out + 4), 0, "the host's event handle");
        assert_eq!(f.u64_at(out + 12), u64::MAX, "nothing is due");
        // It is LocalSystem now.
        let id = f.sys.objects.security.process_token(PID).unwrap();
        assert_eq!(f.sys.objects.security.token(id).unwrap().user, poly_security::sid::local_system());
    }
}
