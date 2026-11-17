//! Task definitions (`ITaskDefinition` and its parts), which live in the calling process.
//!
//! A definition's parts are shared cells, so a part object stays valid on its own (a removed
//! trigger's object still answers) and every change shows at once in the definition's
//! `XmlText`. The definition object keeps one object per part and hands out the same pointer on
//! every call (sheet `ITaskService.NewTask`), as a trigger keeps its repetition pattern. The setters
//! store what they are given; only the checks of `poly_taskschd::def` are made here, the rest at
//! registration.

use crate::*;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_taskschd::account;
use poly_taskschd::def::*;
use poly_taskschd::parse::parse_task;
use poly_taskschd::write::definition_xml;

pub type C<T> = Arc<Lock<T>>;

fn cell<T>(v: T) -> C<T> {
    Arc::new(Lock::new(v))
}

#[derive(Default)]
pub struct Actions {
    pub context: Text,
    pub items: Vec<C<Action>>,
}

/// The parts of one definition.
pub struct DefState {
    pub reg: C<RegistrationInfo>,
    pub principal: C<Principal>,
    pub settings: C<Settings>,
    pub triggers: C<Vec<C<Trigger>>>,
    pub actions: C<Actions>,
    pub data: C<Text>,
}

impl DefState {
    fn new(d: Definition) -> DefState {
        let s = DefState {
            reg: cell(RegistrationInfo::default()),
            principal: cell(Principal::default()),
            settings: cell(Settings::default()),
            triggers: cell(Vec::new()),
            actions: cell(Actions::default()),
            data: cell(None),
        };
        s.load(d);
        s
    }

    /// Replace the contents of every part (the part objects stay).
    fn load(&self, d: Definition) {
        self.reg.with(|r| *r = d.reg);
        self.principal.with(|p| *p = d.principal);
        self.settings.with(|s| *s = d.settings);
        self.triggers.with(|t| *t = d.triggers.into_iter().map(cell).collect());
        self.actions.with(|a| {
            a.context = d.context;
            a.items = d.actions.into_iter().map(cell).collect();
        });
        self.data.with(|x| *x = d.data);
    }

    fn snapshot(&self) -> Definition {
        let triggers: Vec<C<Trigger>> = self.triggers.with(|t| t.clone());
        let (context, items): (Text, Vec<C<Action>>) = self.actions.with(|a| (a.context.clone(), a.items.clone()));
        Definition {
            reg: self.reg.with(|r| r.clone()),
            principal: self.principal.with(|p| p.clone()),
            settings: self.settings.with(|s| s.clone()),
            triggers: triggers.iter().map(|t| t.with(|t| t.clone())).collect(),
            actions: items.iter().map(|a| a.with(|a| a.clone())).collect(),
            context,
            data: self.data.with(|x| x.clone()),
        }
    }
}

// ------------------------------------------------------------------------------- property macros

/// A text property `get_X(BSTR*)` / `put_X(BSTR)` of an object whose data has the cell `c`.
macro_rules! text {
    ($obj:ty, $get:ident, $put:ident, |$x:ident| $f:expr) => {
        text!($obj, $get, $put, |$x| $f, |v| v);
    };
    ($obj:ty, $get:ident, $put:ident, |$x:ident| $f:expr, |$v:ident| $conv:expr) => {
        unsafe extern "system" fn $get(this: *mut c_void, out: *mut Bstr) -> HRESULT {
            // SAFETY: an object of this type.
            let o = unsafe { data::<$obj>(this) };
            let v = o.c.with(|$x| $f.clone());
            // SAFETY: the caller's out parameter.
            unsafe { put_text(out, &v) }
        }
        unsafe extern "system" fn $put(this: *mut c_void, b: Bstr) -> HRESULT {
            // SAFETY: an object of this type.
            let o = unsafe { data::<$obj>(this) };
            let $v = from_bstr(b);
            let s = $conv;
            o.c.with(|$x| $f = s);
            S_OK
        }
    };
}

/// A `VARIANT_BOOL` property.
macro_rules! boolean {
    ($obj:ty, $get:ident, $put:ident, |$x:ident| $f:expr) => {
        unsafe extern "system" fn $get(this: *mut c_void, out: *mut i16) -> HRESULT {
            // SAFETY: an object of this type.
            let o = unsafe { data::<$obj>(this) };
            let v = o.c.with(|$x| $f);
            // SAFETY: the caller's out parameter.
            unsafe { put_bool(out, v) }
        }
        unsafe extern "system" fn $put(this: *mut c_void, v: i16) -> HRESULT {
            // SAFETY: an object of this type.
            let o = unsafe { data::<$obj>(this) };
            o.c.with(|$x| $f = v);
            S_OK
        }
    };
}

/// A number property (`LONG`, `INT`, an enumeration, or `SHORT`), with an optional setter check.
macro_rules! number {
    ($obj:ty, $t:ty, $get:ident, $put:ident, |$x:ident| $f:expr) => {
        number!($obj, $t, $get, $put, |$x| $f, |$x, v| {
            $f = v;
            S_OK
        });
    };
    ($obj:ty, $t:ty, $get:ident, $put:ident, |$x:ident| $f:expr, |$y:ident, $v:ident| $set:expr) => {
        unsafe extern "system" fn $get(this: *mut c_void, out: *mut $t) -> HRESULT {
            // SAFETY: an object of this type.
            let o = unsafe { data::<$obj>(this) };
            let v = o.c.with(|$x| $f);
            // SAFETY: the caller's out parameter.
            unsafe { put_val(out, v) }
        }
        unsafe extern "system" fn $put(this: *mut c_void, $v: $t) -> HRESULT {
            // SAFETY: an object of this type.
            let o = unsafe { data::<$obj>(this) };
            o.c.with(|$y| $set)
        }
    };
}

