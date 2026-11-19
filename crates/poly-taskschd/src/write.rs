//! The two XML forms of a definition (TASKSCHD.md section 4, the sheets `ITaskService.NewTask`,
//! `IRegisteredTask.Xml`, `ITriggerCollection.Create`, `ITaskSettings.MultipleInstances`,
//! `IPrincipal.UserId`, `IRegistrationInfo.Description`, `IActionCollection.Create`):
//!
//! - [`definition_xml`]: `ITaskDefinition::get_XmlText`, every setting written, each trigger with
//!   its `Enabled`, the principal only when something in it was set.
//! - [`registered_xml`]: `IRegisteredTask::get_Xml`, the stored form: defaults left out, the parts in
//!   the order RegistrationInfo, Principals, Settings, Triggers, Actions, the principal `Author`.

use crate::account;
use crate::def::{Action, Definition, Principal, RegistrationInfo, Settings, Text, Trigger, action, has, trigger};
use crate::xml::Writer;
use alloc::format;
use alloc::string::String;

pub const NAMESPACE: &str = "http://schemas.microsoft.com/windows/2004/02/mit/task";
const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-16\"?>";

/// The schema version a definition is written with: 1.2 for the default compatibility (V2), the
/// version of a later compatibility level otherwise.
pub fn version(d: &Definition) -> String {
    let c = d.settings.compatibility;
    if (3..=6).contains(&c) { format!("1.{c}") } else { String::from("1.2") }
}

fn b(v: i16) -> &'static str {
    if v != 0 { "true" } else { "false" }
}

fn opt(w: &mut Writer, name: &str, t: &Text) {
    if let Some(s) = t.as_deref().filter(|s| !s.is_empty()) {
        w.leaf(name, s);
    }
}

fn policy(v: i32) -> &'static str {
    match v {
        0 => "Parallel",
        1 => "Queue",
        3 => "StopExisting",
        _ => "IgnoreNew",
    }
}

fn logon_name(v: i32) -> Option<&'static str> {
    Some(match v {
        1 => "Password",
        2 => "S4U",
        3 => "InteractiveToken",
        4 => "Group",
        5 => "ServiceAccount",
        6 => "InteractiveTokenOrPassword",
        _ => return None,
    })
}

fn begin(w: &mut Writer, d: &Definition) {
    w.raw(DECLARATION);
    let v = version(d);
    w.open("Task", &[("version", &v), ("xmlns", NAMESPACE)]);
}

fn registration(w: &mut Writer, r: &RegistrationInfo) {
    let items = [
        ("Date", &r.date),
        ("Author", &r.author),
        ("Version", &r.version),
        ("Source", &r.source),
        ("Description", &r.description),
        ("Documentation", &r.documentation),
        ("URI", &r.uri),
        ("SecurityDescriptor", &r.security_descriptor),
    ];
    if !items.iter().any(|(_, t)| has(t)) {
        w.empty("RegistrationInfo", &[]);
        return;
    }
    w.open("RegistrationInfo", &[]);
    for (n, t) in items {
        opt(w, n, t);
    }
    w.close("RegistrationInfo");
}

fn principal(w: &mut Writer, p: &Principal, id: Option<&str>) {
    w.open("Principals", &[]);
    match id.filter(|s| !s.is_empty()) {
        Some(id) => w.open("Principal", &[("id", id)]),
        None => w.open("Principal", &[]),
    }
    if let Some(u) = p.user_id.as_deref().filter(|s| !s.is_empty()) {
        w.leaf("UserId", &account::xml_form(u));
    }
    if let Some(g) = p.group_id.as_deref().filter(|s| !s.is_empty()) {
        w.leaf("GroupId", &account::xml_form(g));
    }
    opt(w, "DisplayName", &p.display_name);
    if let Some(u) = p.user_id.as_deref().filter(|s| !s.is_empty())
        && !account::is_service_account(u)
        && let Some(n) = logon_name(p.logon_type)
    {
        let _ = u;
        w.leaf("LogonType", n);
    }
    if p.run_level == 1 {
        w.leaf("RunLevel", "HighestAvailable");
    }
    w.close("Principal");
    w.close("Principals");
}

