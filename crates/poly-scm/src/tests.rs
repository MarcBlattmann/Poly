//! The manager's behaviour, as the visible tests of the service entries walk through it.

use crate::consts::*;
use crate::manager::*;
use crate::wire::{Config2, Status};
use alloc::vec;
use alloc::vec::Vec;
use poly_registry::Registry;
use poly_security::token::Token;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

struct Env {
    scm: Scm,
    reg: Registry,
    token: &'static Token,
    now: u64,
}

const ME: u32 = 100;
const HOST: u32 = 4;

impl Env {
    fn new() -> Env {
        let mut e = Env {
            scm: Scm::new(),
            reg: Registry::new(),
            token: alloc::boxed::Box::leak(alloc::boxed::Box::new(Token::poly_user())),
            now: 1000,
        };
        e.scm.load(&mut e.reg, 1);
        e
    }

    fn c(&self, pid: u32) -> Caller<'static> {
        Caller { pid, token: Some(self.token), now: self.now }
    }

    fn mgr(&mut self) -> u64 {
        let c = Caller { pid: ME, token: Some(self.token), now: self.now };
        u64::from(self.scm.open_manager(&c, SC_MANAGER_ALL_ACCESS).unwrap())
    }

    fn create(&mut self, mgr: u64, name: &str) -> Result<u64, u32> {
        let c = Caller { pid: ME, token: Some(self.token), now: self.now };
        let req = Create {
            name: Some(w(name)),
            access: SERVICE_ALL_ACCESS,
            ty: SERVICE_WIN32_OWN_PROCESS,
            start: SERVICE_DEMAND_START,
            error: 1,
            path: Some(w("\"C:\\t\\svc.exe\" --child run")),
            ..Create::default()
        };
        let r = self.scm.create(&c, mgr, req).map(u64::from);
        self.scm.sync(&mut self.reg, 2);
        r
    }

    fn status(&self, h: u64) -> Status {
        self.scm.query_status(&self.c(ME), h).unwrap()
    }

    /// The host's turn: start every process it is asked for with consecutive pids from `pid`.
    fn host(&mut self, pid: &mut u32) -> Vec<Effect> {
        let c = Caller { pid: HOST, token: Some(self.token), now: self.now };
        let (spawns, _, mut fx) = self.scm.host_work(&c, Some(77));
        for s in spawns {
            *pid += 4;
            fx.extend(self.scm.spawned(&s.name, Ok(*pid), self.now));
        }
        fx
    }

    fn key_exists(&self, name: &str) -> bool {
        self.reg.find(self.reg.machine(), &alloc::format!("{SERVICES_PATH}\\{name}")).is_some()
    }
}

#[test]
fn create_writes_the_registry_and_reads_back_the_defaults() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "PolySpecSvcDef").unwrap();
    let s = e.status(h);
    assert_eq!((s.state, s.win32, s.pid), (SERVICE_STOPPED, ERROR_SERVICE_NEVER_STARTED, 0));
    let cfg = e.scm.query_config(&e.c(ME), h, &|_| None).unwrap();
    assert_eq!(cfg.account, w("LocalSystem"));
    assert_eq!(cfg.display, w("PolySpecSvcDef"));
    assert!(e.key_exists("PolySpecSvcDef"));
    assert_eq!(e.create(m, "polyspecsvcdef"), Err(ERROR_SERVICE_EXISTS));
    // RpcSs is there and running.
    let r = e.scm.open_service(&e.c(ME), m, Some(&w("RPCSS")), SERVICE_QUERY_STATUS).unwrap();
    assert_eq!(e.status(u64::from(r)).state, SERVICE_RUNNING);
}

#[test]
fn handles_belong_to_their_process() {
    let mut e = Env::new();
    let m = e.mgr();
    assert_eq!(e.scm.close(&e.c(ME + 4), m), Err(ERROR_INVALID_HANDLE));
    assert_eq!(e.scm.close(&e.c(ME), 0x1234), Err(ERROR_INVALID_HANDLE));
    assert_eq!(e.scm.close(&e.c(ME), m), Ok(()));
    assert_eq!(e.scm.close(&e.c(ME), m), Err(ERROR_INVALID_HANDLE));
}