unsafe extern "system" fn xml_not_impl(_this: *mut c_void, _x: *mut c_void) -> HRESULT {
    E_NOTIMPL
}

// ------------------------------------------------------------------------------- registration info

struct RegObj {
    c: C<RegistrationInfo>,
}

text!(RegObj, reg_get_description, reg_put_description, |x| x.description);
text!(RegObj, reg_get_author, reg_put_author, |x| x.author);
text!(RegObj, reg_get_version, reg_put_version, |x| x.version);
text!(RegObj, reg_get_date, reg_put_date, |x| x.date);
text!(RegObj, reg_get_documentation, reg_put_documentation, |x| x.documentation);
text!(RegObj, reg_get_uri, reg_put_uri, |x| x.uri);
text!(RegObj, reg_get_source, reg_put_source, |x| x.source);

unsafe extern "system" fn reg_get_sd(this: *mut c_void, out: *mut Variant) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a registration info object.
    let o = unsafe { data::<RegObj>(this) };
    let v = o.c.with(|x| x.security_descriptor.clone());
    let var = match v {
        Some(s) => Variant { vt: VT_BSTR, r: [0; 3], v: [bstr(&s) as usize, 0] },
        None => Variant { vt: VT_EMPTY, r: [0; 3], v: [0; 2] },
    };
    // SAFETY: the caller's out parameter.
    unsafe { *out = var };
    S_OK
}

unsafe extern "system" fn reg_put_sd(this: *mut c_void, v: Variant) -> HRESULT {
    // SAFETY: a registration info object.
    let o = unsafe { data::<RegObj>(this) };
    let s = match v.text() {
        Some(t) => t,
        None if v.is_empty() => None,
        None => return E_INVALIDARG,
    };
    o.c.with(|x| x.security_descriptor = s);
    S_OK
}

static REG_VTBL: Vt<25> = dual![
    reg_get_description,
    reg_put_description,
    reg_get_author,
    reg_put_author,
    reg_get_version,
    reg_put_version,
    reg_get_date,
    reg_put_date,
    reg_get_documentation,
    reg_put_documentation,
    xml_not_impl,
    xml_not_impl,
    reg_get_uri,
    reg_put_uri,
    reg_get_sd,
    reg_put_sd,
    reg_get_source,
    reg_put_source,
];
static REG_IIDS: &[Guid] = &[IID_IREGISTRATIONINFO];

// ------------------------------------------------------------------------------- principal

struct PrinObj {
    c: C<Principal>,
}

text!(PrinObj, pr_get_id, pr_put_id, |x| x.id);
text!(PrinObj, pr_get_display, pr_put_display, |x| x.display_name);
text!(PrinObj, pr_get_user, pr_put_user, |x| x.user_id, |v| v.map(|s| account::canonical(&s)));
text!(PrinObj, pr_get_group, pr_put_group, |x| x.group_id, |v| v.map(|s| account::canonical(&s)));
number!(PrinObj, i32, pr_get_logon, pr_put_logon, |x| x.logon_type, |x, v| x.set_logon_type(v));
number!(PrinObj, i32, pr_get_runlevel, pr_put_runlevel, |x| x.run_level);

unsafe extern "system" fn pr2_get_sid_type(this: *mut c_void, out: *mut i32) -> HRESULT {
    // SAFETY: the second pointer of a principal object.
    let o = unsafe { data::<PrinObj>(primary(this)) };
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, o.c.with(|x| x.process_token_sid_type)) }
}

unsafe extern "system" fn pr2_put_sid_type(this: *mut c_void, v: i32) -> HRESULT {
    // SAFETY: as above.
    let o = unsafe { data::<PrinObj>(primary(this)) };
    o.c.with(|x| x.process_token_sid_type = v);
    S_OK
}

unsafe extern "system" fn pr2_priv_count(_this: *mut c_void, out: *mut i32) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, 0) }
}

unsafe extern "system" fn pr2_get_priv(_this: *mut c_void, _i: i32, _out: *mut Bstr) -> HRESULT {
    E_INVALIDARG
}

unsafe extern "system" fn pr2_add_priv(_this: *mut c_void, _p: Bstr) -> HRESULT {
    S_OK
}

static PRIN_VTBL: Vt<19> = dual![
    pr_get_id,
    pr_put_id,
    pr_get_display,
    pr_put_display,
    pr_get_user,
    pr_put_user,
    pr_get_logon,
    pr_put_logon,
    pr_get_group,
    pr_put_group,
    pr_get_runlevel,
    pr_put_runlevel,
];
static PRIN2_VTBL: Vt<12> = dual2![pr2_get_sid_type, pr2_put_sid_type, pr2_priv_count, pr2_get_priv, pr2_add_priv];
static PRIN_IIDS: &[Guid] = &[IID_IPRINCIPAL];
static PRIN2_IIDS: &[Guid] = &[IID_IPRINCIPAL2];

// ------------------------------------------------------------------------------- settings

struct SetObj {
    c: C<Settings>,
    idle: Lock<usize>,
    net: Lock<usize>,
}

impl Drop for SetObj {
    fn drop(&mut self) {
        for p in [self.idle.with(|p| *p), self.net.with(|p| *p)] {
            // SAFETY: references this object holds.
            unsafe { com_release(p as *mut c_void) };
        }
    }
}

