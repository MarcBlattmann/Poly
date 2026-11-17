//! The task store: the folders and tasks of the machine, each task's definition, whether it is
//! enabled, its last run and result, its missed runs and its running instances. Every operation of
//! `ITaskFolder`, `IRegisteredTask` and `IRunningTask` that needs the store is a method here with
//! the result codes of the sheets; `schedsvc.exe` holds one store and does the I/O (files,
//! processes) the methods ask for.
//!
//! Folder and task objects of `taskschd.dll` hold the path text they were asked with; every call
//! names the object by that text and the store looks it up again (case does not count), so the
//! object of a deleted task still answers its name but no longer its state.

use crate::def::{Definition, action, has, logon, trigger};
use crate::hr::*;
use crate::parse::validate;
use crate::path::{self, components, folder_to_create, join, same, sort_key, task_to_register};
use crate::sched::{self, Events};
use crate::{account, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A running instance of a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    /// `IRunningTask::InstanceGuid`: a braced GUID.
    pub guid: String,
    pub pid: u32,
    /// `IRunningTask::CurrentAction`: the command of the action, without quotes.
    pub action: String,
    pub started: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub name: String,
    pub def: Definition,
    pub enabled: bool,
    /// Seconds since 1970 of the last start, if any.
    pub last_run: Option<i64>,
    pub last_result: Hresult,
    pub missed: i32,
    pub instances: Vec<Instance>,
    /// The time up to which the clock has handled the triggers.
    pub handled_until: i64,
    /// The events the logon, boot and registration triggers count from.
    pub events: Events,
    /// The boot (its time) whose logon the logon triggers have seen.
    pub logon_boot: i64,
    /// The SDDL text given at registration.
    pub sddl: Option<String>,
}

impl Task {
    /// `TASK_STATE`: 4 running, 1 disabled, 3 ready.
    pub fn state(&self) -> i32 {
        if !self.instances.is_empty() {
            4
        } else if !self.enabled {
            1
        } else {
            3
        }
    }

    /// The definition as `get_Definition` gives it (its `Settings.Enabled` is the task's state).
    pub fn definition(&self) -> Definition {
        let mut d = self.def.clone();
        d.settings.enabled = crate::def::vb(self.enabled);
        d
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Folder {
    pub name: String,
    pub folders: Vec<Folder>,
    pub tasks: Vec<Task>,
    pub sddl: Option<String>,
}

impl Folder {
    fn new(name: &str) -> Folder {
        Folder { name: String::from(name), ..Folder::default() }
    }

    fn sub(&self, name: &str) -> Option<usize> {
        self.folders.iter().position(|f| same(&f.name, name))
    }

    fn task(&self, name: &str) -> Option<usize> {
        self.tasks.iter().position(|t| same(&t.name, name))
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.tasks.is_empty()
    }
}

/// Who registers: the account a task without a user is registered for (the caller's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caller {
    /// The caller's account as `DOMAIN\name` and as a SID text.
    pub name: String,
    pub sid: String,
}

/// What `RegisterTaskDefinition` and `RegisterTask` were given.
#[derive(Clone, Debug)]
pub struct Registration {
    /// The path text (`None` for a NULL BSTR).
    pub path: Option<String>,
    pub flags: i32,
    pub user: Option<String>,
    pub logon_type: i32,
    pub sddl: Option<String>,
    /// From `RegisterTask`: the document's own principal decides over the arguments.
    pub from_xml: bool,
}

/// `TASK_CREATION` (taskschd.h).
pub const TASK_VALIDATE_ONLY: i32 = 1;
pub const TASK_CREATE: i32 = 2;
pub const TASK_UPDATE: i32 = 4;
pub const TASK_DISABLE: i32 = 8;
const TASK_CREATION_ALL: i32 = 0x3F;

/// What a registration made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registered {
    /// The canonical path of the task (`\folder\name`, the folders with their stored case).
    pub path: String,
    /// The name of the task as stored (a braced GUID for a NULL path).
    pub name: String,
    /// The folders made on the way (canonical paths), for the files.
    pub made: Vec<String>,
}

/// How a task is started: a new process, or the instance already running (policy IgnoreNew).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunPlan {
    Running(Instance),
    Start {
        guid: String,
        /// The command line: the program (quoted when it has a space) and the arguments.
        command_line: String,
        working_directory: Option<String>,
        /// `CurrentAction`: the program without quotes.
        action: String,
    },
    /// No exec action: nothing to start (the run counts, its result 0).
    Nothing,
}