#[test]
fn delete_removes_the_key_after_the_last_handle() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "Del").unwrap();
    let h2 = u64::from(e.scm.open_service(&e.c(ME), m, Some(&w("Del")), SERVICE_ALL_ACCESS).unwrap());
    e.scm.delete(&e.c(ME), h).unwrap();
    assert_eq!(e.scm.delete(&e.c(ME), h2), Err(ERROR_SERVICE_MARKED_FOR_DELETE));
    assert_eq!(e.create(m, "Del"), Err(ERROR_SERVICE_MARKED_FOR_DELETE));
    e.scm.close(&e.c(ME), h).unwrap();
    e.scm.sync(&mut e.reg, 3);
    assert!(e.key_exists("Del"));
    e.scm.close(&e.c(ME), h2).unwrap();
    e.scm.sync(&mut e.reg, 3);
    assert!(!e.key_exists("Del"));
}

#[test]
fn a_service_starts_runs_takes_controls_and_stops() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "Run").unwrap();
    let fx = e.scm.start(&e.c(ME), h, vec![w("a")], 500).unwrap();
    assert!(fx.contains(&Effect::WakeHost));
    assert_eq!(e.status(h).state, SERVICE_START_PENDING);
    assert_eq!(e.status(h).hint, 2000);
    let mut pid = 200;
    e.host(&mut pid);
    assert_eq!(e.status(h).pid, 204);
    // The service process connects.
    assert_eq!(e.scm.connect(&e.c(999), 1), Err(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT));
    assert_eq!(e.scm.connect(&e.c(999), 1), Err(ERROR_SERVICE_ALREADY_RUNNING));
    let (name, argv, fx) = e.scm.connect(&e.c(204), 600).unwrap();
    assert_eq!(name, w("Run"));
    assert_eq!(argv, vec![w("Run"), w("a")]);
    assert_eq!(fx, vec![Effect::Signal(500)]);
    assert_eq!(e.scm.start_result(&e.c(ME), h), Ok(()));
    let run = Status { state: SERVICE_RUNNING, accepted: SERVICE_ACCEPT_STOP, ..Status::default() };
    e.scm.set_status(&e.c(204), run).unwrap();
    assert_eq!(e.scm.start(&e.c(ME), h, vec![], 501), Err(ERROR_SERVICE_ALREADY_RUNNING));
    assert_eq!(e.scm.control(&e.c(ME), h, SERVICE_CONTROL_PAUSE, 9).err(), Some(ERROR_INVALID_SERVICE_CONTROL));
    assert_eq!(e.scm.control(&e.c(ME), h, 0, 9).err(), Some(ERROR_INVALID_PARAMETER));
    assert_eq!(e.scm.control(&e.c(ME), h, SERVICE_CONTROL_SHUTDOWN, 9).err(), Some(ERROR_INVALID_PARAMETER));
    let (r, fx) = e.scm.control(&e.c(ME), h, SERVICE_CONTROL_STOP, 700).unwrap();
    assert_eq!(r, Controlled::Pending);
    assert_eq!(fx, vec![Effect::Set(600)]);
    assert_eq!(e.scm.next_control(&e.c(204)), Next::Control(SERVICE_CONTROL_STOP));
    let pending = Status { state: SERVICE_STOP_PENDING, checkpoint: 1, hint: 3000, ..Status::default() };
    e.scm.set_status(&e.c(204), pending).unwrap();
    assert_eq!(e.scm.control_done(&e.c(204)), vec![Effect::Signal(700)]);
    let fx = e.scm.set_status(&e.c(204), Status { state: SERVICE_STOPPED, ..Status::default() }).unwrap();
    assert_eq!(fx, vec![Effect::Set(600)]);
    let s = e.status(h);
    assert_eq!((s.state, s.win32, s.pid), (SERVICE_STOPPED, 0, 0));
    assert_eq!(e.scm.next_control(&e.c(204)), Next::End);
    e.scm.process_ended(204, e.now);
    assert_eq!(e.status(h).win32, 0);
    assert_eq!(e.scm.control(&e.c(ME), h, SERVICE_CONTROL_STOP, 9).err(), Some(ERROR_SERVICE_NOT_ACTIVE));
}