boolean!(SetObj, st_get_demand, st_put_demand, |x| x.allow_demand_start);
text!(SetObj, st_get_restart_interval, st_put_restart_interval, |x| x.restart_interval);
number!(SetObj, i32, st_get_restart_count, st_put_restart_count, |x| x.restart_count);
number!(SetObj, i32, st_get_instances, st_put_instances, |x| x.multiple_instances);
boolean!(SetObj, st_get_stop_batt, st_put_stop_batt, |x| x.stop_if_going_on_batteries);
boolean!(SetObj, st_get_disallow_batt, st_put_disallow_batt, |x| x.disallow_start_if_on_batteries);
boolean!(SetObj, st_get_hard, st_put_hard, |x| x.allow_hard_terminate);
boolean!(SetObj, st_get_available, st_put_available, |x| x.start_when_available);
boolean!(SetObj, st_get_network_only, st_put_network_only, |x| x.run_only_if_network_available);
text!(SetObj, st_get_limit, st_put_limit, |x| x.execution_time_limit);
boolean!(SetObj, st_get_enabled, st_put_enabled, |x| x.enabled);
text!(SetObj, st_get_expire, st_put_expire, |x| x.delete_expired_task_after);
number!(SetObj, i32, st_get_priority, st_put_priority, |x| x.priority, |x, v| x.set_priority(v));
number!(SetObj, i32, st_get_compat, st_put_compat, |x| x.compatibility, |x, v| x.set_compatibility(v));
boolean!(SetObj, st_get_hidden, st_put_hidden, |x| x.hidden);
boolean!(SetObj, st_get_idle_only, st_put_idle_only, |x| x.run_only_if_idle);
boolean!(SetObj, st_get_wake, st_put_wake, |x| x.wake_to_run);
boolean!(SetObj, st_get_remote, st_put_remote, |x| x.disallow_start_on_remote_app_session);
boolean!(SetObj, st_get_unified, st_put_unified, |x| x.use_unified_scheduling_engine);
boolean!(SetObj, st_get_volatile, st_put_volatile, |x| x.volatile);

unsafe extern "system" fn st_get_idle(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a settings object.
    let o = unsafe { data::<SetObj>(this) };
    let p = o.idle.with(|p| {
        if *p == 0 {
            *p = make(IDLE_VTBL.ptr(), IDLE_IIDS, SubObj { c: o.c.clone() }) as usize;
        }
        *p
    }) as *mut c_void;
    // SAFETY: our cached object; the caller gets a reference.
    unsafe {
        add_ref(p);
        *out = p;
    }
    S_OK
}

unsafe extern "system" fn st_get_net(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a settings object.
    let o = unsafe { data::<SetObj>(this) };
    let p = o.net.with(|p| {
        if *p == 0 {
            *p = make(NET_VTBL.ptr(), NET_IIDS, SubObj { c: o.c.clone() }) as usize;
        }
        *p
    }) as *mut c_void;
    // SAFETY: as above.
    unsafe {
        add_ref(p);
        *out = p;
    }
    S_OK
}

/// `put_IdleSettings` / `put_NetworkSettings`: copy from one of our objects.
unsafe extern "system" fn st_put_idle(this: *mut c_void, from: *mut c_void) -> HRESULT {
    if from.is_null() {
        return E_POINTER;
    }
    if head_vtbl(from) != IDLE_VTBL.ptr() {
        return E_INVALIDARG;
    }
    // SAFETY: a settings object and one of our idle settings objects.
    let (o, f) = unsafe { (data::<SetObj>(this), data::<SubObj>(from)) };
    let v = f.c.with(|s| s.idle.clone());
    o.c.with(|s| s.idle = v);
    S_OK
}

unsafe extern "system" fn st_put_net(this: *mut c_void, from: *mut c_void) -> HRESULT {
    if from.is_null() {
        return E_POINTER;
    }
    if head_vtbl(from) != NET_VTBL.ptr() {
        return E_INVALIDARG;
    }
    // SAFETY: a settings object and one of our network settings objects.
    let (o, f) = unsafe { (data::<SetObj>(this), data::<SubObj>(from)) };
    let v = f.c.with(|s| s.network.clone());
    o.c.with(|s| s.network = v);
    S_OK
}

unsafe extern "system" fn st_maintenance(_this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's out parameter.
    unsafe { clear(out) };
    E_NOTIMPL
}

unsafe extern "system" fn st2_get_remote(this: *mut c_void, out: *mut i16) -> HRESULT {
    // SAFETY: the second pointer of a settings object.
    unsafe { st_get_remote(primary(this), out) }
}
unsafe extern "system" fn st2_put_remote(this: *mut c_void, v: i16) -> HRESULT {
    // SAFETY: as above.
    unsafe { st_put_remote(primary(this), v) }
}
unsafe extern "system" fn st2_get_unified(this: *mut c_void, out: *mut i16) -> HRESULT {
    // SAFETY: as above.
    unsafe { st_get_unified(primary(this), out) }
}
unsafe extern "system" fn st2_put_unified(this: *mut c_void, v: i16) -> HRESULT {
    // SAFETY: as above.
    unsafe { st_put_unified(primary(this), v) }
}

static SET_VTBL: Vt<56> = dual![
    st_get_demand,
    st_put_demand,
    st_get_restart_interval,
    st_put_restart_interval,
    st_get_restart_count,
    st_put_restart_count,
    st_get_instances,
    st_put_instances,
    st_get_stop_batt,
    st_put_stop_batt,
    st_get_disallow_batt,
    st_put_disallow_batt,
    st_get_hard,
    st_put_hard,
    st_get_available,
    st_put_available,
    xml_not_impl,
    xml_not_impl,
    st_get_network_only,
    st_put_network_only,
    st_get_limit,
    st_put_limit,
    st_get_enabled,
    st_put_enabled,
    st_get_expire,
    st_put_expire,
    st_get_priority,
    st_put_priority,
    st_get_compat,
    st_put_compat,
    st_get_hidden,
    st_put_hidden,
    st_get_idle,
    st_put_idle,
    st_get_idle_only,
    st_put_idle_only,
    st_get_wake,
    st_put_wake,
    st_get_net,
    st_put_net,
    st_get_remote,
    st_put_remote,
    st_get_unified,
    st_put_unified,
    st_maintenance,
    xml_not_impl,
    st_maintenance,
    st_get_volatile,
    st_put_volatile,
];
static SET2_VTBL: Vt<11> = dual2![st2_get_remote, st2_put_remote, st2_get_unified, st2_put_unified];
static SET_IIDS: &[Guid] = &[IID_ITASKSETTINGS, IID_ITASKSETTINGS3];
static SET2_IIDS: &[Guid] = &[IID_ITASKSETTINGS2];