/// What a `DeleteTask` deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Deleted {
    /// A task (its canonical path); its processes keep running.
    Task(String),
    /// The empty folder whose name was given (its canonical path).
    Folder(String),
}

#[derive(Debug)]
pub struct Store {
    pub root: Folder,
    rng: u64,
}

/// Whether `s` reads as SDDL: empty, or made of `O:`/`G:`/`D:`/`S:` parts with balanced parentheses.
pub fn sddl_ok(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if !["O:", "G:", "D:", "S:"].iter().any(|p| s.starts_with(p)) {
        return false;
    }
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if !(0..=1).contains(&depth) {
            return false;
        }
    }
    depth == 0 && !s.contains(' ')
}

/// The access entries the scheduler adds to a task's DACL after the given ones (sheet
/// `ITaskFolder.RegisterTaskDefinition`, `sddl_argument`).
const INHERITED_DACL: &str = "(A;ID;0x1f019f;;;BA)(A;ID;0x1f019f;;;SY)(A;ID;FA;;;BA)(A;;FR;;;SY)";

impl Default for Store {
    fn default() -> Self {
        Self::new(0x9E37_79B9_7F4A_7C15)
    }
}

impl Store {
    /// An empty store with the folders every Windows has (`\Microsoft\Windows`); `seed` starts the
    /// generator of instance and task GUIDs.
    pub fn new(seed: u64) -> Store {
        let mut root = Folder::new("\\");
        let mut ms = Folder::new("Microsoft");
        ms.folders.push(Folder::new("Windows"));
        root.folders.push(ms);
        Store { root, rng: seed | 1 }
    }