/// Whether anything of a definition's principal was set (a new definition's XML has none).
fn principal_set(p: &Principal) -> bool {
    has(&p.id) || has(&p.display_name) || has(&p.user_id) || has(&p.group_id) || p.logon_type != 3 || p.run_level != 0
}

fn idle(w: &mut Writer, s: &Settings) {
    w.open("IdleSettings", &[]);
    opt(w, "Duration", &s.idle.duration);
    opt(w, "WaitTimeout", &s.idle.wait_timeout);
    w.leaf("StopOnIdleEnd", b(s.idle.stop_on_idle_end));
    w.leaf("RestartOnIdle", b(s.idle.restart_on_idle));
    w.close("IdleSettings");
}

fn network(w: &mut Writer, s: &Settings) {
    if has(&s.network.name) || has(&s.network.id) {
        w.open("NetworkSettings", &[]);
        opt(w, "Name", &s.network.name);
        opt(w, "Id", &s.network.id);
        w.close("NetworkSettings");
    }
}

fn restart(w: &mut Writer, s: &Settings) {
    if has(&s.restart_interval) {
        w.open("RestartOnFailure", &[]);
        opt(w, "Interval", &s.restart_interval);
        w.leaf("Count", &format!("{}", s.restart_count));
        w.close("RestartOnFailure");
    }
}

fn settings_full(w: &mut Writer, s: &Settings) {
    w.open("Settings", &[]);
    w.leaf("MultipleInstancesPolicy", policy(s.multiple_instances));
    w.leaf("DisallowStartIfOnBatteries", b(s.disallow_start_if_on_batteries));
    w.leaf("StopIfGoingOnBatteries", b(s.stop_if_going_on_batteries));
    w.leaf("AllowHardTerminate", b(s.allow_hard_terminate));
    w.leaf("StartWhenAvailable", b(s.start_when_available));
    w.leaf("RunOnlyIfNetworkAvailable", b(s.run_only_if_network_available));
    network(w, s);
    idle(w, s);
    w.leaf("AllowStartOnDemand", b(s.allow_demand_start));
    w.leaf("Enabled", b(s.enabled));
    w.leaf("Hidden", b(s.hidden));
    w.leaf("RunOnlyIfIdle", b(s.run_only_if_idle));
    if s.disallow_start_on_remote_app_session != 0 {
        w.leaf("DisallowStartOnRemoteAppSession", "true");
    }
    if s.use_unified_scheduling_engine != 0 {
        w.leaf("UseUnifiedSchedulingEngine", "true");
    }
    w.leaf("WakeToRun", b(s.wake_to_run));
    opt(w, "ExecutionTimeLimit", &s.execution_time_limit);
    opt(w, "DeleteExpiredTaskAfter", &s.delete_expired_task_after);
    w.leaf("Priority", &format!("{}", s.priority));
    restart(w, s);
    if s.volatile != 0 {
        w.leaf("Volatile", "true");
    }
    w.close("Settings");
}