/// The idle and network settings objects: the settings cell.
struct SubObj {
    c: C<Settings>,
}

text!(SubObj, idle_get_duration, idle_put_duration, |x| x.idle.duration);
text!(SubObj, idle_get_wait, idle_put_wait, |x| x.idle.wait_timeout);
boolean!(SubObj, idle_get_stop, idle_put_stop, |x| x.idle.stop_on_idle_end);
boolean!(SubObj, idle_get_restart, idle_put_restart, |x| x.idle.restart_on_idle);
text!(SubObj, net_get_name, net_put_name, |x| x.network.name);
text!(SubObj, net_get_id, net_put_id, |x| x.network.id);

static IDLE_VTBL: Vt<15> = dual![
    idle_get_duration,
    idle_put_duration,
    idle_get_wait,
    idle_put_wait,
    idle_get_stop,
    idle_put_stop,
    idle_get_restart,
    idle_put_restart,
];
static IDLE_IIDS: &[Guid] = &[IID_IIDLESETTINGS];
static NET_VTBL: Vt<11> = dual![net_get_name, net_put_name, net_get_id, net_put_id];
static NET_IIDS: &[Guid] = &[IID_INETWORKSETTINGS];

fn head_vtbl(p: *mut c_void) -> *const c_void {
    // SAFETY: a live COM object: its first word is its vtable.
    unsafe { *(p as *const *const c_void) }
}

// ------------------------------------------------------------------------------- triggers

struct TrigObj {
    c: C<Trigger>,
    rep: Lock<usize>,
}

impl Drop for TrigObj {
    fn drop(&mut self) {
        let p = self.rep.with(|p| *p);
        // SAFETY: the reference this object holds.
        unsafe { com_release(p as *mut c_void) };
    }
}

unsafe extern "system" fn tr_type(this: *mut c_void, out: *mut i32) -> HRESULT {
    // SAFETY: a trigger object.
    let o = unsafe { data::<TrigObj>(this) };
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, o.c.with(|x| x.kind)) }
}
text!(TrigObj, tr_get_id, tr_put_id, |x| x.id);
text!(TrigObj, tr_get_limit, tr_put_limit, |x| x.execution_time_limit);
text!(TrigObj, tr_get_start, tr_put_start, |x| x.start_boundary);
text!(TrigObj, tr_get_end, tr_put_end, |x| x.end_boundary);
boolean!(TrigObj, tr_get_enabled, tr_put_enabled, |x| x.enabled);
text!(TrigObj, tr_get_random, tr_put_random, |x| x.random_delay);
text!(TrigObj, tr_get_delay, tr_put_delay, |x| x.delay);
text!(TrigObj, tr_get_user, tr_put_user, |x| x.user_id, |v| v.map(|s| account::canonical(&s)));
text!(TrigObj, tr_get_subscription, tr_put_subscription, |x| x.subscription);
number!(TrigObj, i16, tr_get_days, tr_put_days, |x| x.days_interval, |x, v| x.set_days_interval(v));
number!(TrigObj, i16, tr_get_weeks_interval, tr_put_weeks_interval, |x| x.weeks_interval);
number!(TrigObj, i16, tr_get_dow, tr_put_dow, |x| x.days_of_week);
number!(TrigObj, i32, tr_get_dom, tr_put_dom, |x| x.days_of_month);
number!(TrigObj, i16, tr_get_moy, tr_put_moy, |x| x.months_of_year);
number!(TrigObj, i16, tr_get_wom, tr_put_wom, |x| x.weeks_of_month);
boolean!(TrigObj, tr_get_last_day, tr_put_last_day, |x| x.run_on_last_day_of_month);
boolean!(TrigObj, tr_get_last_week, tr_put_last_week, |x| x.run_on_last_week_of_month);
number!(TrigObj, i32, tr_get_state_change, tr_put_state_change, |x| x.state_change);

unsafe extern "system" fn tr_get_rep(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a trigger object.
    let o = unsafe { data::<TrigObj>(this) };
    let p = o.rep.with(|p| {
        if *p == 0 {
            *p = make(REP_VTBL.ptr(), REP_IIDS, RepObj { c: o.c.clone() }) as usize;
        }
        *p
    }) as *mut c_void;
    // SAFETY: our cached object; the caller gets a reference.
    unsafe {
        add_ref(p);
        *out = p;
    }
    S_OK
}

unsafe extern "system" fn tr_put_rep(this: *mut c_void, from: *mut c_void) -> HRESULT {
    if from.is_null() {
        return E_POINTER;
    }
    if head_vtbl(from) != REP_VTBL.ptr() {
        return E_INVALIDARG;
    }
    // SAFETY: a trigger object and one of our repetition objects.
    let (o, f) = unsafe { (data::<TrigObj>(this), data::<RepObj>(from)) };
    let v = f.c.with(|t| t.repetition.clone());
    o.c.with(|t| t.repetition = v);
    S_OK
}

