//! A task XML document into a [`Definition`] (`ITaskDefinition::put_XmlText`,
//! `ITaskFolder::RegisterTask`, the scheduler reading its files back), and the checks a definition
//! passes at registration ([`validate`]).
//!
//! The errors are the sheets' (`ITaskService.NewTask`, `ITaskFolder.RegisterTask`,
//! `ITaskFolder.RegisterTaskDefinition`): not XML, an empty text or a UTF-8 declaration
//! `0x8004131A`; a root other than `Task` or an element that is not allowed `0x80041316`; another
//! namespace `0x80041317`; a value that is not valid `0x80041318`; a required element missing
//! `0x80041319`.

use crate::account;
use crate::def::{Action, Definition, Settings, Trigger, action, has, logon, trigger};
use crate::hr::*;
use crate::time::{parse_datetime, parse_duration};
use crate::write::{DAY_NAMES, MONTH_NAMES, NAMESPACE, STATE_CHANGES};
use crate::xml::{self, Element};
use alloc::string::String;

type R<T> = Result<T, Hresult>;

fn bad<T>() -> R<T> {
    Err(SCHED_E_UNEXPECTEDNODE)
}

fn text(e: &Element) -> String {
    String::from(e.text.trim())
}

fn boolean(e: &Element) -> R<i16> {
    match e.text.trim() {
        "true" | "1" => Ok(-1),
        "false" | "0" => Ok(0),
        _ => Err(SCHED_E_INVALIDVALUE),
    }
}

fn int(e: &Element) -> R<i32> {
    e.text.trim().parse::<i32>().map_err(|_| SCHED_E_INVALIDVALUE)
}

fn duration(e: &Element) -> R<String> {
    let t = text(e);
    parse_duration(&t).ok_or(SCHED_E_INVALIDVALUE)?;
    Ok(t)
}

fn datetime(e: &Element) -> R<String> {
    let t = text(e);
    parse_datetime(&t).ok_or(SCHED_E_INVALIDVALUE)?;
    Ok(t)
}

/// The schema versions accepted: 1.1 to 1.6 (and none at all); 1.0 and an empty version are refused
/// as elements that are not allowed, a higher or unreadable one as a value that is not valid.
fn check_version(v: Option<&str>) -> R<()> {
    let Some(v) = v else { return Ok(()) };
    match v {
        "1.1" | "1.2" | "1.3" | "1.4" | "1.5" | "1.6" => Ok(()),
        "1.0" | "" => Err(SCHED_E_UNEXPECTEDNODE),
        _ => Err(SCHED_E_INVALIDVALUE),
    }
}

/// Parse a whole task document.
pub fn parse_task(s: &str) -> R<Definition> {
    if s.trim().is_empty() {
        return Err(SCHED_E_MALFORMEDXML);
    }
    let doc = xml::parse(s).ok_or(SCHED_E_MALFORMEDXML)?;
    if let Some(enc) = &doc.encoding
        && !enc.eq_ignore_ascii_case("UTF-16")
    {
        return Err(SCHED_E_MALFORMEDXML);
    }
    let root = &doc.root;
    if root.name != "Task" {
        return Err(SCHED_E_UNEXPECTEDNODE);
    }
    if root.attr("xmlns") != Some(NAMESPACE) {
        return Err(SCHED_E_NAMESPACE);
    }
    check_version(root.attr("version"))?;
    let mut d = Definition::default();
    // A document that names no idle duration or wait timeout has none (sheet: an `IdleSettings`
    // without them loses both, and a document with no settings at all too).
    d.settings.idle.duration = None;
    d.settings.idle.wait_timeout = None;
    for c in &root.children {
        match c.name.as_str() {
            "RegistrationInfo" => registration(c, &mut d)?,
            "Triggers" => triggers(c, &mut d)?,
            "Settings" => settings(c, &mut d.settings)?,
            "Data" => d.data = Some(text(c)),
            "Principals" => principals(c, &mut d)?,
            "Actions" => actions(c, &mut d)?,
            _ => return bad(),
        }
    }
    Ok(d)
}

