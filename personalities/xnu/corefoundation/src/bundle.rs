//! CFBundle (`<CoreFoundation/CFBundle.h>`): a directory with an
//! Info.plist, resources and an executable; the main bundle is the one that
//! holds the running program. Path logic is `poly_cf::bundle`'s.

use crate::base::{self, Boolean, Header, Ref, Shared, id};
use crate::string::{self, ConstStr, cfstr};
use alloc::string::String;
use alloc::vec::Vec;
use poly_cf::bundle::{self as paths, Layout};
use poly_cf::plist::Value;

#[repr(C)]
pub struct Bundle {
    h: Header,
    dir: String,
    layout: Layout,
    /// The Info dictionary (immutable, owned).
    info: Ref,
    /// The bundle directory's URL (owned).
    url: Ref,
}

static IDENTIFIER_KEY: ConstStr = cfstr!("CFBundleIdentifier");
static NAME_KEY: ConstStr = cfstr!("CFBundleName");
static VERSION_KEY: ConstStr = cfstr!("CFBundleVersion");
static EXECUTABLE_KEY: ConstStr = cfstr!("CFBundleExecutable");
static INFO_VERSION_KEY: ConstStr = cfstr!("CFBundleInfoDictionaryVersion");
static LOCALIZATIONS_KEY: ConstStr = cfstr!("CFBundleLocalizations");
static DEVELOPMENT_REGION_KEY: ConstStr = cfstr!("CFBundleDevelopmentRegion");

#[unsafe(no_mangle)]
pub static kCFBundleIdentifierKey: Shared<ConstStr> = Shared(&IDENTIFIER_KEY);
#[unsafe(no_mangle)]
pub static kCFBundleNameKey: Shared<ConstStr> = Shared(&NAME_KEY);
#[unsafe(no_mangle)]
pub static kCFBundleVersionKey: Shared<ConstStr> = Shared(&VERSION_KEY);
#[unsafe(no_mangle)]
pub static kCFBundleExecutableKey: Shared<ConstStr> = Shared(&EXECUTABLE_KEY);
#[unsafe(no_mangle)]
pub static kCFBundleInfoDictionaryVersionKey: Shared<ConstStr> = Shared(&INFO_VERSION_KEY);
#[unsafe(no_mangle)]
pub static kCFBundleLocalizationsKey: Shared<ConstStr> = Shared(&LOCALIZATIONS_KEY);
#[unsafe(no_mangle)]
pub static kCFBundleDevelopmentRegionKey: Shared<ConstStr> = Shared(&DEVELOPMENT_REGION_KEY);

struct Registry {
    main: Ref,
    /// Every bundle made, for `CFBundleCreate` on an open directory and
    /// `CFBundleGetBundleWithIdentifier` (each holds a reference).
    all: Vec<Ref>,
}

static REGISTRY: crate::Global<Registry> = crate::Global::new(Registry { main: core::ptr::null(), all: Vec::new() });

fn registry() -> &'static mut Registry {
    // SAFETY: one thread (v0); no callbacks into the program run while it is
    // in use.
    unsafe { REGISTRY.get() }
}

fn get<'a>(cf: Ref) -> &'a Bundle {
    if base::type_of(cf) != id::BUNDLE {
        base::crash("not a CFBundle");
    }
    // SAFETY: a bundle object.
    unsafe { &*(cf as *const Bundle) }
}