unsafe extern "system" fn tr_value_queries(_this: *mut c_void, _x: *mut c_void) -> HRESULT {
    E_NOTIMPL
}

/// The 13 methods of `ITrigger` after the dual part, then the type's own.
macro_rules! trigger_vtbl {
    ($name:ident, $n:literal, [$($more:expr),* $(,)?]) => {
        static $name: Vt<$n> = dual![
            tr_type,
            tr_get_id,
            tr_put_id,
            tr_get_rep,
            tr_put_rep,
            tr_get_limit,
            tr_put_limit,
            tr_get_start,
            tr_put_start,
            tr_get_end,
            tr_put_end,
            tr_get_enabled,
            tr_put_enabled,
            $($more),*
        ];
    };
}

trigger_vtbl!(EVENT_VTBL, 26, [tr_get_subscription, tr_put_subscription, tr_get_delay, tr_put_delay, tr_value_queries, tr_value_queries]);
trigger_vtbl!(TIME_VTBL, 22, [tr_get_random, tr_put_random]);
trigger_vtbl!(DAILY_VTBL, 24, [tr_get_days, tr_put_days, tr_get_random, tr_put_random]);
trigger_vtbl!(WEEKLY_VTBL, 26, [tr_get_dow, tr_put_dow, tr_get_weeks_interval, tr_put_weeks_interval, tr_get_random, tr_put_random]);
trigger_vtbl!(MONTHLY_VTBL, 28, [tr_get_dom, tr_put_dom, tr_get_moy, tr_put_moy, tr_get_last_day, tr_put_last_day, tr_get_random, tr_put_random]);
trigger_vtbl!(
    MONTHLYDOW_VTBL,
    30,
    [tr_get_dow, tr_put_dow, tr_get_wom, tr_put_wom, tr_get_moy, tr_put_moy, tr_get_last_week, tr_put_last_week, tr_get_random, tr_put_random]
);
trigger_vtbl!(PLAIN_VTBL, 20, []);
trigger_vtbl!(DELAY_VTBL, 22, [tr_get_delay, tr_put_delay]);
trigger_vtbl!(LOGON_VTBL, 24, [tr_get_delay, tr_put_delay, tr_get_user, tr_put_user]);
trigger_vtbl!(SESSION_VTBL, 26, [tr_get_delay, tr_put_delay, tr_get_user, tr_put_user, tr_get_state_change, tr_put_state_change]);

static EVENT_IIDS: &[Guid] = &[IID_ITRIGGER, IID_IEVENTTRIGGER];
static TIME_IIDS: &[Guid] = &[IID_ITRIGGER, IID_ITIMETRIGGER];
static DAILY_IIDS: &[Guid] = &[IID_ITRIGGER, IID_IDAILYTRIGGER];
static WEEKLY_IIDS: &[Guid] = &[IID_ITRIGGER, IID_IWEEKLYTRIGGER];
static MONTHLY_IIDS: &[Guid] = &[IID_ITRIGGER, IID_IMONTHLYTRIGGER];
static MONTHLYDOW_IIDS: &[Guid] = &[IID_ITRIGGER, IID_IMONTHLYDOWTRIGGER];
static IDLE_TRIGGER_IIDS: &[Guid] = &[IID_ITRIGGER, IID_IIDLETRIGGER];
static REGISTRATION_IIDS: &[Guid] = &[IID_ITRIGGER, IID_IREGISTRATIONTRIGGER];
static BOOT_IIDS: &[Guid] = &[IID_ITRIGGER, IID_IBOOTTRIGGER];
static LOGON_IIDS: &[Guid] = &[IID_ITRIGGER, IID_ILOGONTRIGGER];
static SESSION_IIDS: &[Guid] = &[IID_ITRIGGER, IID_ISESSIONSTATECHANGETRIGGER];
static CUSTOM_IIDS: &[Guid] = &[IID_ITRIGGER];

fn trigger_object(c: C<Trigger>) -> *mut c_void {
    let kind = c.with(|t| t.kind);
    let (v, iids): (*const c_void, &'static [Guid]) = match kind {
        trigger::EVENT => (EVENT_VTBL.ptr(), EVENT_IIDS),
        trigger::TIME => (TIME_VTBL.ptr(), TIME_IIDS),
        trigger::DAILY => (DAILY_VTBL.ptr(), DAILY_IIDS),
        trigger::WEEKLY => (WEEKLY_VTBL.ptr(), WEEKLY_IIDS),
        trigger::MONTHLY => (MONTHLY_VTBL.ptr(), MONTHLY_IIDS),
        trigger::MONTHLYDOW => (MONTHLYDOW_VTBL.ptr(), MONTHLYDOW_IIDS),
        trigger::IDLE => (PLAIN_VTBL.ptr(), IDLE_TRIGGER_IIDS),
        trigger::REGISTRATION => (DELAY_VTBL.ptr(), REGISTRATION_IIDS),
        trigger::BOOT => (DELAY_VTBL.ptr(), BOOT_IIDS),
        trigger::LOGON => (LOGON_VTBL.ptr(), LOGON_IIDS),
        trigger::SESSION_STATE_CHANGE => (SESSION_VTBL.ptr(), SESSION_IIDS),
        _ => (PLAIN_VTBL.ptr(), CUSTOM_IIDS),
    };
    make(v, iids, TrigObj { c, rep: Lock::new(0) })
}

struct RepObj {
    c: C<Trigger>,
}

text!(RepObj, rep_get_interval, rep_put_interval, |x| x.repetition.interval);
text!(RepObj, rep_get_duration, rep_put_duration, |x| x.repetition.duration);
boolean!(RepObj, rep_get_stop, rep_put_stop, |x| x.repetition.stop_at_duration_end);