fn registration(e: &Element, d: &mut Definition) -> R<()> {
    for c in &e.children {
        let v = Some(text(c));
        match c.name.as_str() {
            "Description" => d.reg.description = v,
            "Author" => d.reg.author = v,
            "Version" => d.reg.version = v,
            "Date" => d.reg.date = v,
            "Documentation" => d.reg.documentation = v,
            "URI" => d.reg.uri = v,
            "Source" => d.reg.source = v,
            "SecurityDescriptor" => d.reg.security_descriptor = v,
            _ => return bad(),
        }
    }
    Ok(())
}

fn principals(e: &Element, d: &mut Definition) -> R<()> {
    for p in &e.children {
        if p.name != "Principal" {
            return bad();
        }
        d.principal.id = p.attr("id").map(String::from);
        let mut lt = None;
        for c in &p.children {
            match c.name.as_str() {
                "UserId" => d.principal.user_id = Some(account::canonical(&text(c))),
                "GroupId" => d.principal.group_id = Some(account::canonical(&text(c))),
                "DisplayName" => d.principal.display_name = Some(text(c)),
                "LogonType" => {
                    lt = Some(match c.text.trim() {
                        "Password" => logon::PASSWORD,
                        "S4U" => logon::S4U,
                        "InteractiveToken" => logon::INTERACTIVE_TOKEN,
                        "Group" => logon::GROUP,
                        "ServiceAccount" => logon::SERVICE_ACCOUNT,
                        "InteractiveTokenOrPassword" => logon::INTERACTIVE_TOKEN_OR_PASSWORD,
                        _ => return Err(SCHED_E_INVALIDVALUE),
                    })
                }
                "RunLevel" => {
                    d.principal.run_level = match c.text.trim() {
                        "HighestAvailable" => 1,
                        "LeastPrivilege" => 0,
                        _ => return Err(SCHED_E_INVALIDVALUE),
                    }
                }
                "ProcessTokenSidType" | "RequiredPrivileges" => {}
                _ => return bad(),
            }
        }
        if let Some(lt) = lt {
            d.principal.logon_type = lt;
        } else if d.principal.user_id.as_deref().is_some_and(account::is_service_account) {
            d.principal.logon_type = logon::SERVICE_ACCOUNT;
        }
    }
    Ok(())
}