fn settings_registered(w: &mut Writer, s: &Settings) {
    w.open("Settings", &[]);
    w.leaf("DisallowStartIfOnBatteries", b(s.disallow_start_if_on_batteries));
    w.leaf("StopIfGoingOnBatteries", b(s.stop_if_going_on_batteries));
    if s.allow_hard_terminate == 0 {
        w.leaf("AllowHardTerminate", "false");
    }
    if let Some(t) = s.execution_time_limit.as_deref().filter(|t| !t.is_empty() && *t != "PT72H") {
        w.leaf("ExecutionTimeLimit", t);
    }
    w.leaf("MultipleInstancesPolicy", policy(s.multiple_instances));
    if s.start_when_available != 0 {
        w.leaf("StartWhenAvailable", "true");
    }
    if s.run_only_if_network_available != 0 {
        w.leaf("RunOnlyIfNetworkAvailable", "true");
    }
    network(w, s);
    idle(w, s);
    if s.allow_demand_start == 0 {
        w.leaf("AllowStartOnDemand", "false");
    }
    if s.enabled == 0 {
        w.leaf("Enabled", "false");
    }
    if s.hidden != 0 {
        w.leaf("Hidden", "true");
    }
    if s.run_only_if_idle != 0 {
        w.leaf("RunOnlyIfIdle", "true");
    }
    if s.disallow_start_on_remote_app_session != 0 {
        w.leaf("DisallowStartOnRemoteAppSession", "true");
    }
    if s.use_unified_scheduling_engine != 0 {
        w.leaf("UseUnifiedSchedulingEngine", "true");
    }
    if s.wake_to_run != 0 {
        w.leaf("WakeToRun", "true");
    }
    opt(w, "DeleteExpiredTaskAfter", &s.delete_expired_task_after);
    if s.priority != 7 {
        w.leaf("Priority", &format!("{}", s.priority));
    }
    restart(w, s);
    if s.volatile != 0 {
        w.leaf("Volatile", "true");
    }
    w.close("Settings");
}

/// The element name of a trigger type; `None` for the custom type (nothing is written for it).
pub fn trigger_element(kind: i32) -> Option<&'static str> {
    Some(match kind {
        trigger::EVENT => "EventTrigger",
        trigger::TIME => "TimeTrigger",
        trigger::DAILY | trigger::WEEKLY | trigger::MONTHLY | trigger::MONTHLYDOW => "CalendarTrigger",
        trigger::IDLE => "IdleTrigger",
        trigger::REGISTRATION => "RegistrationTrigger",
        trigger::BOOT => "BootTrigger",
        trigger::LOGON => "LogonTrigger",
        trigger::SESSION_STATE_CHANGE => "SessionStateChangeTrigger",
        _ => return None,
    })
}

pub const DAY_NAMES: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
pub const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
pub const STATE_CHANGES: [&str; 9] = [
    "",
    "ConsoleConnect",
    "ConsoleDisconnect",
    "RemoteConnect",
    "RemoteDisconnect",
    "",
    "",
    "SessionLock",
    "SessionUnlock",
];

fn days_of_week(w: &mut Writer, bits: i16) {
    if bits & 0x7F == 0 {
        w.empty("DaysOfWeek", &[]);
        return;
    }
    w.open("DaysOfWeek", &[]);
    for (i, n) in DAY_NAMES.iter().enumerate() {
        if bits & (1 << i) != 0 {
            w.empty(n, &[]);
        }
    }
    w.close("DaysOfWeek");
}

fn months(w: &mut Writer, bits: i16) {
    if bits & 0x0FFF == 0 {
        w.empty("Months", &[]);
        return;
    }
    w.open("Months", &[]);
    for (i, n) in MONTH_NAMES.iter().enumerate() {
        if bits & (1 << i) != 0 {
            w.empty(n, &[]);
        }
    }
    w.close("Months");
}

fn weeks(w: &mut Writer, t: &Trigger) {
    w.open("Weeks", &[]);
    for i in 0..4 {
        if t.weeks_of_month & (1 << i) != 0 {
            w.leaf("Week", &format!("{}", i + 1));
        }
    }
    if t.run_on_last_week_of_month != 0 {
        w.leaf("Week", "Last");
    }
    w.close("Weeks");
}

fn days_of_month(w: &mut Writer, t: &Trigger) {
    if t.days_of_month == 0 && t.run_on_last_day_of_month == 0 {
        w.empty("DaysOfMonth", &[]);
        return;
    }
    w.open("DaysOfMonth", &[]);
    for i in 0..31 {
        if t.days_of_month & (1 << i) != 0 {
            w.leaf("Day", &format!("{}", i + 1));
        }
    }
    if t.run_on_last_day_of_month != 0 {
        w.leaf("Day", "Last");
    }
    w.close("DaysOfMonth");
}

fn repetition(w: &mut Writer, t: &Trigger) {
    if !t.repetition.is_set() {
        return;
    }
    w.open("Repetition", &[]);
    opt(w, "Interval", &t.repetition.interval);
    opt(w, "Duration", &t.repetition.duration);
    if t.repetition.stop_at_duration_end != 0 {
        w.leaf("StopAtDurationEnd", "true");
    }
    w.close("Repetition");
}