#[test]
fn a_crash_is_1067_and_restarts_by_the_failure_actions() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "Crash").unwrap();
    let acts =
        Config2::Failure { reset: 86400, reboot_msg: None, command: None, actions: Some(vec![(1, 500), (0, 0)]) };
    e.scm.change2(&e.c(ME), h, SERVICE_CONFIG_FAILURE_ACTIONS, acts).unwrap();
    e.scm.start(&e.c(ME), h, vec![], 1).unwrap();
    let mut pid = 300;
    e.host(&mut pid);
    e.scm.connect(&e.c(304), 2).unwrap();
    e.scm.set_status(&e.c(304), Status { state: SERVICE_RUNNING, accepted: 1, ..Status::default() }).unwrap();
    let fx = e.scm.process_ended(304, e.now);
    assert!(fx.contains(&Effect::WakeHost));
    assert_eq!(e.status(h).win32, ERROR_PROCESS_ABORTED);
    // Not before the delay.
    e.now += 100;
    let c = Caller { pid: HOST, token: None, now: e.now };
    let (spawns, wait, _) = e.scm.host_work(&c, None);
    assert!(spawns.is_empty());
    assert_eq!(wait, Some(400));
    e.now += 400;
    e.host(&mut pid);
    assert_eq!(e.status(h).state, SERVICE_START_PENDING);
    assert_eq!(e.status(h).pid, 308);
    e.scm.connect(&e.c(308), 3).unwrap();
    e.scm.set_status(&e.c(308), Status { state: SERVICE_RUNNING, accepted: 1, ..Status::default() }).unwrap();
    // The second failure takes the second action: none.
    let fx = e.scm.process_ended(308, e.now);
    assert!(!fx.contains(&Effect::WakeHost));
}

#[test]
fn a_reported_error_restarts_only_with_the_flag() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "Exit").unwrap();
    let acts = Config2::Failure { reset: 3600, reboot_msg: None, command: None, actions: Some(vec![(1, 300); 3]) };
    e.scm.change2(&e.c(ME), h, SERVICE_CONFIG_FAILURE_ACTIONS, acts).unwrap();
    let mut pid = 400;
    let run_once = |e: &mut Env, pid: &mut u32| -> Vec<Effect> {
        e.host(pid);
        e.scm.connect(&e.c(*pid), 2).unwrap();
        e.scm.set_status(&e.c(*pid), Status { state: SERVICE_STOPPED, win32: 5, ..Status::default() }).unwrap()
    };
    e.scm.start(&e.c(ME), h, vec![], 1).unwrap();
    assert!(!run_once(&mut e, &mut pid).contains(&Effect::WakeHost));
    e.scm.process_ended(pid, e.now);
    assert_eq!(e.status(h).win32, 5);
    e.scm.change2(&e.c(ME), h, SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, Config2::Value(4, 1)).unwrap();
    e.scm.start(&e.c(ME), h, vec![], 1).unwrap();
    assert!(run_once(&mut e, &mut pid).contains(&Effect::WakeHost));
}

#[test]
fn a_process_that_ends_before_the_dispatcher_fails_the_start_with_1053() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "Early").unwrap();
    e.scm.start(&e.c(ME), h, vec![], 11).unwrap();
    let mut pid = 500;
    e.host(&mut pid);
    let fx = e.scm.process_ended(504, e.now);
    assert_eq!(fx, vec![Effect::Signal(11)]);
    assert_eq!(e.scm.start_result(&e.c(ME), h), Err(ERROR_SERVICE_REQUEST_TIMEOUT));
    let s = e.status(h);
    assert_eq!((s.state, s.win32, s.hint), (SERVICE_STOPPED, 0, 2000));
    // A program that cannot be started: 2.
    e.scm.start(&e.c(ME), h, vec![], 12).unwrap();
    let c = Caller { pid: HOST, token: None, now: e.now };
    let (spawns, _, _) = e.scm.host_work(&c, None);
    let fx = e.scm.spawned(&spawns[0].name, Err(ERROR_FILE_NOT_FOUND), e.now);
    assert_eq!(fx, vec![Effect::Signal(12)]);
    assert_eq!(e.scm.start_result(&e.c(ME), h), Err(ERROR_FILE_NOT_FOUND));
}

#[test]
fn the_30_second_rule() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "Slow").unwrap();
    e.scm.start(&e.c(ME), h, vec![], 11).unwrap();
    let mut pid = 600;
    e.host(&mut pid);
    // Once its process is made, the host waits no longer than the start deadline.
    assert_eq!(e.scm.next_wake(e.now), Some(START_TIMEOUT_MS));
    e.now += START_TIMEOUT_MS;
    let fx = e.host(&mut pid);
    assert_eq!(fx, vec![Effect::Terminate(604)]);
}

