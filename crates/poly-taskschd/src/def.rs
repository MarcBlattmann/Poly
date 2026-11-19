//! A task definition as data (the parts of `ITaskDefinition`), with the defaults of a new
//! definition (sheet `ITaskService.NewTask`, TASKSCHD.md section 4) and the few checks the setters
//! make (the rest is checked at registration, [`crate::parse::validate`]).
//!
//! Text properties are `Option<String>`: NULL and the empty string are different things to a
//! program. `VARIANT_BOOL`s keep the 16-bit value given (`put_Hidden(1)` reads back 1); anything but
//! 0 counts as true.

use crate::hr::{E_INVALIDARG, Hresult};
use alloc::string::String;
use alloc::vec::Vec;

pub type Text = Option<String>;

pub const VARIANT_TRUE: i16 = -1;
pub const VARIANT_FALSE: i16 = 0;

pub fn vb(b: bool) -> i16 {
    if b { VARIANT_TRUE } else { VARIANT_FALSE }
}

/// `TASK_TRIGGER_TYPE2` (taskschd.h).
pub mod trigger {
    pub const EVENT: i32 = 0;
    pub const TIME: i32 = 1;
    pub const DAILY: i32 = 2;
    pub const WEEKLY: i32 = 3;
    pub const MONTHLY: i32 = 4;
    pub const MONTHLYDOW: i32 = 5;
    pub const IDLE: i32 = 6;
    pub const REGISTRATION: i32 = 7;
    pub const BOOT: i32 = 8;
    pub const LOGON: i32 = 9;
    pub const SESSION_STATE_CHANGE: i32 = 11;
    pub const CUSTOM: i32 = 12;

    /// Whether `Create` accepts the type (sheet: 0 to 9, 11, 12).
    pub fn valid(t: i32) -> bool {
        matches!(t, 0..=9 | 11 | 12)
    }
}

/// `TASK_ACTION_TYPE` (taskschd.h).
pub mod action {
    pub const EXEC: i32 = 0;
    pub const COM_HANDLER: i32 = 5;
    pub const SEND_EMAIL: i32 = 6;
    pub const SHOW_MESSAGE: i32 = 7;

    pub fn valid(t: i32) -> bool {
        matches!(t, 0 | 5 | 6 | 7)
    }
}