/// The children that belong to the trigger's type.
fn trigger_specific(w: &mut Writer, t: &Trigger, registered: bool) {
    match t.kind {
        trigger::EVENT => {
            opt(w, "Subscription", &t.subscription);
            opt(w, "Delay", &t.delay);
        }
        trigger::TIME => opt(w, "RandomDelay", &t.random_delay),
        trigger::DAILY => {
            w.open("ScheduleByDay", &[]);
            w.leaf("DaysInterval", &format!("{}", t.days_interval));
            w.close("ScheduleByDay");
            opt(w, "RandomDelay", &t.random_delay);
        }
        trigger::WEEKLY => {
            w.open("ScheduleByWeek", &[]);
            days_of_week(w, t.days_of_week);
            w.leaf("WeeksInterval", &format!("{}", t.weeks_interval));
            w.close("ScheduleByWeek");
            opt(w, "RandomDelay", &t.random_delay);
        }
        trigger::MONTHLY => {
            w.open("ScheduleByMonth", &[]);
            days_of_month(w, t);
            months(w, t.months_of_year);
            w.close("ScheduleByMonth");
            opt(w, "RandomDelay", &t.random_delay);
        }
        trigger::MONTHLYDOW => {
            w.open("ScheduleByMonthDayOfWeek", &[]);
            if registered {
                months(w, t.months_of_year);
                weeks(w, t);
                days_of_week(w, t.days_of_week);
            } else {
                weeks(w, t);
                days_of_week(w, t.days_of_week);
                months(w, t.months_of_year);
            }
            w.close("ScheduleByMonthDayOfWeek");
            opt(w, "RandomDelay", &t.random_delay);
        }
        trigger::REGISTRATION | trigger::BOOT => opt(w, "Delay", &t.delay),
        trigger::LOGON => {
            if let Some(u) = t.user_id.as_deref().filter(|s| !s.is_empty()) {
                w.leaf("UserId", &account::xml_form(u));
            }
            opt(w, "Delay", &t.delay);
        }
        trigger::SESSION_STATE_CHANGE => {
            if let Some(u) = t.user_id.as_deref().filter(|s| !s.is_empty()) {
                w.leaf("UserId", &account::xml_form(u));
            }
            opt(w, "Delay", &t.delay);
            if let Some(n) = STATE_CHANGES.get(t.state_change as usize).filter(|n| !n.is_empty()) {
                w.leaf("StateChange", n);
            }
        }
        _ => {}
    }
}

/// Whether the registered form of a trigger has any child (else it is `<Name />`).
fn registered_children(t: &Trigger) -> bool {
    let mut probe = Writer::new();
    trigger_specific(&mut probe, t, true);
    has(&t.start_boundary)
        || has(&t.end_boundary)
        || t.repetition.is_set()
        || has(&t.execution_time_limit)
        || t.enabled == 0
        || !probe.out.is_empty()
}

fn trigger_def(w: &mut Writer, t: &Trigger) {
    let Some(name) = trigger_element(t.kind) else { return };
    let id = t.id.as_deref().filter(|s| !s.is_empty());
    let attrs: &[(&str, &str)] = match &id {
        Some(i) => &[("id", i)],
        None => &[],
    };
    w.open(name, attrs);
    repetition(w, t);
    opt(w, "StartBoundary", &t.start_boundary);
    opt(w, "EndBoundary", &t.end_boundary);
    opt(w, "ExecutionTimeLimit", &t.execution_time_limit);
    w.leaf("Enabled", b(t.enabled));
    trigger_specific(w, t, false);
    w.close(name);
}

