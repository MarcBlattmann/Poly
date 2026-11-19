//! The registration of type libraries (Learn: "Type Library Registration"; sheets
//! `RegisterTypeLib`, `UnRegisterTypeLib`, `LoadRegTypeLib`, `QueryPathOfRegTypeLib`,
//! `LoadTypeLibEx`): the keys under `SOFTWARE\Classes` and how a request picks a registration.
//!
//! ```text
//! TypeLib\{LIBID}\<major>.<minor>          (default) = the library's doc string
//!                 \<lcid hex>\win64|win32   (default) = the file's path (with `\N` for a resource)
//!                 \FLAGS                    (default) = the library flags, decimal
//!                 \HELPDIR                  (default) = the help directory
//! Interface\{IID}                           (default) = the interface's name
//!               \ProxyStubClsid32           (default) = {00020424-0000-0000-C000-000000000046}
//!               \TypeLib                    (default) = {LIBID}, Version = <major>.<minor>
//! ```

use super::*;
use alloc::format;

/// The automation marshaller's class, `CLSID_PSOAInterface`.
pub const CLSID_PSOAINTERFACE: Guid =
    Guid { data1: 0x0002_0424, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// The older `CLSID_PSDispatch` (the proxy/stub class of `IDispatch` itself).
pub const CLSID_PSDISPATCH: Guid =
    Guid { data1: 0x0002_0420, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// The braced upper-case text of a GUID.
pub fn guid_text(g: &Guid) -> String {
    g.to_text().iter().map(|&u| u as u8 as char).collect()
}

/// The name of a version key: `<major>.<minor>`, decimal (sheet `RegisterTypeLib`).
pub fn version_key(major: u16, minor: u16) -> String {
    format!("{major}.{minor}")
}

/// A version key's numbers.
pub fn parse_version(name: &str) -> Option<(u16, u16)> {
    let (a, b) = name.split_once('.')?;
    let num = |s: &str| -> Option<u16> {
        if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    Some((num(a)?, num(b)?))
}

/// The registered version a request for `major.minor` loads: the same major and, of those, the
/// lowest minor at least the requested one (sheet `LoadRegTypeLib`, rule `version_rule`).
pub fn pick_version(available: &[(u16, u16)], major: u16, minor: u16) -> Option<(u16, u16)> {
    available.iter().copied().filter(|&(ma, mi)| ma == major && mi >= minor).min_by_key(|&(_, mi)| mi)
}

/// The name of an LCID key: hex, lower case, without leading zeros (`0`, `409`).
pub fn lcid_key(lcid: u32) -> String {
    format!("{lcid:x}")
}

/// An LCID key's number.
pub fn parse_lcid(name: &str) -> Option<u32> {
    if name.is_empty() || name.len() > 8 || !name.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(name, 16).ok()
}

/// The registered LCID a request loads: the same one, else the neutral one (sheet
/// `QueryPathOfRegTypeLib`, rule `lcid_rule`).
pub fn pick_lcid(available: &[u32], want: u32) -> Option<u32> {
    if available.contains(&want) {
        Some(want)
    } else if available.contains(&0) {
        Some(0)
    } else {
        None
    }
}

/// The platform key of a system kind (`None`: not one `UnRegisterTypeLib` accepts, which is
/// `E_INVALIDARG` for `SYS_MAC` and unknown kinds).
pub fn platform_key(syskind: i32) -> Option<&'static str> {
    match syskind {
        SYS_WIN16 => Some("win16"),
        SYS_WIN32 => Some("win32"),
        SYS_WIN64 => Some("win64"),
        _ => None,
    }
}

/// The platform keys a 64-bit process looks for, in order (sheet `QueryPathOfRegTypeLib`, rule
/// `win32_fallback`).
pub const LOOKUP_PLATFORMS: [&str; 2] = ["win64", "win32"];

/// A registered or given path split into a file and a resource number: `file.dll\4` is resource 4
/// of `file.dll` when the last part is a plain decimal number (leading zeros allowed) of at most
/// 65535. `None` when the path has no such ending (then the whole path is the file and resource
/// 1 is meant if it is an executable image).
pub fn split_resource(path: &str) -> Option<(&str, u32)> {
    let (head, tail) = path.rsplit_once('\\')?;
    if head.is_empty() || tail.is_empty() || tail.len() > 9 || !tail.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u32 = tail.parse().ok()?;
    (n <= 0xFFFF).then_some((head, n))
}

/// The directory of a file path (without the resource number and the last backslash).
pub fn directory_of(path: &str) -> &str {
    let file = split_resource(path).map_or(path, |(f, _)| f);
    file.rsplit_once('\\').map_or("", |(d, _)| d)
}

/// A registry value to write: a key below `SOFTWARE\Classes`, the value's name (`None`: the
/// default value) and the string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Write {
    pub key: String,
    pub name: Option<&'static str>,
    pub value: String,
}

fn w(key: String, name: Option<&'static str>, value: String) -> Write {
    Write { key, name, value }
}

/// Whether type `t` gets an `Interface` key: an interface flagged `oleautomation`, or a dual one
/// (made as an interface or as a dispatch type).
pub fn registers_interface(lib: &Library, t: usize) -> bool {
    let ty = &lib.types[t];
    ty.guid != Guid::default()
        && (lib.is_dual(t) || (ty.kind == TKIND_INTERFACE && ty.flags & TYPEFLAG_FOLEAUTOMATION != 0))
}

/// Everything `RegisterTypeLib(lib, path, help_dir)` writes.
pub fn register_writes(lib: &Library, path: &str, help_dir: Option<&str>) -> Vec<Write> {
    let id = guid_text(&lib.guid);
    let ver = version_key(lib.major, lib.minor);
    let base = format!("TypeLib\\{id}\\{ver}");
    let platform = if lib.syskind == SYS_WIN32 { "win32" } else { "win64" };
    let mut out = vec![
        w(base.clone(), None, lib.doc.clone()),
        w(format!("{base}\\{}\\{platform}", lcid_key(lib.lcid)), None, String::from(path)),
        w(format!("{base}\\FLAGS"), None, format!("{}", lib.flags & !LIBFLAG_FHASDISKIMAGE)),
        w(format!("{base}\\HELPDIR"), None, String::from(help_dir.unwrap_or_else(|| directory_of(path)))),
    ];
    for t in 0..lib.types.len() {
        if !registers_interface(lib, t) {
            continue;
        }
        let ty = &lib.types[t];
        let key = format!("Interface\\{}", guid_text(&ty.guid));
        out.push(w(key.clone(), None, ty.name.clone()));
        out.push(w(format!("{key}\\ProxyStubClsid32"), None, guid_text(&CLSID_PSOAINTERFACE)));
        out.push(w(format!("{key}\\TypeLib"), None, id.clone()));
        out.push(w(format!("{key}\\TypeLib"), Some("Version"), ver.clone()));
    }
    out
}

use alloc::vec;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_pick_the_lowest_minor_at_least_asked() {
        let reg = [(1, 0), (1, 2), (2, 0)];
        assert_eq!(pick_version(&reg, 1, 0), Some((1, 0)));
        assert_eq!(pick_version(&reg, 1, 1), Some((1, 2)));
        assert_eq!(pick_version(&reg, 1, 2), Some((1, 2)));
        assert_eq!(pick_version(&reg, 1, 3), None);
        assert_eq!(pick_version(&reg, 2, 0), Some((2, 0)));
        assert_eq!(pick_version(&reg, 2, 1), None);
        assert_eq!(pick_version(&reg, 3, 0), None);
        assert_eq!(pick_version(&reg, 0, 5), None);
        assert_eq!(pick_version(&reg, 1, 65535), None);
        assert_eq!(parse_version("1.5"), Some((1, 5)));
        assert_eq!(parse_version("1."), None);
        assert_eq!(parse_version("x.0"), None);
    }

    #[test]
    fn lcids_match_exactly_then_neutral() {
        assert_eq!(pick_lcid(&[0x409], 0x409), Some(0x409));
        for l in [0, 0x809, 0x09, 0x407, 0x400, 0x800] {
            assert_eq!(pick_lcid(&[0x409], l), None);
        }
        for l in [0, 0x409, 0x400] {
            assert_eq!(pick_lcid(&[0], l), Some(0));
        }
        assert_eq!(pick_lcid(&[0, 0x409, 0x407], 0x407), Some(0x407));
        assert_eq!(pick_lcid(&[0, 0x409], 0x809), Some(0));
        assert_eq!(lcid_key(0x409), "409");
        assert_eq!(lcid_key(0), "0");
        assert_eq!(parse_lcid("409"), Some(0x409));
        assert_eq!(parse_lcid("FLAGS"), None);
    }

    #[test]
    fn resource_paths() {
        assert_eq!(split_resource(r"C:\a\u.exe\4"), Some((r"C:\a\u.exe", 4)));
        assert_eq!(split_resource(r"C:\a\u.exe\04"), Some((r"C:\a\u.exe", 4)));
        assert_eq!(split_resource(r"C:\a\u.exe\65540"), None);
        assert_eq!(split_resource(r"C:\a\u.exe\x"), None);
        assert_eq!(split_resource(r"C:\a\u.exe\-1"), None);
        assert_eq!(split_resource(r"C:\a\u.exe\4\"), None);
        assert_eq!(split_resource(r"C:\a\u.exe"), None);
        assert_eq!(directory_of(r"C:\a\u.exe\4"), r"C:\a");
        assert_eq!(directory_of(r"C:\a\x.tlb"), r"C:\a");
        assert_eq!(platform_key(SYS_MAC), None);
        assert_eq!(platform_key(9), None);
    }
}