#[test]
fn dependencies_start_first_and_missing_or_disabled_ones_fail() {
    let mut e = Env::new();
    let m = e.mgr();
    let a = e.create(m, "DepA").unwrap();
    let b = e.create(m, "DepB").unwrap();
    let ch = Change {
        ty: SERVICE_NO_CHANGE,
        start: SERVICE_NO_CHANGE,
        error: SERVICE_NO_CHANGE,
        deps: Some(vec![w("DepA")]),
        ..Change::default()
    };
    e.scm.change(&e.c(ME), b, ch).unwrap();
    e.scm.start(&e.c(ME), b, vec![], 1).unwrap();
    assert_eq!(e.status(a).state, SERVICE_START_PENDING);
    let mut pid = 700;
    e.host(&mut pid);
    // Only A goes first.
    assert_eq!(e.status(a).pid, 704);
    assert_eq!(e.status(b).pid, 0);
    e.scm.connect(&e.c(704), 2).unwrap();
    let fx = e.scm.set_status(&e.c(704), Status { state: SERVICE_RUNNING, accepted: 1, ..Status::default() }).unwrap();
    assert!(fx.contains(&Effect::WakeHost));
    e.host(&mut pid);
    assert_eq!(e.status(b).pid, 708);
    e.scm.connect(&e.c(708), 3).unwrap();
    e.scm.set_status(&e.c(708), Status { state: SERVICE_RUNNING, accepted: 1, ..Status::default() }).unwrap();
    assert_eq!(e.scm.control(&e.c(ME), a, SERVICE_CONTROL_STOP, 5).err(), Some(ERROR_DEPENDENT_SERVICES_RUNNING));
    let c = e.create(m, "DepC").unwrap();
    let ch = Change {
        ty: SERVICE_NO_CHANGE,
        start: SERVICE_NO_CHANGE,
        error: SERVICE_NO_CHANGE,
        deps: Some(vec![w("Nope")]),
        ..Change::default()
    };
    e.scm.change(&e.c(ME), c, ch).unwrap();
    assert_eq!(e.scm.start(&e.c(ME), c, vec![], 1).err(), Some(ERROR_SERVICE_DEPENDENCY_DELETED));
    // Google's services depend on RPCSS: it is running.
    let g = e.create(m, "DepG").unwrap();
    let ch = Change {
        ty: SERVICE_NO_CHANGE,
        start: SERVICE_NO_CHANGE,
        error: SERVICE_NO_CHANGE,
        deps: Some(vec![w("RPCSS")]),
        ..Change::default()
    };
    e.scm.change(&e.c(ME), g, ch).unwrap();
    let fx = e.scm.start(&e.c(ME), g, vec![], 1).unwrap();
    assert!(fx.contains(&Effect::WakeHost));
}

#[test]
fn creation_errors() {
    let mut e = Env::new();
    let m = e.mgr();
    let c = e.c(ME);
    let base = || Create {
        name: Some(w("E")),
        access: SERVICE_ALL_ACCESS,
        ty: SERVICE_WIN32_OWN_PROCESS,
        start: SERVICE_DEMAND_START,
        error: 1,
        path: Some(w("x")),
        ..Create::default()
    };
    let mut e2 = Env::new();
    let m2 = e2.mgr();
    let _ = (&mut e2, m2);
    let cases: Vec<(Create, u32)> = vec![
        (Create { name: None, ..base() }, ERROR_INVALID_ADDRESS),
        (Create { path: None, ..base() }, ERROR_INVALID_ADDRESS),
        (Create { path: Some(vec![]), ..base() }, ERROR_INVALID_PARAMETER),
        (Create { ty: 0x30, ..base() }, ERROR_INVALID_PARAMETER),
        (Create { start: 0, ..base() }, ERROR_INVALID_PARAMETER),
        (Create { error: 4, ..base() }, ERROR_INVALID_PARAMETER),
        (Create { name: Some(w("a/b")), ..base() }, ERROR_INVALID_NAME),
        (Create { deps: Some(vec![w("e")]), ..base() }, ERROR_CIRCULAR_DEPENDENCY),
        (Create { account: Some(w("Nobody")), ..base() }, ERROR_INVALID_SERVICE_ACCOUNT),
        (Create { display: Some(w("rpcss")), ..base() }, ERROR_DUPLICATE_SERVICE_NAME),
        (Create { ty: SERVICE_KERNEL_DRIVER, start: 0, ..base() }, ERROR_INVALID_PARAMETER),
    ];
    let mut scm = core::mem::take(&mut e.scm);
    for (req, want) in cases {
        assert_eq!(scm.create(&c, m, req), Err(want));
    }
}