fn trigger_registered(w: &mut Writer, t: &Trigger) {
    let Some(name) = trigger_element(t.kind) else { return };
    let id = t.id.as_deref().filter(|s| !s.is_empty());
    let attrs: &[(&str, &str)] = match &id {
        Some(i) => &[("id", i)],
        None => &[],
    };
    if !registered_children(t) {
        w.empty(name, attrs);
        return;
    }
    w.open(name, attrs);
    opt(w, "StartBoundary", &t.start_boundary);
    opt(w, "EndBoundary", &t.end_boundary);
    repetition(w, t);
    opt(w, "ExecutionTimeLimit", &t.execution_time_limit);
    if t.enabled == 0 {
        w.leaf("Enabled", "false");
    }
    trigger_specific(w, t, true);
    w.close(name);
}

fn triggers(w: &mut Writer, d: &Definition, registered: bool) {
    if !d.triggers.iter().any(|t| trigger_element(t.kind).is_some()) {
        w.empty("Triggers", &[]);
        return;
    }
    w.open("Triggers", &[]);
    for t in &d.triggers {
        if registered { trigger_registered(w, t) } else { trigger_def(w, t) }
    }
    w.close("Triggers");
}

fn one_action(w: &mut Writer, a: &Action) {
    let id = a.id.as_deref().filter(|s| !s.is_empty());
    let attrs: &[(&str, &str)] = match &id {
        Some(i) => &[("id", i)],
        None => &[],
    };
    let (name, items): (&str, [(&str, &Text); 3]) = match a.kind {
        action::EXEC => {
            ("Exec", [("Command", &a.path), ("Arguments", &a.arguments), ("WorkingDirectory", &a.working_directory)])
        }
        action::COM_HANDLER => ("ComHandler", [("ClassId", &a.class_id), ("Data", &a.data), ("", &None)]),
        action::SHOW_MESSAGE => ("ShowMessage", [("Title", &a.title), ("Body", &a.message_body), ("", &None)]),
        _ => {
            w.open("SendEmail", attrs);
            for (n, t) in [
                ("Server", &a.server),
                ("Subject", &a.subject),
                ("To", &a.to),
                ("Cc", &a.cc),
                ("Bcc", &a.bcc),
                ("ReplyTo", &a.reply_to),
                ("From", &a.from),
            ] {
                opt(w, n, t);
            }
            w.empty("HeaderFields", &[]);
            opt(w, "Body", &a.body);
            w.empty("Attachments", &[]);
            w.close("SendEmail");
            return;
        }
    };
    if !items.iter().any(|(n, t)| !n.is_empty() && has(t)) {
        w.empty(name, attrs);
        return;
    }
    w.open(name, attrs);
    for (n, t) in items {
        if !n.is_empty() {
            opt(w, n, t);
        }
    }
    w.close(name);
}

fn actions(w: &mut Writer, d: &Definition, context: Option<&str>) {
    let attrs: &[(&str, &str)] = match &context {
        Some(c) => &[("Context", c)],
        None => &[],
    };
    if d.actions.is_empty() {
        w.empty("Actions", attrs);
        return;
    }
    w.open("Actions", attrs);
    for a in &d.actions {
        one_action(w, a);
    }
    w.close("Actions");
}

/// `ITaskDefinition::get_XmlText`.
pub fn definition_xml(d: &Definition) -> String {
    let mut w = Writer::new();
    begin(&mut w, d);
    registration(&mut w, &d.reg);
    triggers(&mut w, d, false);
    if principal_set(&d.principal) {
        principal(&mut w, &d.principal, d.principal.id.as_deref());
    }
    settings_full(&mut w, &d.settings);
    opt(&mut w, "Data", &d.data);
    actions(&mut w, d, d.context.as_deref());
    w.close("Task");
    w.finish()
}

/// `IRegisteredTask::get_Xml`: the stored form of a registered definition (its URI, principal and
/// context already filled in by the registration).
pub fn registered_xml(d: &Definition) -> String {
    let mut w = Writer::new();
    begin(&mut w, d);
    registration(&mut w, &d.reg);
    principal(&mut w, &d.principal, Some("Author"));
    settings_registered(&mut w, &d.settings);
    triggers(&mut w, d, true);
    opt(&mut w, "Data", &d.data);
    actions(&mut w, d, Some("Author"));
    w.close("Task");
    w.finish()
}