static REP_VTBL: Vt<13> = dual![rep_get_interval, rep_put_interval, rep_get_duration, rep_put_duration, rep_get_stop, rep_put_stop];
static REP_IIDS: &[Guid] = &[IID_IREPETITIONPATTERN];

// ------------------------------------------------------------------------------- the collections

/// A 1-based index of a `LONG` collection argument: 0 and below `E_INVALIDARG`, past the end
/// `E_FAIL` (sheets `ITriggerCollection.Item`, `IActionCollection.Item`).
fn index(i: i32, n: usize) -> Result<usize, HRESULT> {
    if i < 1 {
        Err(E_INVALIDARG)
    } else if i as usize > n {
        Err(E_FAIL)
    } else {
        Ok(i as usize - 1)
    }
}

struct TrigColl {
    c: C<Vec<C<Trigger>>>,
}

unsafe extern "system" fn tc_count(this: *mut c_void, out: *mut i32) -> HRESULT {
    // SAFETY: a trigger collection.
    let o = unsafe { data::<TrigColl>(this) };
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, o.c.with(|v| v.len() as i32)) }
}

unsafe extern "system" fn tc_item(this: *mut c_void, i: i32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    // SAFETY: a trigger collection.
    let o = unsafe { data::<TrigColl>(this) };
    let item = o.c.with(|v| index(i, v.len()).map(|k| v[k].clone()));
    match item {
        Ok(c) => {
            // SAFETY: the caller's out parameter.
            unsafe { *out = trigger_object(c) };
            S_OK
        }
        Err(hr) => hr,
    }
}

unsafe extern "system" fn tc_enum(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a trigger collection.
    let o = unsafe { data::<TrigColl>(this) };
    let items: Vec<*mut c_void> = o.c.with(|v| v.clone()).into_iter().map(trigger_object).collect();
    // SAFETY: new objects handed to the enumerator.
    unsafe { new_enum(items, out) }
}

unsafe extern "system" fn tc_create(this: *mut c_void, kind: i32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if !trigger::valid(kind) {
        return E_INVALIDARG;
    }
    // SAFETY: a trigger collection.
    let o = unsafe { data::<TrigColl>(this) };
    let c = cell(Trigger::new(kind));
    o.c.with(|v| v.push(c.clone()));
    // SAFETY: the caller's out parameter.
    unsafe { *out = trigger_object(c) };
    S_OK
}

/// A `Remove(VARIANT)` position: `VT_I4`/`VT_I2` 1..=n.
fn remove_index(v: &Variant, n: usize) -> Result<usize, HRESULT> {
    if v.vt == VT_BSTR {
        return Err(DISP_E_TYPEMISMATCH);
    }
    match v.int() {
        Some(i) if i >= 1 && (i as usize) <= n => Ok(i as usize - 1),
        _ => Err(E_INVALIDARG),
    }
}

unsafe extern "system" fn tc_remove(this: *mut c_void, v: Variant) -> HRESULT {
    // SAFETY: a trigger collection.
    let o = unsafe { data::<TrigColl>(this) };
    o.c.with(|list| match remove_index(&v, list.len()) {
        Ok(k) => {
            list.remove(k);
            S_OK
        }
        Err(hr) => hr,
    })
}

unsafe extern "system" fn tc_clear(this: *mut c_void) -> HRESULT {
    // SAFETY: a trigger collection.
    unsafe { data::<TrigColl>(this) }.c.with(|v| v.clear());
    S_OK
}

static TRIGCOLL_VTBL: Vt<13> = dual![tc_count, tc_item, tc_enum, tc_create, tc_remove, tc_clear];
static TRIGCOLL_IIDS: &[Guid] = &[IID_ITRIGGERCOLLECTION];

struct ActColl {
    c: C<Actions>,
}

struct ActObj {
    c: C<Action>,
}

text!(ActObj, ac_get_id, ac_put_id, |x| x.id);
unsafe extern "system" fn ac_type(this: *mut c_void, out: *mut i32) -> HRESULT {
    // SAFETY: an action object.
    let o = unsafe { data::<ActObj>(this) };
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, o.c.with(|x| x.kind)) }
}
text!(ActObj, ex_get_path, ex_put_path, |x| x.path);
text!(ActObj, ex_get_args, ex_put_args, |x| x.arguments);
text!(ActObj, ex_get_dir, ex_put_dir, |x| x.working_directory);
boolean!(ActObj, ex_get_hide, ex_put_hide, |x| x.hide_app_window);
text!(ActObj, com_get_class, com_put_class, |x| x.class_id);
text!(ActObj, com_get_data, com_put_data, |x| x.data);
text!(ActObj, em_get_server, em_put_server, |x| x.server);
text!(ActObj, em_get_subject, em_put_subject, |x| x.subject);
text!(ActObj, em_get_to, em_put_to, |x| x.to);
text!(ActObj, em_get_cc, em_put_cc, |x| x.cc);
text!(ActObj, em_get_bcc, em_put_bcc, |x| x.bcc);
text!(ActObj, em_get_reply, em_put_reply, |x| x.reply_to);
text!(ActObj, em_get_from, em_put_from, |x| x.from);
text!(ActObj, em_get_body, em_put_body, |x| x.body);
text!(ActObj, msg_get_title, msg_put_title, |x| x.title);
text!(ActObj, msg_get_body, msg_put_body, |x| x.message_body);

static EXEC_VTBL: Vt<18> =
    dual![ac_get_id, ac_put_id, ac_type, ex_get_path, ex_put_path, ex_get_args, ex_put_args, ex_get_dir, ex_put_dir, ex_get_hide, ex_put_hide];