#[test]
fn a_restricted_caller_cannot_create_or_start() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "Sec").unwrap();
    let admins = poly_security::sid::builtin(544);
    let restricted = e.token.filter(0x1, &[admins], &[], &[], 0x55).unwrap();
    let c = Caller { pid: ME, token: Some(&restricted), now: 0 };
    assert_eq!(e.scm.open_manager(&c, SC_MANAGER_ALL_ACCESS), Err(ERROR_ACCESS_DENIED));
    let cm = u64::from(e.scm.open_manager(&c, SC_MANAGER_CONNECT).unwrap());
    assert_eq!(e.scm.open_service(&c, cm, Some(&w("Sec")), SERVICE_START), Err(ERROR_ACCESS_DENIED));
    // Grant Everyone QUERY_STATUS (the sheet of SetServiceObjectSecurity).
    let sd = poly_security::sdstr::parse(&w(
        "D:(A;;LC;;;WD)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWRPWPDTLOCRRC;;;SY)",
    ))
    .unwrap();
    e.scm.set_security(&e.c(ME), h, poly_security::sd::DACL_SECURITY_INFORMATION, &sd).unwrap();
    assert!(e.scm.open_service(&c, cm, Some(&w("Sec")), SERVICE_QUERY_STATUS).is_ok());
    assert_eq!(e.scm.open_service(&c, cm, Some(&w("Sec")), SERVICE_START), Err(ERROR_ACCESS_DENIED));
}

#[test]
fn enumeration_filters() {
    let mut e = Env::new();
    let m = e.mgr();
    e.create(m, "EnA").unwrap();
    let all = e.scm.enumerate(&e.c(ME), m, SERVICE_WIN32, SERVICE_STATE_ALL, None).unwrap();
    assert!(all.iter().any(|(n, _, _)| *n == w("EnA")));
    let active = e.scm.enumerate(&e.c(ME), m, SERVICE_WIN32, SERVICE_ACTIVE, None).unwrap();
    assert!(active.iter().all(|(n, _, _)| *n == w("RpcSs")));
    assert_eq!(e.scm.enumerate(&e.c(ME), m, 0, 3, None).err(), Some(ERROR_INVALID_PARAMETER));
    assert_eq!(e.scm.enumerate(&e.c(ME), m, SERVICE_WIN32, 4, None).err(), Some(ERROR_INVALID_PARAMETER));
    assert_eq!(
        e.scm.enumerate(&e.c(ME), m, SERVICE_WIN32, 3, Some(&w("NoGroup"))).err(),
        Some(ERROR_SERVICE_DOES_NOT_EXIST)
    );
}

#[test]
fn the_database_survives_a_reload_and_marked_services_go() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "Keep").unwrap();
    e.scm.change2(&e.c(ME), h, 1, Config2::Description(Some(w("d")))).unwrap();
    let gone = e.create(m, "Gone").unwrap();
    e.scm.delete(&e.c(ME), gone).unwrap();
    e.scm.sync(&mut e.reg, 5);
    let mut again = Scm::new();
    again.load(&mut e.reg, 6);
    assert_eq!(again.service(&w("keep")).unwrap().config.description, Some(w("d")));
    assert!(again.service(&w("Gone")).is_none());
    assert!(!e.key_exists("Gone"));
}

#[test]
fn the_system_starts_a_service_its_caller_may_not() {
    let mut e = Env::new();
    let m = e.mgr();
    let h = e.create(m, "SysOnly").unwrap();
    // A standard user cannot start it (the default descriptor gives interactive users no start).
    let admins = poly_security::sid::builtin(544);
    let standard = e.token.filter(0, &[admins], &[], &[], 0x56).unwrap();
    let c = Caller { pid: ME, token: Some(&standard), now: 0 };
    let cm = u64::from(e.scm.open_manager(&c, SC_MANAGER_CONNECT).unwrap());
    assert_eq!(e.scm.open_service(&c, cm, Some(&w("SysOnly")), SERVICE_START), Err(ERROR_ACCESS_DENIED));
    // The system can: the host is woken to start the process, nobody waits on an event.
    let fx = e.scm.start_for_system(&w("SysOnly")).unwrap();
    assert!(fx.contains(&Effect::WakeHost));
    assert_eq!(e.status(h).state, SERVICE_START_PENDING);
    // Asking again while it starts is no error and changes nothing.
    assert_eq!(e.scm.start_for_system(&w("sysonly")), Ok(Vec::new()));
    assert_eq!(e.scm.start_for_system(&w("NoSuch")), Err(ERROR_SERVICE_DOES_NOT_EXIST));
}
