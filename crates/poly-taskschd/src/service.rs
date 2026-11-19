//! The scheduler's side of the pipe (ADR 0226, section 4): one request in, one reply out, and the
//! clock. `schedsvc.exe` gives it a [`Host`] for what needs the system (the time, processes, files)
//! and serialises the calls with a lock.

use crate::hr::*;
use crate::persist::{self, TaskState};
use crate::store::{Caller, Deleted, Instance, Registration, RunPlan, Store, last_run_date, name_of};
use crate::time;
use crate::wire::{Info, InstanceInfo, Reply, Request};
use crate::write::registered_xml;
use alloc::string::String;
use alloc::vec::Vec;

/// What the scheduler needs of the system.
pub trait Host {
    /// Seconds since 1970 (local time, UTC on Poly).
    fn now(&self) -> i64;
    /// The time of this boot (seconds since 1970).
    fn boot(&self) -> i64;
    /// Start a task's process; its id, or the error (an `HRESULT`). The host watches it and calls
    /// [`Service::ended`] when it ends.
    fn spawn(&mut self, guid: &str, command_line: &str, working_directory: Option<&str>) -> Result<u32, Hresult>;
    /// End the process of an instance (Stop).
    fn terminate(&mut self, guid: &str);
    /// Stop watching an instance's process and leave it running (its task was deleted).
    fn forget(&mut self, guid: &str);
    /// Write a task's file (its canonical path) with its registered XML.
    fn write_task(&mut self, path: &str, xml: &str);
    fn delete_task(&mut self, path: &str);
    fn make_folder(&mut self, path: &str);
    fn remove_folder(&mut self, path: &str);
    fn save_state(&mut self, text: &str);
    /// Make the scheduler start at boot (`on`) or only on demand: it starts at boot while it holds
    /// tasks whose triggers fire.
    fn set_boot_start(&mut self, on: bool);
}

#[derive(Debug)]
pub struct Service {
    pub store: Store,
    /// What the scheduler's start type was last set to.
    boot_start: Option<bool>,
}

fn info_of(i: &Instance, path: &str, state: i32) -> InstanceInfo {
    InstanceInfo {
        path: String::from(path),
        name: name_of(path),
        guid: i.guid.clone(),
        pid: i.pid,
        action: i.action.clone(),
        state,
    }
}

impl Service {
    pub fn new(seed: u64) -> Service {
        Service { store: Store::new(seed), boot_start: None }
    }

    /// Whether any task has a trigger that fires (the scheduler must then start at boot).
    pub fn needs_boot_start(&self) -> bool {
        use crate::def::trigger::*;
        self.store
            .each_task()
            .iter()
            .any(|(_, t)| t.def.triggers.iter().any(|x| matches!(x.kind, TIME | DAILY | LOGON | BOOT | REGISTRATION)))
    }

    fn update_boot_start(&mut self, h: &mut dyn Host) {
        let want = self.needs_boot_start();
        if self.boot_start != Some(want) {
            self.boot_start = Some(want);
            h.set_boot_start(want);
        }
    }

    /// Persist every task's state.
    pub fn save(&mut self, h: &mut dyn Host) {
        self.update_boot_start(h);
        let states: Vec<TaskState> =
            self.store.each_task().into_iter().map(|(p, t)| persist::state_of(&p, t)).collect();
        h.save_state(&persist::encode(&states));
    }

    /// After loading the files: apply the saved states, count missed runs, let this boot's logon and
    /// boot triggers fire, and run what starts when available.
    pub fn start(&mut self, saved: &[TaskState], h: &mut dyn Host) {
        for s in saved {
            if let Some(t) = self.store.task_mut(&s.path) {
                persist::apply(t, s);
            }
        }
        let now = h.now();
        let run = self.store.starting(now, h.boot());
        for p in run {
            let _ = self.run(&p, true, h);
        }
        self.save(h);
    }

    /// The clock: run the tasks whose triggers fired.
    pub fn tick(&mut self, h: &mut dyn Host) {
        let now = h.now();
        let due = self.store.due(now);
        let any = !due.is_empty();
        for p in due {
            let _ = self.run(&p, true, h);
        }
        if any {
            self.save(h);
        }
    }

    /// An instance's process ended with `code`.
    pub fn ended(&mut self, guid: &str, code: u32, h: &mut dyn Host) {
        if self.store.ended(guid, code as i32) {
            self.save(h);
        }
    }

    fn run(&mut self, path: &str, by_trigger: bool, h: &mut dyn Host) -> Result<InstanceInfo, Hresult> {
        let plan = self.store.begin_run(path, by_trigger)?;
        let canon = self.canonical_task(path).unwrap_or_else(|| String::from(path));
        let now = h.now();
        match plan {
            RunPlan::Running(i) => Ok(info_of(&i, &canon, 4)),
            RunPlan::Nothing => {
                self.store.ran_without_process(path, S_OK, now);
                let guid = self.store.guid();
                Ok(InstanceInfo {
                    path: canon.clone(),
                    name: name_of(&canon),
                    guid,
                    pid: 0,
                    action: String::new(),
                    state: 3,
                })
            }
            RunPlan::Start { guid, command_line, working_directory, action } => {
                match h.spawn(&guid, &command_line, working_directory.as_deref()) {
                    Ok(pid) => {
                        let inst = Instance { guid, pid, action, started: now };
                        let info = info_of(&inst, &canon, 4);
                        self.store.started(path, inst, now);
                        Ok(info)
                    }
                    Err(hr) => {
                        self.store.ran_without_process(path, hr, now);
                        Ok(InstanceInfo { path: canon.clone(), name: name_of(&canon), guid, pid: 0, action, state: 3 })
                    }
                }
            }
        }
    }