fn settings(e: &Element, s: &mut Settings) -> R<()> {
    for c in &e.children {
        match c.name.as_str() {
            "AllowStartOnDemand" => s.allow_demand_start = boolean(c)?,
            "RestartOnFailure" => {
                for r in &c.children {
                    match r.name.as_str() {
                        "Interval" => s.restart_interval = Some(duration(r)?),
                        "Count" => s.restart_count = int(r)?,
                        _ => return bad(),
                    }
                }
            }
            "MultipleInstancesPolicy" => {
                s.multiple_instances = match c.text.trim() {
                    "Parallel" => 0,
                    "Queue" => 1,
                    "IgnoreNew" => 2,
                    "StopExisting" => 3,
                    _ => return Err(SCHED_E_INVALIDVALUE),
                }
            }
            "StopIfGoingOnBatteries" => s.stop_if_going_on_batteries = boolean(c)?,
            "DisallowStartIfOnBatteries" => s.disallow_start_if_on_batteries = boolean(c)?,
            "AllowHardTerminate" => s.allow_hard_terminate = boolean(c)?,
            "StartWhenAvailable" => s.start_when_available = boolean(c)?,
            "RunOnlyIfNetworkAvailable" => s.run_only_if_network_available = boolean(c)?,
            "ExecutionTimeLimit" => s.execution_time_limit = Some(duration(c)?),
            "Enabled" => s.enabled = boolean(c)?,
            "DeleteExpiredTaskAfter" => s.delete_expired_task_after = Some(duration(c)?),
            "Priority" => {
                let p = int(c)?;
                if !(0..=10).contains(&p) {
                    return Err(SCHED_E_INVALIDVALUE);
                }
                s.priority = p;
            }
            "Hidden" => s.hidden = boolean(c)?,
            "RunOnlyIfIdle" => s.run_only_if_idle = boolean(c)?,
            "WakeToRun" => s.wake_to_run = boolean(c)?,
            "DisallowStartOnRemoteAppSession" => s.disallow_start_on_remote_app_session = boolean(c)?,
            "UseUnifiedSchedulingEngine" => s.use_unified_scheduling_engine = boolean(c)?,
            "Volatile" => s.volatile = boolean(c)?,
            "IdleSettings" => {
                for i in &c.children {
                    match i.name.as_str() {
                        "Duration" => s.idle.duration = Some(duration(i)?),
                        "WaitTimeout" => s.idle.wait_timeout = Some(duration(i)?),
                        "StopOnIdleEnd" => s.idle.stop_on_idle_end = boolean(i)?,
                        "RestartOnIdle" => s.idle.restart_on_idle = boolean(i)?,
                        _ => return bad(),
                    }
                }
            }
            "NetworkSettings" => {
                for n in &c.children {
                    match n.name.as_str() {
                        "Name" => s.network.name = Some(text(n)),
                        "Id" => s.network.id = Some(text(n)),
                        _ => return bad(),
                    }
                }
            }
            "MaintenanceSettings" => {}
            _ => return bad(),
        }
    }
    Ok(())
}

fn day_bits(e: &Element) -> R<i16> {
    let mut bits = 0i16;
    for c in &e.children {
        let i = DAY_NAMES.iter().position(|n| *n == c.name).ok_or(SCHED_E_UNEXPECTEDNODE)?;
        bits |= 1 << i;
    }
    Ok(bits)
}

fn month_bits(e: &Element) -> R<i16> {
    let mut bits = 0i16;
    for c in &e.children {
        let i = MONTH_NAMES.iter().position(|n| *n == c.name).ok_or(SCHED_E_UNEXPECTEDNODE)?;
        bits |= 1 << i;
    }
    Ok(bits)
}

fn schedule(e: &Element, t: &mut Trigger) -> R<()> {
    match e.name.as_str() {
        "ScheduleByDay" => {
            t.kind = trigger::DAILY;
            for c in &e.children {
                match c.name.as_str() {
                    "DaysInterval" => t.days_interval = int(c)?.clamp(1, i16::MAX.into()) as i16,
                    _ => return bad(),
                }
            }
        }
        "ScheduleByWeek" => {
            t.kind = trigger::WEEKLY;
            t.days_of_week = 0;
            for c in &e.children {
                match c.name.as_str() {
                    "DaysOfWeek" => t.days_of_week = day_bits(c)?,
                    "WeeksInterval" => t.weeks_interval = int(c)? as i16,
                    _ => return bad(),
                }
            }
        }
        "ScheduleByMonth" => {
            t.kind = trigger::MONTHLY;
            t.months_of_year = 0;
            for c in &e.children {
                match c.name.as_str() {
                    "DaysOfMonth" => {
                        for day in &c.children {
                            match day.text.trim() {
                                "Last" => t.run_on_last_day_of_month = -1,
                                n => {
                                    let n = n.parse::<u32>().map_err(|_| SCHED_E_INVALIDVALUE)?;
                                    if !(1..=31).contains(&n) {
                                        return Err(SCHED_E_INVALIDVALUE);
                                    }
                                    t.days_of_month |= 1 << (n - 1);
                                }
                            }
                        }
                    }
                    "Months" => t.months_of_year = month_bits(c)?,
                    _ => return bad(),
                }
            }
        }
        "ScheduleByMonthDayOfWeek" => {
            t.kind = trigger::MONTHLYDOW;
            t.months_of_year = 0;
            t.days_of_week = 0;
            t.weeks_of_month = 0;
            for c in &e.children {
                match c.name.as_str() {
                    "Weeks" => {
                        for wk in &c.children {
                            match wk.text.trim() {
                                "Last" => t.run_on_last_week_of_month = -1,
                                n => {
                                    let n = n.parse::<u32>().map_err(|_| SCHED_E_INVALIDVALUE)?;
                                    if !(1..=4).contains(&n) {
                                        return Err(SCHED_E_INVALIDVALUE);
                                    }
                                    t.weeks_of_month |= 1 << (n - 1);
                                }
                            }
                        }
                    }
                    "DaysOfWeek" => t.days_of_week = day_bits(c)?,
                    "Months" => t.months_of_year = month_bits(c)?,
                    _ => return bad(),
                }
            }
        }
        _ => return bad(),
    }
    Ok(())
}