static COM_VTBL: Vt<14> = dual![ac_get_id, ac_put_id, ac_type, com_get_class, com_put_class, com_get_data, com_put_data];
static EMAIL_VTBL: Vt<30> = dual![
    ac_get_id,
    ac_put_id,
    ac_type,
    em_get_server,
    em_put_server,
    em_get_subject,
    em_put_subject,
    em_get_to,
    em_put_to,
    em_get_cc,
    em_put_cc,
    em_get_bcc,
    em_put_bcc,
    em_get_reply,
    em_put_reply,
    em_get_from,
    em_put_from,
    xml_not_impl,
    xml_not_impl,
    em_get_body,
    em_put_body,
    xml_not_impl,
    xml_not_impl,
];
static MSG_VTBL: Vt<14> = dual![ac_get_id, ac_put_id, ac_type, msg_get_title, msg_put_title, msg_get_body, msg_put_body];
static EXEC_IIDS: &[Guid] = &[IID_IACTION, IID_IEXECACTION, IID_IEXECACTION2];
static COM_IIDS: &[Guid] = &[IID_IACTION, IID_ICOMHANDLERACTION];
static EMAIL_IIDS: &[Guid] = &[IID_IACTION, IID_IEMAILACTION];
static MSG_IIDS: &[Guid] = &[IID_IACTION, IID_ISHOWMESSAGEACTION];

fn action_object(c: C<Action>) -> *mut c_void {
    let kind = c.with(|a| a.kind);
    let (v, iids): (*const c_void, &'static [Guid]) = match kind {
        action::EXEC => (EXEC_VTBL.ptr(), EXEC_IIDS),
        action::COM_HANDLER => (COM_VTBL.ptr(), COM_IIDS),
        action::SEND_EMAIL => (EMAIL_VTBL.ptr(), EMAIL_IIDS),
        _ => (MSG_VTBL.ptr(), MSG_IIDS),
    };
    make(v, iids, ActObj { c })
}

unsafe extern "system" fn acc_count(this: *mut c_void, out: *mut i32) -> HRESULT {
    // SAFETY: an action collection.
    let o = unsafe { data::<ActColl>(this) };
    // SAFETY: the caller's out parameter.
    unsafe { put_val(out, o.c.with(|a| a.items.len() as i32)) }
}

unsafe extern "system" fn acc_item(this: *mut c_void, i: i32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    // SAFETY: an action collection.
    let o = unsafe { data::<ActColl>(this) };
    match o.c.with(|a| index(i, a.items.len()).map(|k| a.items[k].clone())) {
        Ok(c) => {
            // SAFETY: the caller's out parameter.
            unsafe { *out = action_object(c) };
            S_OK
        }
        Err(hr) => hr,
    }
}

unsafe extern "system" fn acc_enum(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: an action collection.
    let o = unsafe { data::<ActColl>(this) };
    let items: Vec<*mut c_void> = o.c.with(|a| a.items.clone()).into_iter().map(action_object).collect();
    // SAFETY: new objects handed to the enumerator.
    unsafe { new_enum(items, out) }
}

unsafe extern "system" fn acc_create(this: *mut c_void, kind: i32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if !action::valid(kind) {
        return E_INVALIDARG;
    }
    // SAFETY: an action collection.
    let o = unsafe { data::<ActColl>(this) };
    let c = cell(Action::new(kind));
    o.c.with(|a| a.items.push(c.clone()));
    // SAFETY: the caller's out parameter.
    unsafe { *out = action_object(c) };
    S_OK
}

unsafe extern "system" fn acc_remove(this: *mut c_void, v: Variant) -> HRESULT {
    // SAFETY: an action collection.
    let o = unsafe { data::<ActColl>(this) };
    if let Some(id) = v.text() {
        let id = id.unwrap_or_default();
        return o.c.with(|a| match a.items.iter().position(|x| x.with(|x| x.id.as_deref() == Some(id.as_str()))) {
            Some(k) => {
                a.items.remove(k);
                S_OK
            }
            None => E_INVALIDARG,
        });
    }
    o.c.with(|a| match remove_index(&v, a.items.len()) {
        Ok(k) => {
            a.items.remove(k);
            S_OK
        }
        Err(hr) => hr,
    })
}

unsafe extern "system" fn acc_clear(this: *mut c_void) -> HRESULT {
    // SAFETY: an action collection.
    unsafe { data::<ActColl>(this) }.c.with(|a| a.items.clear());
    S_OK
}

text!(ActColl, acc_get_context, acc_put_context, |x| x.context);

static ACTCOLL_VTBL: Vt<17> =
    dual![acc_count, acc_item, acc_enum, xml_not_impl, xml_not_impl, acc_create, acc_remove, acc_clear, acc_get_context, acc_put_context];
static ACTCOLL_IIDS: &[Guid] = &[IID_IACTIONCOLLECTION];

// ------------------------------------------------------------------------------- the definition

struct DefObj {
    st: DefState,
    /// The part objects: registration info, triggers, settings, principal, actions.
    parts: Lock<[usize; 5]>,
}

impl Drop for DefObj {
    fn drop(&mut self) {
        for p in self.parts.with(|p| *p) {
            // SAFETY: references this object holds.
            unsafe { com_release(p as *mut c_void) };
        }
    }
}