    fn canonical_task(&self, path: &str) -> Option<String> {
        self.store.get_task("\\", path).ok()
    }

    /// Answer one request.
    pub fn handle(&mut self, req: Request, h: &mut dyn Host) -> Reply {
        match self.answer(req, h) {
            Ok(r) => r,
            Err(hr) => Reply::hr(hr),
        }
    }

    fn answer(&mut self, req: Request, h: &mut dyn Host) -> Result<Reply, Hresult> {
        let ok = Reply::hr(S_OK);
        Ok(match req {
            Request::Ping => ok,
            Request::GetFolder { folder, rel } => {
                self.store.get_folder(&folder, &rel)?;
                ok
            }
            Request::CreateFolder { folder, rel, sddl } => {
                let made = self.store.create_folder(&folder, rel.as_deref(), sddl.as_deref())?;
                for m in made {
                    h.make_folder(&m);
                }
                ok
            }
            Request::DeleteFolder { folder, rel } => {
                let p = self.store.delete_folder(&folder, rel.as_deref())?;
                h.remove_folder(&p);
                ok
            }
            Request::Folders { folder, flags } => Reply { names: self.store.folders(&folder, flags)?, ..ok },
            Request::Tasks { folder, flags } => Reply { names: self.store.tasks(&folder, flags)?, ..ok },
            Request::GetTask { folder, rel } => Reply { text: self.store.get_task(&folder, &rel)?, ..ok },
            Request::DeleteTask { folder, rel, flags } => {
                let running: Vec<String> = rel
                    .as_deref()
                    .filter(|r| !r.is_empty() && flags == 0)
                    .and_then(|r| self.store.get_task(&folder, r).ok())
                    .and_then(|p| self.store.task(&p).map(|t| t.instances.iter().map(|i| i.guid.clone()).collect()))
                    .unwrap_or_default();
                match self.store.delete_task(&folder, rel.as_deref(), flags)? {
                    Deleted::Task(p) => {
                        for g in running {
                            h.forget(&g);
                        }
                        h.delete_task(&p);
                        self.save(h);
                    }
                    Deleted::Folder(p) => h.remove_folder(&p),
                }
                ok
            }
            Request::Register {
                folder,
                def,
                path,
                flags,
                user,
                logon_type,
                sddl,
                from_xml,
                caller_name,
                caller_sid,
            } => {
                let r = Registration { path, flags, user, logon_type, sddl, from_xml };
                let caller = Caller { name: caller_name, sid: caller_sid };
                let now = h.now();
                let boot = h.boot();
                match self.store.register(&folder, &def, &r, &caller, now, boot)? {
                    None => ok,
                    Some(reg) => {
                        for m in &reg.made {
                            h.make_folder(m);
                        }
                        if let Some(t) = self.store.task(&reg.path) {
                            h.write_task(&reg.path, &registered_xml(&t.def));
                        }
                        self.save(h);
                        Reply { text: reg.path, names: alloc::vec![reg.name], ..ok }
                    }
                }
            }
            Request::Info { path } => {
                let now = h.now();
                let next = self.store.next_run(&path, now);
                let t = self.store.task(&path).ok_or(NOT_FOUND)?;
                Reply {
                    info: Some(Info {
                        state: t.state(),
                        enabled: t.enabled,
                        last_run: last_run_date(t),
                        next_run: next.map_or(0.0, time::date_from_unix),
                        last_result: t.last_result,
                        missed: t.missed,
                    }),
                    ..ok
                }
            }
            Request::SetEnabled { path, on } => {
                self.store.set_enabled(&path, on)?;
                self.save(h);
                ok
            }
            Request::Xml { path } => {
                let t = self.store.task(&path).ok_or(NOT_FOUND)?;
                Reply { text: registered_xml(&t.def), ..ok }
            }
            Request::Definition { path } => {
                let t = self.store.task(&path).ok_or(NOT_FOUND)?;
                Reply { def: Some(t.definition()), ..ok }
            }
            Request::Run { path } => {
                let i = self.run(&path, false, h)?;
                self.save(h);
                Reply { instances: alloc::vec![i], ..ok }
            }
            Request::Stop { path, flags } => {
                let stopped = self.store.stop(&path, flags)?;
                if stopped.is_empty() {
                    return Ok(Reply::hr(S_FALSE));
                }
                for i in &stopped {
                    h.terminate(&i.guid);
                }
                self.save(h);
                ok
            }
            Request::Instances { path } => {
                let canon = self.canonical_task(&path).ok_or(NOT_FOUND)?;
                let t = self.store.task(&path).ok_or(NOT_FOUND)?;
                Reply { instances: t.instances.iter().map(|i| info_of(i, &canon, 4)).collect(), ..ok }
            }
            Request::Running { flags } => {
                Reply { instances: self.store.running(flags)?.iter().map(|(p, i)| info_of(i, p, 4)).collect(), ..ok }
            }
            Request::StopInstance { guid } => {
                let i = self.store.stop_instance(&guid)?;
                h.terminate(&i.guid);
                self.save(h);
                ok
            }
            Request::SecurityDescriptor { path } => Reply { text: self.store.security_descriptor(&path)?, ..ok },
        })
    }
}