    fn next(&mut self) -> u64 {
        // xorshift64*.
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A new braced GUID (version 4 form).
    pub fn guid(&mut self) -> String {
        let a = self.next();
        let b = self.next();
        format!(
            "{{{:08X}-{:04X}-4{:03X}-{:04X}-{:012X}}}",
            (a >> 32) as u32,
            (a >> 16) as u16,
            (a & 0x0FFF) as u16,
            (((b >> 48) as u16) & 0x3FFF) | 0x8000,
            b & 0xFFFF_FFFF_FFFF
        )
    }

    // ------------------------------------------------------------------ folders

    fn find(&self, comps: &[String]) -> Option<&Folder> {
        let mut f = &self.root;
        for c in comps {
            f = &f.folders[f.sub(c)?];
        }
        Some(f)
    }

    fn find_mut(&mut self, comps: &[String]) -> Option<&mut Folder> {
        let mut f = &mut self.root;
        for c in comps {
            let i = f.sub(c)?;
            f = &mut f.folders[i];
        }
        Some(f)
    }

    /// The canonical components (stored case) of existing folders.
    fn canonical(&self, comps: &[String]) -> Option<Vec<String>> {
        let mut f = &self.root;
        let mut out = Vec::new();
        for c in comps {
            f = &f.folders[f.sub(c)?];
            out.push(f.name.clone());
        }
        Some(out)
    }

    /// The components of a folder object's own path text.
    fn base(&self, folder: &str) -> Result<Vec<String>, Hresult> {
        let c = components(folder)?;
        if self.find(&c).is_none() {
            return Err(NOT_FOUND);
        }
        Ok(c)
    }

    /// `GetFolder(rel)` asked of the folder `folder`: whether the folder exists.
    pub fn get_folder(&self, folder: &str, rel: &str) -> Result<(), Hresult> {
        let rel = components(rel)?;
        let mut c = components(folder)?;
        c.extend(rel);
        if self.find(&c).is_some() { Ok(()) } else { Err(NOT_FOUND) }
    }

    /// `CreateFolder(rel, sddl)`: makes the folder and the missing ones above it; returns the
    /// canonical paths of the folders made.
    pub fn create_folder(&mut self, folder: &str, rel: Option<&str>, sddl: Option<&str>) -> Result<Vec<String>, Hresult> {
        if let Some(s) = sddl
            && !sddl_ok(s)
        {
            return Err(E_INVALIDARG);
        }
        let rel = folder_to_create(rel.unwrap_or(""))?;
        let mut c = components(folder)?;
        if rel.is_empty() {
            // The folder itself.
            return Err(if self.find(&c).is_some() { ALREADY_EXISTS } else { NOT_FOUND });
        }
        c.extend(rel);
        if self.find(&c).is_some() {
            return Err(ALREADY_EXISTS);
        }
        let mut made = Vec::new();
        let mut f = &mut self.root;
        let mut canon = Vec::new();
        for n in &c {
            let i = match f.sub(n) {
                Some(i) => i,
                None => {
                    f.folders.push(Folder::new(n));
                    canon.push(n.clone());
                    made.push(join(&canon));
                    canon.pop();
                    f.folders.len() - 1
                }
            };
            f = &mut f.folders[i];
            canon.push(f.name.clone());
        }
        f.sddl = sddl.filter(|s| !s.is_empty()).map(String::from);
        Ok(made)
    }

    /// `DeleteFolder(rel)`: an empty or NULL name is the folder itself. Returns the canonical path.
    pub fn delete_folder(&mut self, folder: &str, rel: Option<&str>) -> Result<String, Hresult> {
        let rel = components(rel.unwrap_or(""))?;
        let mut c = components(folder)?;
        c.extend(rel);
        if c.is_empty() {
            return Err(E_ACCESSDENIED);
        }
        let canon = self.canonical(&c).ok_or(NOT_FOUND)?;
        let name = c.pop().unwrap_or_default();
        if name.contains(['*', '?', '<', '>', '|', '"']) {
            return Err(INVALID_NAME);
        }
        let parent = self.find_mut(&c).ok_or(NOT_FOUND)?;
        let i = parent.sub(&name).ok_or(NOT_FOUND)?;
        if !parent.folders[i].is_empty() {
            return Err(DIR_NOT_EMPTY);
        }
        parent.folders.remove(i);
        Ok(join(&canon))
    }

    /// `GetFolders(flags)`: the names of the direct subfolders, alphabetical without regard to case.
    pub fn folders(&self, folder: &str, flags: i32) -> Result<Vec<String>, Hresult> {
        if flags != 0 {
            return Err(E_INVALIDARG);
        }
        let c = self.base(folder)?;
        let f = self.find(&c).ok_or(NOT_FOUND)?;
        let mut v: Vec<String> = f.folders.iter().map(|f| f.name.clone()).collect();
        v.sort_by_key(|n| sort_key(n));
        Ok(v)
    }

    /// `GetTasks(flags)`: the names of the folder's tasks (hidden ones only with `TASK_ENUM_HIDDEN`).
    pub fn tasks(&self, folder: &str, flags: i32) -> Result<Vec<String>, Hresult> {
        if flags != 0 && flags != 1 {
            return Err(E_INVALIDARG);
        }
        let c = self.base(folder)?;
        let f = self.find(&c).ok_or(NOT_FOUND)?;
        let mut v: Vec<String> =
            f.tasks.iter().filter(|t| flags == 1 || t.def.settings.hidden == 0).map(|t| t.name.clone()).collect();
        v.sort_by_key(|n| sort_key(n));
        Ok(v)
    }

    /// Make the folder of a canonical path (and those above it) if it is missing (loading the files).
    pub fn ensure_folder(&mut self, p: &str) {
        let Ok(c) = components(p) else { return };
        let mut f = &mut self.root;
        for n in &c {
            let i = match f.sub(n) {
                Some(i) => i,
                None => {
                    f.folders.push(Folder::new(n));
                    f.folders.len() - 1
                }
            };
            f = &mut f.folders[i];
        }
    }

    /// Put a task read from its file at its canonical path (replacing one of that name).
    pub fn insert(&mut self, p: &str, def: Definition, now: i64, boot: i64) {
        let Ok(mut c) = components(p) else { return };
        let Some(name) = c.pop() else { return };
        self.ensure_folder(&join(&c));
        let Some(f) = self.find_mut(&c) else { return };
        if let Some(i) = f.task(&name) {
            f.tasks.remove(i);
        }
        let enabled = def.settings.enabled != 0;
        f.tasks.push(Task {
            name,
            def,
            enabled,
            last_run: None,
            last_result: SCHED_S_TASK_HAS_NOT_RUN,
            missed: 0,
            instances: Vec::new(),
            handled_until: now,
            events: Events::default(),
            logon_boot: boot,
            sddl: None,
        });
    }

    // ------------------------------------------------------------------ tasks

    /// The task a path text names (`\folder\name`, case does not count).
    pub fn task(&self, p: &str) -> Option<&Task> {
        let mut c = components(p).ok()?;
        let name = c.pop()?;
        let f = self.find(&c)?;
        f.tasks.get(f.task(&name)?)
    }

    pub fn task_mut(&mut self, p: &str) -> Option<&mut Task> {
        let mut c = components(p).ok()?;
        let name = c.pop()?;
        let f = self.find_mut(&c)?;
        let i = f.task(&name)?;
        f.tasks.get_mut(i)
    }

    /// `GetTask(rel)` asked of the folder `folder`: the canonical path of the task.
    pub fn get_task(&self, folder: &str, rel: &str) -> Result<String, Hresult> {
        let mut rel = components(rel)?;
        let name = rel.pop().ok_or(NOT_FOUND)?;
        let mut c = components(folder)?;
        c.extend(rel);
        let f = self.find(&c).ok_or(NOT_FOUND)?;
        let t = &f.tasks[f.task(&name).ok_or(NOT_FOUND)?];
        let mut canon = self.canonical(&c).ok_or(NOT_FOUND)?;
        canon.push(t.name.clone());
        Ok(join(&canon))
    }

    /// `DeleteTask(rel, flags)`: a task, or an empty folder of that name; the empty or NULL name is
    /// the folder itself.
    pub fn delete_task(&mut self, folder: &str, rel: Option<&str>, flags: i32) -> Result<Deleted, Hresult> {
        if flags != 0 {
            return Err(E_INVALIDARG);
        }
        let rel = rel.unwrap_or("");
        if rel.is_empty() {
            return self.delete_folder(folder, None).map(Deleted::Folder);
        }
        let mut rc = components(rel)?;
        let name = rc.pop().ok_or(NOT_FOUND)?;
        let mut c = components(folder)?;
        c.extend(rc);
        let canon_folder = self.canonical(&c).ok_or(NOT_FOUND)?;
        let f = self.find_mut(&c).ok_or(NOT_FOUND)?;
        if let Some(i) = f.task(&name) {
            let t = f.tasks.remove(i);
            let mut canon = canon_folder;
            canon.push(t.name);
            return Ok(Deleted::Task(join(&canon)));
        }
        if let Some(i) = f.sub(&name) {
            if !f.folders[i].is_empty() {
                return Err(DIR_NOT_EMPTY);
            }
            let sub = f.folders.remove(i);
            let mut canon = canon_folder;
            canon.push(sub.name);
            return Ok(Deleted::Folder(join(&canon)));
        }
        Err(NOT_FOUND)
    }

    /// The principal a registration stores (sheet `ITaskFolder.RegisterTaskDefinition`,
    /// `user_argument` and `logon_type`).
    fn principal(def: &mut Definition, r: &Registration, caller: &Caller) -> Result<(), Hresult> {
        if !(0..=6).contains(&r.logon_type) {
            return Err(E_INVALIDARG);
        }
        let own = def.principal.user_id.clone().filter(|u| !u.is_empty());
        let given = r.user.clone().filter(|u| !u.is_empty());
        let user = if r.from_xml { own.or(given) } else { given.or(own) };
        let logon_type = if r.from_xml && def.principal.user_id.is_some() { def.principal.logon_type } else { r.logon_type };
        match user {
            Some(u) if account::is_service_account(&u) => {
                def.principal.user_id = Some(account::canonical(&u));
                def.principal.logon_type = logon::SERVICE_ACCOUNT;
            }
            Some(u) if account::is_known(&u) && !u.eq_ignore_ascii_case("LocalSystem") => {
                // A group (`Users`, `Administrators`) as the user.
                def.principal.user_id = Some(account::canonical(&u));
                def.principal.logon_type = if logon_type == 0 { logon::INTERACTIVE_TOKEN } else { logon_type };
            }
            Some(u) if same(&u, &caller.name) || same(&u, &caller.sid) || caller.name.rsplit('\\').next().is_some_and(|n| same(n, &u)) => {
                def.principal.user_id = Some(caller.sid.clone());
                def.principal.logon_type = Self::own_logon(logon_type)?;
            }
            Some(_) => return Err(NONE_MAPPED),
            None => {
                def.principal.user_id = Some(caller.sid.clone());
                def.principal.logon_type = Self::own_logon(logon_type)?;
            }
        }
        def.principal.id = Some(String::from("Author"));
        Ok(())
    }

    /// The logon type stored for the caller's own account.
    fn own_logon(t: i32) -> Result<i32, Hresult> {
        match t {
            logon::NONE => Ok(logon::INTERACTIVE_TOKEN),
            logon::GROUP => Err(SCHED_E_INVALIDVALUE),
            t => Ok(t),
        }
    }

    /// `RegisterTaskDefinition` / `RegisterTask` asked of the folder `folder` at the time `now`.
    /// `Ok(None)` for a validation only.
    pub fn register(
        &mut self,
        folder: &str,
        def: &Definition,
        r: &Registration,
        caller: &Caller,
        now: i64,
        boot: i64,
    ) -> Result<Option<Registered>, Hresult> {
        let f = r.flags;
        if f < 0 || f & !TASK_CREATION_ALL != 0 || (f & TASK_VALIDATE_ONLY != 0 && f & (TASK_CREATE | TASK_UPDATE) != 0) {
            return Err(E_INVALIDARG);
        }
        if !(0..=6).contains(&r.logon_type) {
            return Err(E_INVALIDARG);
        }
        let (mut comps, name) = match r.path.as_deref().filter(|p| !p.is_empty()) {
            None => (Vec::new(), None),
            Some(p) => {
                let mut c = task_to_register(p)?;
                let n = c.pop();
                // A leading backslash asked of a folder object is ignored (as for `GetTask`).
                let mut base = components(folder)?;
                base.extend(c);
                (base, n)
            }
        };
        if let Some(s) = &r.sddl
            && !sddl_ok(s)
        {
            return Err(E_INVALIDARG);
        }
        validate(def)?;
        let mut d = def.clone();
        Self::principal(&mut d, r, caller)?;
        if f & TASK_VALIDATE_ONLY != 0 {
            return Ok(None);
        }
        let name = match name {
            Some(n) => n,
            None => {
                comps.clear();
                self.guid()
            }
        };
        let existing = self.find(&comps).and_then(|fo| fo.task(&name).map(|i| fo.tasks[i].name.clone()));
        let create = f & TASK_CREATE != 0 || f & (TASK_CREATE | TASK_UPDATE) == 0;
        let update = f & TASK_UPDATE != 0;
        match (&existing, create, update) {
            (Some(_), _, false) => return Err(ALREADY_EXISTS),
            (None, false, _) => return Err(NOT_FOUND),
            _ => {}
        }
        // The folders on the way.
        let mut made = Vec::new();
        let mut canon = Vec::new();
        {
            let mut fo = &mut self.root;
            for n in &comps {
                let i = match fo.sub(n) {
                    Some(i) => i,
                    None => {
                        fo.folders.push(Folder::new(n));
                        let mut p = canon.clone();
                        p.push(n.clone());
                        made.push(join(&p));
                        fo.folders.len() - 1
                    }
                };
                fo = &mut fo.folders[i];
                canon.push(fo.name.clone());
            }
        }
        let stored_name = existing.unwrap_or(name);
        let mut full = canon.clone();
        full.push(stored_name.clone());
        let path = join(&full);
        d.reg.uri = Some(path.clone());
        d.context = Some(String::from("Author"));
        if f & TASK_DISABLE != 0 {
            d.settings.enabled = 0;
        }
        let enabled = d.settings.enabled != 0;
        let fo = self.find_mut(&canon).ok_or(NOT_FOUND)?;
        let sddl = r.sddl.clone().filter(|s| !s.is_empty());
        match fo.task(&stored_name) {
            Some(i) => {
                let t = &mut fo.tasks[i];
                t.def = d;
                t.enabled = enabled;
                t.sddl = sddl;
                t.events.registered = Some(now);
                t.handled_until = now;
            }
            None => fo.tasks.push(Task {
                name: stored_name.clone(),
                def: d,
                enabled,
                last_run: None,
                last_result: SCHED_S_TASK_HAS_NOT_RUN,
                missed: 0,
                instances: Vec::new(),
                handled_until: now,
                events: Events { registered: Some(now), ..Events::default() },
                logon_boot: boot,
                sddl,
            }),
        }
        Ok(Some(Registered { path, name: stored_name, made }))
    }

    /// `put_Enabled`.
    pub fn set_enabled(&mut self, p: &str, on: bool) -> Result<(), Hresult> {
        let t = self.task_mut(p).ok_or(NOT_FOUND)?;
        t.enabled = on;
        Ok(())
    }

    /// `GetSecurityDescriptor`: the task's DACL as SDDL.
    pub fn security_descriptor(&self, p: &str) -> Result<String, Hresult> {
        let t = self.task(p).ok_or(NOT_FOUND)?;
        let mut s = t.sddl.clone().unwrap_or_else(|| String::from("D:"));
        if !s.starts_with("D:") {
            s.insert_str(0, "D:");
        }
        s.push_str(INHERITED_DACL);
        Ok(s)
    }

    // ------------------------------------------------------------------ running

    /// How to start the task `p` (`Run`, or a trigger when `by_trigger`).
    pub fn begin_run(&mut self, p: &str, by_trigger: bool) -> Result<RunPlan, Hresult> {
        let mut guid = self.guid();
        let t = self.task_mut(p).ok_or(NOT_FOUND)?;
        if !t.enabled {
            return Err(SCHED_E_TASK_DISABLED);
        }
        if !by_trigger && t.def.settings.allow_demand_start == 0 {
            return Err(SCHED_E_START_ON_DEMAND);
        }
        if let Some(first) = t.instances.first()
            && t.def.settings.multiple_instances != 0
        {
            return Ok(RunPlan::Running(first.clone()));
        }
        let Some(a) = t.def.actions.iter().find(|a| a.kind == action::EXEC && has(&a.path)) else {
            return Ok(RunPlan::Nothing);
        };
        let program = a.path.clone().unwrap_or_default();
        let bare = String::from(program.trim().trim_matches('"'));
        let mut cmd = if program.trim_start().starts_with('"') || !bare.contains(' ') {
            String::from(program.trim())
        } else {
            format!("\"{bare}\"")
        };
        if let Some(args) = a.arguments.as_deref().filter(|s| !s.is_empty()) {
            cmd.push(' ');
            cmd.push_str(args);
        }
        let wd = a.working_directory.clone().filter(|s| !s.is_empty());
        if guid.is_empty() {
            guid = String::from("{00000000-0000-0000-0000-000000000000}");
        }
        Ok(RunPlan::Start { guid, command_line: cmd, working_directory: wd, action: bare })
    }

    /// The process of a [`RunPlan::Start`] runs.
    pub fn started(&mut self, p: &str, inst: Instance, now: i64) {
        if let Some(t) = self.task_mut(p) {
            t.instances.push(inst);
            t.last_run = Some(now);
            t.last_result = SCHED_S_TASK_RUNNING;
        }
    }

    /// The task ran without a process ([`RunPlan::Nothing`]), or its process could not start
    /// (`result` the error).
    pub fn ran_without_process(&mut self, p: &str, result: Hresult, now: i64) {
        if let Some(t) = self.task_mut(p) {
            t.last_run = Some(now);
            t.last_result = result;
        }
    }

    /// The task that has the instance `guid`.
    fn with_instance<'a>(f: &'a mut Folder, guid: &str) -> Option<&'a mut Task> {
        if let Some(i) = f.tasks.iter().position(|t| t.instances.iter().any(|x| x.guid == guid)) {
            return f.tasks.get_mut(i);
        }
        for s in &mut f.folders {
            if let Some(t) = Self::with_instance(s, guid) {
                return Some(t);
            }
        }
        None
    }

    /// Every task with its canonical path.
    pub fn each_task(&self) -> Vec<(String, &Task)> {
        fn walk<'a>(f: &'a Folder, at: &mut Vec<String>, out: &mut Vec<(String, &'a Task)>) {
            for t in &f.tasks {
                let mut p = at.clone();
                p.push(t.name.clone());
                out.push((join(&p), t));
            }
            for s in &f.folders {
                at.push(s.name.clone());
                walk(s, at, out);
                at.pop();
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut Vec::new(), &mut out);
        out
    }

    /// Every folder's canonical path (the root first).
    pub fn each_folder(&self) -> Vec<String> {
        fn walk(f: &Folder, at: &mut Vec<String>, out: &mut Vec<String>) {
            out.push(join(at));
            for s in &f.folders {
                at.push(s.name.clone());
                walk(s, at, out);
                at.pop();
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut Vec::new(), &mut out);
        out
    }

    /// The instance `guid` ended with `code`: its task is ready again with that result. Whether
    /// the instance was still known (a stopped or deleted task's process is not).
    pub fn ended(&mut self, guid: &str, code: i32) -> bool {
        let Some(t) = Self::with_instance(&mut self.root, guid) else { return false };
        t.instances.retain(|x| x.guid != guid);
        t.last_result = code;
        true
    }

    /// `IRegisteredTask::Stop(flags)`: the instances to end (their GUIDs); `S_FALSE` (as an
    /// empty list and `Ok(false)`) when none runs.
    pub fn stop(&mut self, p: &str, flags: i32) -> Result<Vec<Instance>, Hresult> {
        if flags != 0 {
            return Err(E_INVALIDARG);
        }
        let t = self.task_mut(p).ok_or(NOT_FOUND)?;
        if t.instances.is_empty() {
            return Ok(Vec::new());
        }
        t.last_result = SCHED_S_TASK_TERMINATED;
        Ok(core::mem::take(&mut t.instances))
    }

    /// `IRunningTask::Stop`: the instance to end.
    pub fn stop_instance(&mut self, guid: &str) -> Result<Instance, Hresult> {
        let t = Self::with_instance(&mut self.root, guid).ok_or(SCHED_E_TASK_NOT_RUNNING)?;
        let i = t.instances.iter().position(|x| x.guid == guid).ok_or(SCHED_E_TASK_NOT_RUNNING)?;
        let inst = t.instances.remove(i);
        t.last_result = SCHED_S_TASK_TERMINATED;
        Ok(inst)
    }

    /// The running instances of the machine (`GetRunningTasks`; hidden tasks only with flag 1),
    /// with each task's path.
    pub fn running(&self, flags: i32) -> Result<Vec<(String, Instance)>, Hresult> {
        if flags != 0 && flags != 1 {
            return Err(E_INVALIDARG);
        }
        let mut v = Vec::new();
        for (p, t) in self.each_task() {
            if flags == 0 && t.def.settings.hidden != 0 {
                continue;
            }
            for i in &t.instances {
                v.push((p.clone(), i.clone()));
            }
        }
        Ok(v)
    }

    /// `IRegisteredTask::NextRunTime` as seconds since 1970 (`None`: no time trigger is coming).
    pub fn next_run(&self, p: &str, now: i64) -> Option<i64> {
        let t = self.task(p)?;
        sched::next_fire(&t.def, &t.events, now)
    }

    /// The tasks whose triggers fire in `(handled_until, now]`: their paths; each one's
    /// `handled_until` moves to `now`.
    pub fn due(&mut self, now: i64) -> Vec<String> {
        let mut due = Vec::new();
        let paths: Vec<String> = self.each_task().into_iter().map(|(p, _)| p).collect();
        for p in paths {
            let Some(t) = self.task_mut(&p) else { continue };
            if t.handled_until >= now {
                continue;
            }
            let fire = t.enabled && sched::next_fire(&t.def, &t.events, t.handled_until).is_some_and(|f| f <= now);
            t.handled_until = now;
            if fire {
                due.push(p);
            }
        }
        due
    }

    /// The scheduler starts after a boot at `boot` (its time) at `now`: count the runs missed
    /// while it was not running, and let the logon and boot triggers of this boot fire. Returns the
    /// tasks to run now (missed runs of tasks that start when available).
    pub fn starting(&mut self, now: i64, boot: i64) -> Vec<String> {
        let mut run = Vec::new();
        let paths: Vec<String> = self.each_task().into_iter().map(|(p, _)| p).collect();
        for p in paths {
            let Some(t) = self.task_mut(&p) else { continue };
            if t.enabled && t.handled_until < now {
                let n = sched::fires_between(&t.def, &t.events, t.handled_until, now);
                if n > 0 {
                    t.missed = t.missed.saturating_add(n as i32);
                    if t.def.settings.start_when_available != 0 {
                        run.push(p.clone());
                    }
                }
            }
            if (t.logon_boot - boot).abs() > 60 {
                t.logon_boot = boot;
                t.events.boot = Some(boot);
                t.events.logon = Some(now);
                if t.def.triggers.iter().any(|x| matches!(x.kind, trigger::LOGON | trigger::BOOT)) {
                    // The logon and boot of this boot are due now.
                    t.handled_until = now - 1;
                    continue;
                }
            }
            t.handled_until = now;
        }
        run
    }
}

/// `LastRunTime` as a `DATE`.
pub fn last_run_date(t: &Task) -> f64 {
    t.last_run.map_or(time::NEVER_RUN, time::date_from_unix)
}

/// The last component of a canonical path.
pub fn name_of(p: &str) -> String {
    path::last(p)
}