fn part(o: &DefObj, k: usize, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let p = o.parts.with(|parts| {
        if parts[k] == 0 {
            let s = &o.st;
            parts[k] = match k {
                0 => make(REG_VTBL.ptr(), REG_IIDS, RegObj { c: s.reg.clone() }),
                1 => make(TRIGCOLL_VTBL.ptr(), TRIGCOLL_IIDS, TrigColl { c: s.triggers.clone() }),
                2 => make2(
                    SET_VTBL.ptr(),
                    SET_IIDS,
                    SET2_VTBL.ptr(),
                    SET2_IIDS,
                    SetObj { c: s.settings.clone(), idle: Lock::new(0), net: Lock::new(0) },
                ),
                3 => make2(PRIN_VTBL.ptr(), PRIN_IIDS, PRIN2_VTBL.ptr(), PRIN2_IIDS, PrinObj { c: s.principal.clone() }),
                _ => make(ACTCOLL_VTBL.ptr(), ACTCOLL_IIDS, ActColl { c: s.actions.clone() }),
            } as usize;
        }
        parts[k]
    }) as *mut c_void;
    // SAFETY: our cached part object; the caller gets a reference.
    unsafe {
        add_ref(p);
        *out = p;
    }
    S_OK
}

unsafe extern "system" fn def_get_reg(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a definition object.
    part(unsafe { data::<DefObj>(this) }, 0, out)
}
unsafe extern "system" fn def_get_triggers(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: as above.
    part(unsafe { data::<DefObj>(this) }, 1, out)
}
unsafe extern "system" fn def_get_settings(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: as above.
    part(unsafe { data::<DefObj>(this) }, 2, out)
}
unsafe extern "system" fn def_get_principal(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: as above.
    part(unsafe { data::<DefObj>(this) }, 3, out)
}
unsafe extern "system" fn def_get_actions(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: as above.
    part(unsafe { data::<DefObj>(this) }, 4, out)
}

/// The `put_X(part)` methods copy the content of one of our part objects of that kind.
macro_rules! put_part {
    ($name:ident, $vt:ident, $obj:ty, |$o:ident, $f:ident| $copy:expr) => {
        unsafe extern "system" fn $name(this: *mut c_void, from: *mut c_void) -> HRESULT {
            if from.is_null() {
                return E_POINTER;
            }
            if head_vtbl(from) != $vt.ptr() {
                return E_INVALIDARG;
            }
            // SAFETY: a definition object and one of our part objects of that kind.
            let ($o, $f) = unsafe { (data::<DefObj>(this), data::<$obj>(from)) };
            $copy;
            S_OK
        }
    };
}

put_part!(def_put_reg, REG_VTBL, RegObj, |o, f| {
    let v = f.c.with(|x| x.clone());
    o.st.reg.with(|x| *x = v);
});
put_part!(def_put_triggers, TRIGCOLL_VTBL, TrigColl, |o, f| {
    let v: Vec<C<Trigger>> = f.c.with(|x| x.iter().map(|t| cell(t.with(|t| t.clone()))).collect());
    o.st.triggers.with(|x| *x = v);
});
put_part!(def_put_settings, SET_VTBL, SetObj, |o, f| {
    let v = f.c.with(|x| x.clone());
    o.st.settings.with(|x| *x = v);
});
put_part!(def_put_principal, PRIN_VTBL, PrinObj, |o, f| {
    let v = f.c.with(|x| x.clone());
    o.st.principal.with(|x| *x = v);
});
put_part!(def_put_actions, ACTCOLL_VTBL, ActColl, |o, f| {
    let (ctx, items): (Text, Vec<C<Action>>) =
        f.c.with(|x| (x.context.clone(), x.items.iter().map(|a| cell(a.with(|a| a.clone()))).collect()));
    o.st.actions.with(|x| {
        x.context = ctx;
        x.items = items;
    });
});

unsafe extern "system" fn def_get_data(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    // SAFETY: a definition object.
    let o = unsafe { data::<DefObj>(this) };
    let v = o.st.data.with(|x| x.clone());
    // SAFETY: the caller's out parameter.
    unsafe { put_text(out, &v) }
}

unsafe extern "system" fn def_put_data(this: *mut c_void, b: Bstr) -> HRESULT {
    // SAFETY: a definition object.
    let o = unsafe { data::<DefObj>(this) };
    let v = from_bstr(b);
    o.st.data.with(|x| *x = v);
    S_OK
}

unsafe extern "system" fn def_get_xml(this: *mut c_void, out: *mut Bstr) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a definition object.
    let o = unsafe { data::<DefObj>(this) };
    let x = definition_xml(&o.st.snapshot());
    // SAFETY: the caller's out parameter.
    unsafe { *out = bstr(&x) };
    S_OK
}

unsafe extern "system" fn def_put_xml(this: *mut c_void, b: Bstr) -> HRESULT {
    // SAFETY: a definition object.
    let o = unsafe { data::<DefObj>(this) };
    match parse_task(&from_bstr(b).unwrap_or_default()) {
        Ok(d) => {
            o.st.load(d);
            S_OK
        }
        Err(hr) => hr,
    }
}

static DEF_VTBL: Vt<21> = dual![
    def_get_reg,
    def_put_reg,
    def_get_triggers,
    def_put_triggers,
    def_get_settings,
    def_put_settings,
    def_get_data,
    def_put_data,
    def_get_principal,
    def_put_principal,
    def_get_actions,
    def_put_actions,
    def_get_xml,
    def_put_xml,
];
static DEF_IIDS: &[Guid] = &[IID_ITASKDEFINITION];

/// A new `ITaskDefinition` holding `d`.
pub fn new_definition(d: Definition) -> *mut c_void {
    make(DEF_VTBL.ptr(), DEF_IIDS, DefObj { st: DefState::new(d), parts: Lock::new([0; 5]) })
}

/// The content of one of our definitions (`None` for any other object).
pub fn definition_of(p: *mut c_void) -> Option<Definition> {
    if p.is_null() || head_vtbl(p) != DEF_VTBL.ptr() {
        return None;
    }
    // SAFETY: one of our definition objects.
    Some(unsafe { data::<DefObj>(p) }.st.snapshot())
}
