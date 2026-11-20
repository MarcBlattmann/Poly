//! CFPreferences (`<CoreFoundation/CFPreferences.h>`): per-application
//! domains for the current user, plus the user's global domain.
//!
//! v0 keeps the domains in the process: a value set is visible at once and
//! `CFPreferencesAppSynchronize` answers true, as the spec sheets describe;
//! writing `~/Library/Preferences/<domain>.plist` (which macOS leaves to a
//! daemon, "shortly after") comes with a preferences service of Poly's own.
//! The global domain starts with `AppleLanguages` = ("en").

use crate::base::{self, Boolean, CFIndex, Ref, Shared, id};
use crate::string::{self, ConstStr, cfstr};
use alloc::string::String;
use alloc::vec::Vec;

static ANY_APPLICATION: ConstStr = cfstr!("kCFPreferencesAnyApplication");
static CURRENT_APPLICATION: ConstStr = cfstr!("kCFPreferencesCurrentApplication");
static ANY_HOST: ConstStr = cfstr!("kCFPreferencesAnyHost");
static CURRENT_HOST: ConstStr = cfstr!("kCFPreferencesCurrentHost");
static ANY_USER: ConstStr = cfstr!("kCFPreferencesAnyUser");
static CURRENT_USER: ConstStr = cfstr!("kCFPreferencesCurrentUser");

#[unsafe(no_mangle)]
pub static kCFPreferencesAnyApplication: Shared<ConstStr> = Shared(&ANY_APPLICATION);
#[unsafe(no_mangle)]
pub static kCFPreferencesCurrentApplication: Shared<ConstStr> = Shared(&CURRENT_APPLICATION);
#[unsafe(no_mangle)]
pub static kCFPreferencesAnyHost: Shared<ConstStr> = Shared(&ANY_HOST);
#[unsafe(no_mangle)]
pub static kCFPreferencesCurrentHost: Shared<ConstStr> = Shared(&CURRENT_HOST);
#[unsafe(no_mangle)]
pub static kCFPreferencesAnyUser: Shared<ConstStr> = Shared(&ANY_USER);
#[unsafe(no_mangle)]
pub static kCFPreferencesCurrentUser: Shared<ConstStr> = Shared(&CURRENT_USER);

/// The name of the global domain.
const GLOBAL: &str = "kCFPreferencesAnyApplication";

struct Domains {
    /// (domain name, mutable dictionary of its values).
    list: Vec<(String, Ref)>,
}

static DOMAINS: crate::Global<Domains> = crate::Global::new(Domains { list: Vec::new() });

fn domains() -> &'static mut Domains {
    // SAFETY: one thread (v0); no callbacks run while it is used.
    unsafe { DOMAINS.get() }
}

/// The domain name an application id stands for.
fn domain_name(app: Ref) -> String {
    let name = string::text(app);
    if name == "kCFPreferencesCurrentApplication" {
        let b = crate::bundle::CFBundleGetMainBundle();
        let id = crate::bundle::CFBundleGetIdentifier(b);
        if !id.is_null() {
            return string::text(id);
        }
        let exe = crate::sys::executable_path();
        return String::from(poly_cf::bundle::last_component(&exe));
    }
    name
}

/// The dictionary of a domain, made on first use.
fn domain(name: &str) -> Ref {
    let d = domains();
    if let Some((_, dict)) = d.list.iter().find(|(n, _)| n == name) {
        return *dict;
    }
    let dict =
        crate::dict::dictionary(core::ptr::null(), &[], &[], &crate::dict::TYPE_KEYS, &crate::dict::TYPE_VALUES, true);
    if name == GLOBAL {
        let en = string::from_str("en");
        let langs = crate::array::from_refs(core::ptr::null(), &[en], &crate::array::TYPE_CALLBACKS, false);
        let key = string::from_str("AppleLanguages");
        crate::dict::CFDictionarySetValue(dict, key, langs);
        for r in [en, langs, key] {
            base::release(r);
        }
    }
    domains().list.push((String::from(name), dict));
    dict
}

fn is(r: Ref, s: &'static ConstStr) -> bool {
    !r.is_null() && base::equal(r, string::constant(s))
}

/// A deep, immutable copy of a property list value.
pub fn deep_copy(v: Ref) -> Ref {
    let null = core::ptr::null();
    match base::type_of(v) {
        id::STRING => string::from_units(null, string::units(v).to_vec(), false),
        id::DATA => crate::data::from_vec(null, crate::data::bytes(v).to_vec(), false),
        id::ARRAY => {
            let items: Vec<Ref> = crate::array::items(v).iter().map(|&e| deep_copy(e)).collect();
            let a = crate::array::from_refs(null, &items, &crate::array::TYPE_CALLBACKS, false);
            items.into_iter().for_each(base::release);
            a
        }
        id::DICTIONARY => {
            let (k, vals) = crate::dict::pairs(v);
            let keys: Vec<Ref> = k.iter().map(|&x| deep_copy(x)).collect();
            let values: Vec<Ref> = vals.iter().map(|&x| deep_copy(x)).collect();
            let d = crate::dict::dictionary(
                null,
                &keys,
                &values,
                &crate::dict::TYPE_KEYS,
                &crate::dict::TYPE_VALUES,
                false,
            );
            keys.into_iter().chain(values).for_each(base::release);
            d
        }
        _ => base::retain(v),
    }
}