fn triggers(e: &Element, d: &mut Definition) -> R<()> {
    for c in &e.children {
        let kind = match c.name.as_str() {
            "EventTrigger" => trigger::EVENT,
            "TimeTrigger" => trigger::TIME,
            "CalendarTrigger" => trigger::DAILY,
            "IdleTrigger" => trigger::IDLE,
            "RegistrationTrigger" => trigger::REGISTRATION,
            "BootTrigger" => trigger::BOOT,
            "LogonTrigger" => trigger::LOGON,
            "SessionStateChangeTrigger" => trigger::SESSION_STATE_CHANGE,
            _ => return bad(),
        };
        let mut t = Trigger::new(kind);
        t.id = c.attr("id").map(String::from);
        let mut calendar_schedule = false;
        for p in &c.children {
            match p.name.as_str() {
                "StartBoundary" => t.start_boundary = Some(datetime(p)?),
                "EndBoundary" => t.end_boundary = Some(datetime(p)?),
                "ExecutionTimeLimit" => t.execution_time_limit = Some(duration(p)?),
                "Enabled" => t.enabled = boolean(p)?,
                "Repetition" => {
                    for r in &p.children {
                        match r.name.as_str() {
                            "Interval" => t.repetition.interval = Some(duration(r)?),
                            "Duration" => t.repetition.duration = Some(duration(r)?),
                            "StopAtDurationEnd" => t.repetition.stop_at_duration_end = boolean(r)?,
                            _ => return bad(),
                        }
                    }
                }
                "RandomDelay" => t.random_delay = Some(duration(p)?),
                "Delay" => t.delay = Some(duration(p)?),
                "UserId" => t.user_id = Some(account::canonical(&text(p))),
                "Subscription" if kind == trigger::EVENT => t.subscription = Some(text(p)),
                "ValueQueries" if kind == trigger::EVENT => {}
                "StateChange" if kind == trigger::SESSION_STATE_CHANGE => {
                    t.state_change = STATE_CHANGES
                        .iter()
                        .position(|n| !n.is_empty() && *n == p.text.trim())
                        .ok_or(SCHED_E_INVALIDVALUE)? as i32;
                }
                n if kind == trigger::DAILY && n.starts_with("Schedule") => {
                    schedule(p, &mut t)?;
                    calendar_schedule = true;
                }
                _ => return bad(),
            }
        }
        if kind == trigger::DAILY && !calendar_schedule {
            return Err(SCHED_E_MISSINGNODE);
        }
        d.triggers.push(t);
    }
    Ok(())
}