/// `TASK_LOGON_TYPE` (taskschd.h).
pub mod logon {
    pub const NONE: i32 = 0;
    pub const PASSWORD: i32 = 1;
    pub const S4U: i32 = 2;
    pub const INTERACTIVE_TOKEN: i32 = 3;
    pub const GROUP: i32 = 4;
    pub const SERVICE_ACCOUNT: i32 = 5;
    pub const INTERACTIVE_TOKEN_OR_PASSWORD: i32 = 6;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistrationInfo {
    pub description: Text,
    pub author: Text,
    pub version: Text,
    pub date: Text,
    pub documentation: Text,
    pub uri: Text,
    pub source: Text,
    /// The SDDL text of `SecurityDescriptor` (a `VT_BSTR` variant), or `None` (an empty variant).
    pub security_descriptor: Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub id: Text,
    pub display_name: Text,
    pub user_id: Text,
    pub group_id: Text,
    pub logon_type: i32,
    pub run_level: i32,
    /// `IPrincipal2::ProcessTokenSidType` (2, default).
    pub process_token_sid_type: i32,
}

impl Default for Principal {
    fn default() -> Self {
        Principal {
            id: None,
            display_name: None,
            user_id: None,
            group_id: None,
            logon_type: logon::INTERACTIVE_TOKEN,
            run_level: 0,
            process_token_sid_type: 2,
        }
    }
}

impl Principal {
    /// `put_LogonType`: 0 is refused (sheet `IPrincipal.LogonType`), anything else is stored.
    pub fn set_logon_type(&mut self, v: i32) -> Hresult {
        if v == logon::NONE {
            return E_INVALIDARG;
        }
        self.logon_type = v;
        0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleSettings {
    pub duration: Text,
    pub wait_timeout: Text,
    pub stop_on_idle_end: i16,
    pub restart_on_idle: i16,
}

impl Default for IdleSettings {
    fn default() -> Self {
        IdleSettings {
            duration: Some(String::from("PT10M")),
            wait_timeout: Some(String::from("PT1H")),
            stop_on_idle_end: VARIANT_TRUE,
            restart_on_idle: VARIANT_FALSE,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkSettings {
    pub name: Text,
    pub id: Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub allow_demand_start: i16,
    pub restart_interval: Text,
    pub restart_count: i32,
    /// `TASK_INSTANCES_POLICY`: 0 parallel, 1 queue, 2 ignore new (default), 3 stop existing.
    pub multiple_instances: i32,
    pub stop_if_going_on_batteries: i16,
    pub disallow_start_if_on_batteries: i16,
    pub allow_hard_terminate: i16,
    pub start_when_available: i16,
    pub run_only_if_network_available: i16,
    pub execution_time_limit: Text,
    pub enabled: i16,
    pub delete_expired_task_after: Text,
    pub priority: i32,
    pub compatibility: i32,
    pub hidden: i16,
    pub idle: IdleSettings,
    pub run_only_if_idle: i16,
    pub wake_to_run: i16,
    pub network: NetworkSettings,
    pub disallow_start_on_remote_app_session: i16,
    pub use_unified_scheduling_engine: i16,
    pub volatile: i16,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            allow_demand_start: VARIANT_TRUE,
            restart_interval: None,
            restart_count: 0,
            multiple_instances: 2,
            stop_if_going_on_batteries: VARIANT_TRUE,
            disallow_start_if_on_batteries: VARIANT_TRUE,
            allow_hard_terminate: VARIANT_TRUE,
            start_when_available: VARIANT_FALSE,
            run_only_if_network_available: VARIANT_FALSE,
            execution_time_limit: Some(String::from("PT72H")),
            enabled: VARIANT_TRUE,
            delete_expired_task_after: None,
            priority: 7,
            compatibility: 2,
            hidden: VARIANT_FALSE,
            idle: IdleSettings::default(),
            run_only_if_idle: VARIANT_FALSE,
            wake_to_run: VARIANT_FALSE,
            network: NetworkSettings::default(),
            disallow_start_on_remote_app_session: VARIANT_FALSE,
            use_unified_scheduling_engine: VARIANT_FALSE,
            volatile: VARIANT_FALSE,
        }
    }
}

impl Settings {
    /// `put_Priority`: 0 to 10 (sheet: 11 and -1 refused, the value stays).
    pub fn set_priority(&mut self, v: i32) -> Hresult {
        if !(0..=10).contains(&v) {
            return E_INVALIDARG;
        }
        self.priority = v;
        0
    }

    /// `put_Compatibility`: 0 to 6 and -1 (sheet: 7 refused).
    pub fn set_compatibility(&mut self, v: i32) -> Hresult {
        if !(-1..=6).contains(&v) {
            return E_INVALIDARG;
        }
        self.compatibility = v;
        0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repetition {
    pub interval: Text,
    pub duration: Text,
    pub stop_at_duration_end: i16,
}

impl Repetition {
    pub fn is_set(&self) -> bool {
        self.interval.as_deref().is_some_and(|s| !s.is_empty())
            || self.duration.as_deref().is_some_and(|s| !s.is_empty())
            || self.stop_at_duration_end != 0
    }
}

/// One trigger of any type: the fields a type has no use for stay at their defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub kind: i32,
    pub id: Text,
    pub start_boundary: Text,
    pub end_boundary: Text,
    pub execution_time_limit: Text,
    pub enabled: i16,
    pub repetition: Repetition,
    /// Daily: `DaysInterval` (a SHORT, 1 by default).
    pub days_interval: i16,
    /// Weekly: `WeeksInterval` (1) and `DaysOfWeek` (bit 0 Sunday … bit 6 Saturday).
    pub weeks_interval: i16,
    pub days_of_week: i16,
    /// Monthly: `DaysOfMonth` (bit 0 the 1st … bit 30 the 31st), `MonthsOfYear` (bit 0 January),
    /// `RunOnLastDayOfMonth`.
    pub days_of_month: i32,
    pub months_of_year: i16,
    pub run_on_last_day_of_month: i16,
    /// Monthly day of week: `WeeksOfMonth` (bit 0 the first week), `RunOnLastWeekOfMonth`.
    pub weeks_of_month: i16,
    pub run_on_last_week_of_month: i16,
    pub random_delay: Text,
    pub delay: Text,
    pub user_id: Text,
    /// Event: `Subscription`.
    pub subscription: Text,
    /// Session state change: `TASK_SESSION_STATE_CHANGE_TYPE` (1 console connect).
    pub state_change: i32,
}

impl Trigger {
    pub fn new(kind: i32) -> Trigger {
        let (dow, moy, wom) = match kind {
            trigger::MONTHLY => (0, 0x0FFF, 0),
            trigger::MONTHLYDOW => (1, 1, 1),
            _ => (0, 0, 0),
        };
        Trigger {
            kind,
            id: None,
            start_boundary: None,
            end_boundary: None,
            execution_time_limit: None,
            enabled: VARIANT_TRUE,
            repetition: Repetition::default(),
            days_interval: 1,
            weeks_interval: 1,
            days_of_week: dow,
            days_of_month: 0,
            months_of_year: moy,
            run_on_last_day_of_month: 0,
            weeks_of_month: wom,
            run_on_last_week_of_month: 0,
            random_delay: None,
            delay: None,
            user_id: None,
            subscription: None,
            state_change: if kind == trigger::SESSION_STATE_CHANGE { 1 } else { 0 },
        }
    }

    /// `IDailyTrigger::put_DaysInterval`: 1 or more (sheet: 0 and -1 refused, the value stays).
    pub fn set_days_interval(&mut self, v: i16) -> Hresult {
        if v < 1 {
            return E_INVALIDARG;
        }
        self.days_interval = v;
        0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub kind: i32,
    pub id: Text,
    /// Exec: `Path`, `Arguments`, `WorkingDirectory`.
    pub path: Text,
    pub arguments: Text,
    pub working_directory: Text,
    /// Exec2: `HideAppWindow`.
    pub hide_app_window: i16,
    /// COM handler: `ClassId`, `Data`.
    pub class_id: Text,
    pub data: Text,
    /// E-mail and message: the texts.
    pub server: Text,
    pub subject: Text,
    pub to: Text,
    pub cc: Text,
    pub bcc: Text,
    pub reply_to: Text,
    pub from: Text,
    pub body: Text,
    pub title: Text,
    pub message_body: Text,
}

impl Action {
    pub fn new(kind: i32) -> Action {
        Action {
            kind,
            id: None,
            path: None,
            arguments: None,
            working_directory: None,
            hide_app_window: 0,
            class_id: None,
            data: None,
            server: None,
            subject: None,
            to: None,
            cc: None,
            bcc: None,
            reply_to: None,
            from: None,
            body: None,
            title: None,
            message_body: None,
        }
    }
}

/// A whole definition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Definition {
    pub reg: RegistrationInfo,
    pub principal: Principal,
    pub settings: Settings,
    pub triggers: Vec<Trigger>,
    pub actions: Vec<Action>,
    /// `IActionCollection::Context`.
    pub context: Text,
    /// `ITaskDefinition::Data`.
    pub data: Text,
}

/// Whether an optional text is set and not empty (an empty text is never written).
pub fn has(t: &Text) -> bool {
    t.as_deref().is_some_and(|s| !s.is_empty())
}