/// A property list value as CF objects (integers as 64-bit numbers).
pub fn to_cf(v: &Value) -> Ref {
    let null = core::ptr::null();
    match v {
        Value::String(s) => string::from_str(s),
        Value::Integer(i) => crate::number::from_i64(*i),
        Value::Real(d) => crate::number::make(null, poly_cf::number::FLOAT64, poly_cf::number::Num::Float(*d)),
        Value::Bool(b) => crate::number::boolean(*b),
        Value::Date(t) => crate::date::make(null, *t),
        Value::Data(d) => crate::data::from_vec(null, d.clone(), false),
        Value::Array(items) => {
            let refs: Vec<Ref> = items.iter().map(to_cf).collect();
            let a = crate::array::from_refs(null, &refs, &crate::array::TYPE_CALLBACKS, false);
            refs.into_iter().for_each(base::release);
            a
        }
        Value::Dict(pairs) => {
            let keys: Vec<Ref> = pairs.iter().map(|(k, _)| string::from_str(k)).collect();
            let values: Vec<Ref> = pairs.iter().map(|(_, v)| to_cf(v)).collect();
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
    }
}

/// The Info dictionary of a bundle directory: the plist's keys, plus
/// `CFBundleNumericVersion` when it has any.
fn read_info(dir: &str, layout: Layout) -> Ref {
    let null = core::ptr::null();
    let parsed = crate::sys::read_file(&paths::info_plist(dir, layout)).and_then(|b| poly_cf::plist::parse(&b));
    let Some(Value::Dict(mut pairs)) = parsed else {
        return crate::dict::dictionary(null, &[], &[], &crate::dict::TYPE_KEYS, &crate::dict::TYPE_VALUES, false);
    };
    if !pairs.is_empty() {
        let text = |k: &str| {
            pairs.iter().find_map(|(key, v)| match v {
                Value::String(s) if key == k => Some(s.clone()),
                _ => None,
            })
        };
        let n = text("CFBundleVersion")
            .map(|v| paths::version_number(&v))
            .filter(|&n| n != 0)
            .or_else(|| text("CFBundleShortVersionString").map(|v| paths::version_number(&v)))
            .unwrap_or(0);
        pairs.push((String::from("CFBundleNumericVersion"), Value::Integer(i64::from(n))));
    }
    to_cf(&Value::Dict(pairs))
}

fn create_at(dir: &str, layout: Layout) -> Ref {
    let info = read_info(dir, layout);
    let url = crate::url::from_path(core::ptr::null(), dir, true);
    let b =
        base::make(core::ptr::null(), id::BUNDLE, false, |h| Bundle { h, dir: String::from(dir), layout, info, url })
            as Ref;
    registry().all.push(b);
    b
}

/// # Safety
/// The count reached zero (bundles stay in the registry, so this is not
/// reached in v0).
pub unsafe fn destroy(cf: Ref) {
    let b = get(cf);
    base::release(b.info);
    base::release(b.url);
    // SAFETY: a bundle made here.
    unsafe { base::destroy::<Bundle>(cf) }
}

fn info_value(b: Ref, key: &str) -> Option<String> {
    let k = string::from_str(key);
    let v = crate::dict::lookup(get(b).info, k);
    base::release(k);
    v.filter(|&v| base::type_of(v) == id::STRING).map(string::text)
}

fn resources(b: &Bundle) -> String {
    paths::resources_dir(&b.dir, b.layout)
}

/// The URL of the first file the lookup finds.
fn find_resource(res_dir: &str, name: &str, ext: Option<&str>, sub: Option<&str>) -> Ref {
    let Some(file) = paths::resource_file(name, ext) else { return core::ptr::null() };
    for p in paths::resource_candidates(res_dir, sub, &file) {
        if let Some(dir) = crate::sys::is_dir(&p) {
            return crate::url::from_path(core::ptr::null(), &p, dir);
        }
    }
    core::ptr::null()
}

fn opt_text(s: Ref) -> Option<String> {
    (!s.is_null()).then(|| string::text(s))
}

// ----- exports -----

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleGetTypeID() -> usize {
    id::BUNDLE
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleGetMainBundle() -> Ref {
    let r = registry();
    if r.main.is_null() {
        let (dir, layout) = paths::main_bundle_dir(&crate::sys::executable_path());
        let b = create_at(&dir, layout);
        registry().main = b;
    }
    registry().main
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleCreate(_alloc: Ref, url: Ref) -> Ref {
    if url.is_null() {
        return core::ptr::null();
    }
    let Some(p) = crate::url::path(url) else { return core::ptr::null() };
    if crate::sys::is_dir(&p) != Some(true) {
        return core::ptr::null();
    }
    CFBundleGetMainBundle();
    if let Some(&b) = registry().all.iter().find(|&&b| get(b).dir == p) {
        return base::retain(b);
    }
    let layout =
        if crate::sys::is_dir(&paths::join(&p, "Contents")) == Some(true) { Layout::Contents } else { Layout::Flat };
    base::retain(create_at(&p, layout))
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleGetBundleWithIdentifier(ident: Ref) -> Ref {
    CFBundleGetMainBundle();
    let want = string::text(ident);
    registry()
        .all
        .iter()
        .copied()
        .find(|&b| info_value(b, "CFBundleIdentifier").as_deref() == Some(want.as_str()))
        .unwrap_or(core::ptr::null())
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleGetIdentifier(b: Ref) -> Ref {
    CFBundleGetValueForInfoDictionaryKey(b, kCFBundleIdentifierKey.0 as Ref)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleGetInfoDictionary(b: Ref) -> Ref {
    get(b).info
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleGetLocalInfoDictionary(_b: Ref) -> Ref {
    core::ptr::null()
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleGetValueForInfoDictionaryKey(b: Ref, key: Ref) -> Ref {
    if key.is_null() {
        return core::ptr::null();
    }
    crate::dict::lookup(get(b).info, key).unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleCopyBundleURL(b: Ref) -> Ref {
    base::retain(get(b).url)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleCopyExecutableURL(b: Ref) -> Ref {
    let bundle = get(b);
    let name = match info_value(b, "CFBundleExecutable") {
        Some(n) => n,
        None if bundle.layout == Layout::Flat && registry().main == b => {
            let exe = crate::sys::executable_path();
            String::from(paths::last_component(&exe))
        }
        None => return core::ptr::null(),
    };
    crate::url::from_path(core::ptr::null(), &paths::executable(&bundle.dir, bundle.layout, &name), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleCopyResourcesDirectoryURL(b: Ref) -> Ref {
    let bundle = get(b);
    crate::url::make(core::ptr::null(), paths::resources_relative(bundle.layout), bundle.url)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleCopyResourceURL(b: Ref, name: Ref, ext: Ref, sub: Ref) -> Ref {
    let bundle = get(b);
    let Some(name) = opt_text(name) else { return core::ptr::null() };
    if bundle.layout == Layout::Flat && registry().main == b && bundle.info_is_empty() {
        // A program outside a bundle has no resources.
        return core::ptr::null();
    }
    find_resource(&resources(bundle), &name, opt_text(ext).as_deref(), opt_text(sub).as_deref())
}

impl Bundle {
    fn info_is_empty(&self) -> bool {
        crate::dict::count(self.info) == 0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleCopyResourceURLInDirectory(dir_url: Ref, name: Ref, ext: Ref, sub: Ref) -> Ref {
    let Some(dir) = crate::url::path(dir_url) else { return core::ptr::null() };
    let layout =
        if crate::sys::is_dir(&paths::join(&dir, "Contents")) == Some(true) { Layout::Contents } else { Layout::Flat };
    let Some(name) = opt_text(name) else { return core::ptr::null() };
    find_resource(&paths::resources_dir(&dir, layout), &name, opt_text(ext).as_deref(), opt_text(sub).as_deref())
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleCopyResourceURLsOfType(b: Ref, ext: Ref, sub: Ref) -> Ref {
    let bundle = get(b);
    let res = resources(bundle);
    let ext = opt_text(ext);
    let sub = opt_text(sub);
    let mut dirs = Vec::new();
    let with_sub = |d: String| match &sub {
        Some(s) if !s.is_empty() => paths::join(&d, s),
        _ => d,
    };
    dirs.push(with_sub(res.clone()));
    for lang in paths::LANGUAGES {
        dirs.push(with_sub(paths::join(&res, &alloc::format!("{lang}.lproj"))));
    }
    let mut urls: Vec<Ref> = Vec::new();
    if !(bundle.layout == Layout::Flat && registry().main == b && bundle.info_is_empty()) {
        for d in dirs {
            for name in crate::sys::list_dir(&d) {
                let matches = match &ext {
                    Some(e) if !e.is_empty() => name.ends_with(&alloc::format!(".{}", e.trim_start_matches('.'))),
                    _ => true,
                };
                if matches {
                    let p = paths::join(&d, &name);
                    let dir = crate::sys::is_dir(&p) == Some(true);
                    urls.push(crate::url::from_path(core::ptr::null(), &p, dir));
                }
            }
        }
    }
    let a = crate::array::from_refs(core::ptr::null(), &urls, &crate::array::TYPE_CALLBACKS, false);
    urls.into_iter().for_each(base::release);
    a
}

/// # Safety
/// `ty` and `creator` are null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFBundleGetPackageInfo(b: Ref, ty: *mut u32, creator: *mut u32) {
    let unknown = paths::four_cc("????").unwrap_or(0);
    let t = info_value(b, "CFBundlePackageType").and_then(|s| paths::four_cc(&s)).unwrap_or(
        if get(b).layout == Layout::Contents && !get(b).info_is_empty() {
            unknown
        } else {
            paths::four_cc("BNDL").unwrap_or(0)
        },
    );
    let c = info_value(b, "CFBundleSignature").and_then(|s| paths::four_cc(&s)).unwrap_or(unknown);
    // SAFETY: the caller's contract.
    unsafe {
        if !ty.is_null() {
            *ty = t;
        }
        if !creator.is_null() {
            *creator = c;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleGetVersionNumber(b: Ref) -> u32 {
    let k = string::from_str("CFBundleNumericVersion");
    let v = crate::dict::lookup(get(b).info, k);
    base::release(k);
    match v {
        Some(n) if base::type_of(n) == id::NUMBER => match crate::number::value(n) {
            poly_cf::number::Num::Int(i) => i as u32,
            poly_cf::number::Num::Float(_) => 0,
        },
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleGetDevelopmentRegion(b: Ref) -> Ref {
    CFBundleGetValueForInfoDictionaryKey(b, kCFBundleDevelopmentRegionKey.0 as Ref)
}

#[unsafe(no_mangle)]
pub extern "C" fn CFBundleIsExecutableLoaded(b: Ref) -> Boolean {
    (registry().main == b) as Boolean
}