fn copy_value(key: Ref, app: Ref, with_global: bool) -> Ref {
    if key.is_null() {
        return core::ptr::null();
    }
    let name = domain_name(app);
    if let Some(v) = crate::dict::lookup(domain(&name), key) {
        return deep_copy(v);
    }
    if with_global
        && name != GLOBAL
        && let Some(v) = crate::dict::lookup(domain(GLOBAL), key)
    {
        return deep_copy(v);
    }
    core::ptr::null()
}

fn set_value(key: Ref, value: Ref, app: Ref) {
    if key.is_null() {
        return;
    }
    let dict = domain(&domain_name(app));
    if value.is_null() {
        crate::dict::CFDictionaryRemoveValue(dict, key);
    } else {
        let k = string::from_units(core::ptr::null(), string::units(key).to_vec(), false);
        let v = deep_copy(value);
        crate::dict::CFDictionarySetValue(dict, k, v);
        base::release(k);
        base::release(v);
    }
}

/// Whether a user/host pair names the current user's own domain.
fn own(user: Ref, host: Ref) -> bool {
    is(user, &CURRENT_USER) && is(host, &ANY_HOST)
}

// ----- exports -----

#[unsafe(no_mangle)]
pub extern "C" fn CFPreferencesCopyAppValue(key: Ref, app: Ref) -> Ref {
    copy_value(key, app, true)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFPreferencesSetAppValue(key: Ref, value: Ref, app: Ref) {
    set_value(key, value, app)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFPreferencesAppSynchronize(_app: Ref) -> Boolean {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn CFPreferencesSynchronize(_app: Ref, _user: Ref, _host: Ref) -> Boolean {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn CFPreferencesCopyValue(key: Ref, app: Ref, user: Ref, host: Ref) -> Ref {
    if !own(user, host) {
        return core::ptr::null();
    }
    copy_value(key, app, false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFPreferencesSetValue(key: Ref, value: Ref, app: Ref, user: Ref, host: Ref) {
    if own(user, host) {
        set_value(key, value, app)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFPreferencesCopyKeyList(app: Ref, user: Ref, host: Ref) -> Ref {
    if !own(user, host) {
        return core::ptr::null();
    }
    let (keys, _) = crate::dict::pairs(domain(&domain_name(app)));
    if keys.is_empty() {
        return core::ptr::null();
    }
    crate::array::from_refs(core::ptr::null(), &keys, &crate::array::TYPE_CALLBACKS, false)
}

/// # Safety
/// `valid` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFPreferencesGetAppBooleanValue(key: Ref, app: Ref, valid: *mut Boolean) -> Boolean {
    let v = copy_value(key, app, true);
    let r = if v.is_null() {
        None
    } else {
        let out = match base::type_of(v) {
            id::BOOLEAN => Some(crate::number::bool_value(v)),
            id::NUMBER => Some(crate::number::value(v) != poly_cf::number::Num::Int(0)),
            id::STRING => {
                let t = string::text(v);
                match t.as_str() {
                    "YES" | "yes" | "true" | "1" => Some(true),
                    "NO" | "no" | "false" | "0" => Some(false),
                    _ => None,
                }
            }
            _ => None,
        };
        base::release(v);
        out
    };
    if !valid.is_null() {
        // SAFETY: the caller's contract.
        unsafe { *valid = r.is_some() as Boolean };
    }
    r.unwrap_or(false) as Boolean
}

/// # Safety
/// `valid` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFPreferencesGetAppIntegerValue(key: Ref, app: Ref, valid: *mut Boolean) -> CFIndex {
    let v = copy_value(key, app, true);
    let r = if v.is_null() {
        None
    } else {
        let out = match base::type_of(v) {
            id::NUMBER => match crate::number::value(v) {
                poly_cf::number::Num::Int(i) => Some(i as CFIndex),
                poly_cf::number::Num::Float(d) => Some(d as CFIndex),
            },
            id::BOOLEAN => Some(CFIndex::from(crate::number::bool_value(v))),
            id::STRING => string::text(v).trim().parse::<CFIndex>().ok(),
            _ => None,
        };
        base::release(v);
        out
    };
    if !valid.is_null() {
        // SAFETY: the caller's contract.
        unsafe { *valid = r.is_some() as Boolean };
    }
    r.unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFPropertyListCreateDeepCopy(_alloc: Ref, plist: Ref, _option: u64) -> Ref {
    if plist.is_null() {
        return plist;
    }
    deep_copy(plist)
}
