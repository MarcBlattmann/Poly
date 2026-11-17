//! COM activation across processes (ADR 0220): the machine-wide table the activation process
//! `comlaunch.exe` keeps, and what a class's registry entry says about starting its server.
//!
//! [`Activator`] decides; the exe does the I/O (pipes, `CreateProcessW`, the service control
//! manager) and waits. Rules from the sheet `CoCreateInstanceEx` (and `CoRegisterClassObject`):
//! a live registration is served (a single-use one once); a request for a class whose server is
//! being started with the same plan waits behind that start (sheet `CoRegisterClassObject`:
//! `server_that_never_registers`); with another plan (the class's registry entry changed: a
//! missing file, sheet `missing_server_file`) it starts its own; otherwise the server is started
//! and the request waits for its registration, however long (Poly gives up after
//! [`LAUNCH_TIMEOUT_MS`]).

use crate::guid::Guid;
use crate::wire::{ObjRef, Plan};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// How long a started server has to register before its waiters get `CO_E_SERVER_EXEC_FAILURE`
/// (Poly's choice; the sheet only says more than 6 s).
pub const LAUNCH_TIMEOUT_MS: u64 = 120_000;

/// One published class object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    pub clsid: Guid,
    pub pid: u32,
    pub cookie: u32,
    pub factory: ObjRef,
    pub single: bool,
}

/// What to do with an activation request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Hand out this registration's factory.
    Serve(Published),
    /// A server for the class is being started: ask again later.
    Wait,
    /// Start the server (the plan given), then ask again until it registered.
    Launch,
    /// Nothing is registered and nothing can be started: `REGDB_E_CLASSNOTREG`.
    NotRegistered,
    /// The server started for the class did not register in time: `CO_E_SERVER_EXEC_FAILURE`.
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    clsid: Guid,
    plan: Plan,
    since: u64,
}

/// The machine-wide state.
#[derive(Debug, Default)]
pub struct Activator {
    published: Vec<Published>,
    pending: Vec<Pending>,
}

impl Activator {
    pub const fn new() -> Activator {
        Activator { published: Vec::new(), pending: Vec::new() }
    }

    /// A request for `clsid` at time `now` (ms), with the plan its client read (if any).
    /// `started` is when this request itself started a launch (it waits for it like the others).
    pub fn activate(&mut self, clsid: &Guid, plan: Option<&Plan>, now: u64, started: Option<u64>) -> Decision {
        if let Some(i) = self.published.iter().position(|p| p.clsid == *clsid) {
            let p = if self.published[i].single { self.published.remove(i) } else { self.published[i].clone() };
            return Decision::Serve(p);
        }
        let behind = |p: &Pending| p.clsid == *clsid && plan.is_none_or(|x| *x == p.plan);
        if let Some(p) = self.pending.iter().position(behind) {
            if now.saturating_sub(self.pending[p].since) >= LAUNCH_TIMEOUT_MS {
                self.pending.remove(p);
                return Decision::TimedOut;
            }
            return Decision::Wait;
        }
        if started.is_some() {
            // Our own launch timed out and was cleared by another waiter.
            return Decision::TimedOut;
        }
        if let Some(plan) = plan {
            self.pending.push(Pending { clsid: *clsid, plan: plan.clone(), since: now });
            return Decision::Launch;
        }
        Decision::NotRegistered
    }

    /// The launch of `plan` for `clsid` failed at once (the program or the service could not start).
    pub fn launch_failed(&mut self, clsid: &Guid, plan: &Plan) {
        self.pending.retain(|p| !(p.clsid == *clsid && p.plan == *plan));
    }

    /// A server published a class object; the class's pending launch is over.
    pub fn register(&mut self, p: Published) {
        self.pending.retain(|x| x.clsid != p.clsid);
        self.published.retain(|x| !(x.pid == p.pid && x.cookie == p.cookie));
        self.published.push(p);
    }

    /// A server withdrew one.
    pub fn revoke(&mut self, pid: u32, cookie: u32) {
        self.published.retain(|x| !(x.pid == pid && x.cookie == cookie));
    }