fn actions(e: &Element, d: &mut Definition) -> R<()> {
    d.context = e.attr("Context").map(String::from);
    for c in &e.children {
        let kind = match c.name.as_str() {
            "Exec" => action::EXEC,
            "ComHandler" => action::COM_HANDLER,
            "SendEmail" => action::SEND_EMAIL,
            "ShowMessage" => action::SHOW_MESSAGE,
            _ => return bad(),
        };
        let mut a = Action::new(kind);
        a.id = c.attr("id").map(String::from);
        for p in &c.children {
            let v = Some(text(p));
            match (kind, p.name.as_str()) {
                (action::EXEC, "Command") => a.path = v,
                (action::EXEC, "Arguments") => a.arguments = v,
                (action::EXEC, "WorkingDirectory") => a.working_directory = v,
                (action::COM_HANDLER, "ClassId") => a.class_id = v,
                (action::COM_HANDLER, "Data") => a.data = v,
                (action::SHOW_MESSAGE, "Title") => a.title = v,
                (action::SHOW_MESSAGE, "Body") => a.message_body = v,
                (action::SEND_EMAIL, "Server") => a.server = v,
                (action::SEND_EMAIL, "Subject") => a.subject = v,
                (action::SEND_EMAIL, "To") => a.to = v,
                (action::SEND_EMAIL, "Cc") => a.cc = v,
                (action::SEND_EMAIL, "Bcc") => a.bcc = v,
                (action::SEND_EMAIL, "ReplyTo") => a.reply_to = v,
                (action::SEND_EMAIL, "From") => a.from = v,
                (action::SEND_EMAIL, "Body") => a.body = v,
                (action::SEND_EMAIL, "HeaderFields" | "Attachments") => {}
                _ => return bad(),
            }
        }
        d.actions.push(a);
    }
    Ok(())
}

fn check_duration(t: &Option<String>) -> R<()> {
    if let Some(s) = t.as_deref().filter(|s| !s.is_empty()) {
        parse_duration(s).ok_or(SCHED_E_INVALIDVALUE)?;
    }
    Ok(())
}

fn check_date(t: &Option<String>) -> R<()> {
    if let Some(s) = t.as_deref().filter(|s| !s.is_empty()) {
        parse_datetime(s).ok_or(SCHED_E_INVALIDVALUE)?;
    }
    Ok(())
}

/// What a definition must have to be registered (the setters stored anything; registration checks).
pub fn validate(d: &Definition) -> R<()> {
    if d.actions.is_empty() {
        return Err(SCHED_E_MISSINGNODE);
    }
    for a in &d.actions {
        if a.kind == action::EXEC && !has(&a.path) {
            return Err(SCHED_E_MISSINGNODE);
        }
        if a.kind == action::COM_HANDLER && !has(&a.class_id) {
            return Err(SCHED_E_MISSINGNODE);
        }
    }
    for t in &d.triggers {
        match t.kind {
            trigger::WEEKLY if t.days_of_week & 0x7F == 0 => return Err(SCHED_E_MISSINGNODE),
            trigger::MONTHLY if t.days_of_month == 0 && t.run_on_last_day_of_month == 0 => {
                return Err(SCHED_E_MISSINGNODE);
            }
            _ => {}
        }
        if matches!(t.kind, trigger::TIME | trigger::DAILY | trigger::WEEKLY | trigger::MONTHLY | trigger::MONTHLYDOW)
            && !has(&t.start_boundary)
        {
            return Err(SCHED_E_MISSINGNODE);
        }
        check_date(&t.start_boundary)?;
        check_date(&t.end_boundary)?;
        check_duration(&t.execution_time_limit)?;
        check_duration(&t.random_delay)?;
        check_duration(&t.delay)?;
        check_duration(&t.repetition.duration)?;
        if let Some(i) = t.repetition.interval.as_deref().filter(|s| !s.is_empty()) {
            // Learn ("IRepetitionPattern::Interval"): one minute to 31 days.
            let secs = parse_duration(i).ok_or(SCHED_E_INVALIDVALUE)?;
            if !(60..=31 * 86_400).contains(&secs) {
                return Err(SCHED_E_INVALIDVALUE);
            }
        }
    }
    let s = &d.settings;
    check_duration(&s.execution_time_limit)?;
    check_duration(&s.delete_expired_task_after)?;
    check_duration(&s.restart_interval)?;
    check_duration(&s.idle.duration)?;
    check_duration(&s.idle.wait_timeout)?;
    Ok(())
}