    /// A process ended: its registrations go.
    pub fn process_gone(&mut self, pid: u32) {
        self.published.retain(|x| x.pid != pid);
    }

    /// The processes that have registrations (to check whether they still run).
    pub fn pids(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.published.iter().map(|p| p.pid).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// Where a class's registry keys are read from (the client's `ole32` implements it with the
/// registry functions; tests with a map).
pub trait ClassRegistry {
    /// The value `name` (`None`: the default value) of the key `path` under `hive`, as text.
    fn value(&self, hive: Hive, path: &str, name: Option<&str>) -> Option<String>;
    /// Whether the key exists.
    fn exists(&self, hive: Hive, path: &str) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

/// What the registry says about a class.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassEntry {
    /// `InprocServer32`'s default value.
    pub inproc_server: Option<String>,
    /// `InprocHandler32`'s default value.
    pub inproc_handler: Option<String>,
    /// `LocalServer32`'s default value.
    pub local_server: Option<String>,
    /// The AppID's `LocalService` and `ServiceParameters`.
    pub local_service: Option<String>,
    pub service_parameters: Option<String>,
}

/// The class keys in the order COM reads them (sheet: `HKCU\Software\Classes` wins over the
/// machine's; a class only in the 32-bit view is found by a 64-bit process too).
const CLASS_ROOTS: [(Hive, &str); 3] = [
    (Hive::CurrentUser, r"Software\Classes"),
    (Hive::LocalMachine, r"SOFTWARE\Classes"),
    (Hive::LocalMachine, r"SOFTWARE\Classes\WOW6432Node"),
];

/// Read the entry of the class `clsid` (`{...}` text): from the first root that has its key.
pub fn find_class(reg: &dyn ClassRegistry, clsid: &str) -> Option<ClassEntry> {
    for (hive, root) in CLASS_ROOTS {
        let key = alloc::format!(r"{root}\CLSID\{clsid}");
        if !reg.exists(hive, &key) {
            continue;
        }
        let sub = |s: &str| reg.value(hive, &alloc::format!(r"{key}\{s}"), None).filter(|v| !v.is_empty());
        let mut e = ClassEntry {
            inproc_server: sub("InprocServer32"),
            inproc_handler: sub("InprocHandler32"),
            local_server: sub("LocalServer32"),
            ..ClassEntry::default()
        };
        if let Some(appid) = reg.value(hive, &key, Some("AppID")) {
            // The AppID key in the same hive, else the machine's.
            let keys = [
                (hive, alloc::format!(r"{root}\AppID\{appid}")),
                (Hive::LocalMachine, alloc::format!(r"SOFTWARE\Classes\AppID\{appid}")),
            ];
            for (h, a) in keys {
                if reg.exists(h, &a) {
                    e.local_service = reg.value(h, &a, Some("LocalService")).filter(|v| !v.is_empty());
                    e.service_parameters = reg.value(h, &a, Some("ServiceParameters"));
                    break;
                }
            }
        }
        return Some(e);
    }
    None
}

/// The plan for starting the local server of `entry`: the AppID's service wins over
/// `LocalServer32` (sheet `CoCreateInstanceEx`, `service_over_server`).
pub fn local_plan(entry: &ClassEntry) -> Option<Plan> {
    if let Some(name) = &entry.local_service {
        let args = entry.service_parameters.as_deref().map(split_service_parameters).unwrap_or_default();
        return Some(Plan::Service { name: name.clone(), args });
    }
    entry.local_server.as_ref().map(|c| Plan::Command(server_command_line(c)))
}

/// `ServiceParameters` as the service's arguments: split at spaces (sheet: `--one --two=2 three`
/// gives three arguments).
pub fn split_service_parameters(s: &str) -> Vec<String> {
    s.split(' ').filter(|w| !w.is_empty()).map(ToString::to_string).collect()
}

/// The command line a local server is started with: `LocalServer32` and `-Embedding` (Learn,
/// "Out-of-Process Server Implementation").
pub fn server_command_line(local_server: &str) -> String {
    alloc::format!("{} -Embedding", local_server.trim_end())
}

/// The program of a command line: the quoted first word, or up to the first space.
pub fn command_program(cmd: &str) -> &str {
    let cmd = cmd.trim_start();
    if let Some(rest) = cmd.strip_prefix('"') {
        return rest.split('"').next().unwrap_or("");
    }
    cmd.split(' ').next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;
    use alloc::vec;

    const G: Guid = Guid { data1: 1, data2: 2, data3: 3, data4: [4; 8] };
    const H: Guid = Guid { data1: 9, data2: 2, data3: 3, data4: [4; 8] };

    fn publ(clsid: Guid, pid: u32, cookie: u32, single: bool) -> Published {
        Published { clsid, pid, cookie, factory: ObjRef { endpoint: "p".to_string(), oid: 1 }, single }
    }

    fn cmd(c: &str) -> Plan {
        Plan::Command(c.to_string())
    }

    #[test]
    fn launch_wait_serve() {
        let mut a = Activator::new();
        let p = cmd("s.exe");
        assert_eq!(a.activate(&G, None, 0, None), Decision::NotRegistered);
        assert_eq!(a.activate(&G, Some(&p), 0, None), Decision::Launch);
        // Another request with the same plan waits behind the start; one without a plan too.
        assert_eq!(a.activate(&G, Some(&p), 10, None), Decision::Wait);
        assert_eq!(a.activate(&G, None, 10, None), Decision::Wait);
        assert_eq!(a.activate(&G, Some(&p), 10, Some(0)), Decision::Wait);
        a.register(publ(G, 5, 1, false));
        assert!(matches!(a.activate(&G, Some(&p), 20, Some(0)), Decision::Serve(_)));
        assert!(matches!(a.activate(&G, None, 20, None), Decision::Serve(_)));
        a.revoke(5, 1);
        assert_eq!(a.activate(&G, None, 30, None), Decision::NotRegistered);
    }

    #[test]
    fn another_plan_starts_its_own_launch() {
        let mut a = Activator::new();
        let (p, q) = (cmd("never-registers.exe"), cmd("missing.exe"));
        assert_eq!(a.activate(&G, Some(&p), 0, None), Decision::Launch);
        assert_eq!(a.activate(&G, Some(&q), 10, None), Decision::Launch);
        a.launch_failed(&G, &q);
        // The first launch still waits.
        assert_eq!(a.activate(&G, Some(&p), 20, Some(0)), Decision::Wait);
    }

    #[test]
    fn single_use_is_served_once() {
        let mut a = Activator::new();
        let p = cmd("s.exe");
        a.register(publ(G, 5, 1, true));
        assert!(matches!(a.activate(&G, Some(&p), 0, None), Decision::Serve(_)));
        assert_eq!(a.activate(&G, Some(&p), 0, None), Decision::Launch);
    }

    #[test]
    fn a_launch_that_never_registers_times_out() {
        let mut a = Activator::new();
        let p = cmd("s.exe");
        assert_eq!(a.activate(&G, Some(&p), 0, None), Decision::Launch);
        assert_eq!(a.activate(&G, Some(&p), 6_000, Some(0)), Decision::Wait);
        assert_eq!(a.activate(&G, Some(&p), LAUNCH_TIMEOUT_MS, Some(0)), Decision::TimedOut);
        assert_eq!(a.activate(&G, Some(&p), LAUNCH_TIMEOUT_MS + 1, Some(0)), Decision::TimedOut);
        assert_eq!(a.activate(&G, Some(&p), LAUNCH_TIMEOUT_MS + 1, None), Decision::Launch);
        a.launch_failed(&G, &p);
        assert_eq!(a.activate(&G, None, 0, None), Decision::NotRegistered);
    }

    #[test]
    fn processes_that_end_lose_their_registrations() {
        let mut a = Activator::new();
        a.register(publ(G, 5, 1, false));
        a.register(publ(H, 6, 1, false));
        assert_eq!(a.pids(), [5, 6]);
        a.process_gone(5);
        assert_eq!(a.activate(&G, None, 0, None), Decision::NotRegistered);
        assert!(matches!(a.activate(&H, None, 0, None), Decision::Serve(_)));
    }

    struct Map(BTreeMap<(u8, String, String), String>);

    impl Map {
        fn put(&mut self, hive: Hive, path: &str, name: &str, value: &str) {
            self.0.insert((hive as u8, path.to_string(), name.to_string()), value.to_string());
        }
    }

    impl ClassRegistry for Map {
        fn value(&self, hive: Hive, path: &str, name: Option<&str>) -> Option<String> {
            self.0.get(&(hive as u8, path.to_string(), name.unwrap_or("").to_string())).cloned()
        }
        fn exists(&self, hive: Hive, path: &str) -> bool {
            self.0.keys().any(|(h, p, _)| *h == hive as u8 && (p == path || p.starts_with(&alloc::format!("{path}\\"))))
        }
    }

    const C: &str = "{B2A0E301-0000-4000-8000-706F6C797773}";

    #[test]
    fn hives_and_plans() {
        let mut m = Map(BTreeMap::new());
        assert_eq!(find_class(&m, C), None);
        m.put(Hive::LocalMachine, &alloc::format!(r"SOFTWARE\Classes\WOW6432Node\CLSID\{C}\LocalServer32"), "", "w.exe");
        assert_eq!(local_plan(&find_class(&m, C).unwrap()), Some(Plan::Command("w.exe -Embedding".to_string())));
        m.put(Hive::LocalMachine, &alloc::format!(r"SOFTWARE\Classes\CLSID\{C}\LocalServer32"), "", "\"m.exe\" a");
        assert_eq!(
            local_plan(&find_class(&m, C).unwrap()),
            Some(Plan::Command("\"m.exe\" a -Embedding".to_string()))
        );
        m.put(Hive::CurrentUser, &alloc::format!(r"Software\Classes\CLSID\{C}\LocalServer32"), "", "u.exe");
        assert_eq!(local_plan(&find_class(&m, C).unwrap()), Some(Plan::Command("u.exe -Embedding".to_string())));
    }

    #[test]
    fn appid_local_service() {
        let mut m = Map(BTreeMap::new());
        let k = alloc::format!(r"SOFTWARE\Classes\CLSID\{C}");
        m.put(Hive::LocalMachine, &k, "", "name");
        m.put(Hive::LocalMachine, &alloc::format!(r"{k}\LocalServer32"), "", "s.exe");
        m.put(Hive::LocalMachine, &k, "AppID", "{A}");
        // An AppID value with no key, or a key with no values: the command.
        assert_eq!(local_plan(&find_class(&m, C).unwrap()), Some(Plan::Command("s.exe -Embedding".to_string())));
        m.put(Hive::LocalMachine, r"SOFTWARE\Classes\AppID\{A}", "", "x");
        assert_eq!(local_plan(&find_class(&m, C).unwrap()), Some(Plan::Command("s.exe -Embedding".to_string())));
        m.put(Hive::LocalMachine, r"SOFTWARE\Classes\AppID\{A}", "LocalService", "Svc");
        assert_eq!(
            local_plan(&find_class(&m, C).unwrap()),
            Some(Plan::Service { name: "Svc".to_string(), args: vec![] })
        );
        m.put(Hive::LocalMachine, r"SOFTWARE\Classes\AppID\{A}", "ServiceParameters", "--one  --two=2 three");
        assert_eq!(
            local_plan(&find_class(&m, C).unwrap()),
            Some(Plan::Service {
                name: "Svc".to_string(),
                args: vec!["--one".to_string(), "--two=2".to_string(), "three".to_string()]
            })
        );
    }

    #[test]
    fn command_programs() {
        assert_eq!(command_program("\"C:\\a b\\s.exe\" --child srv"), "C:\\a b\\s.exe");
        assert_eq!(command_program("C:\\s.exe --x"), "C:\\s.exe");
        assert_eq!(command_program("C:\\s.exe"), "C:\\s.exe");
    }
}
