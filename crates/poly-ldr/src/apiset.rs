//! API sets: Poly's mapping from API-set contract names to the DLLs that
//! host them (ADR 0071).
//!
//! Learn ("Windows API sets", "API set loader operation"): a name starting
//! with `api-` or `ext-` names a contract, not a file; the loader resolves
//! it to a host DLL before searching for files. `ext-` contracts are
//! optional extensions that may be missing (`ApiSetQueryApiSetPresence`).
//! Which DLL hosts which contract is the system's business; the host DLLs
//! here are Poly's own (ADR 0055), so the table is ours. A name is matched
//! without its `.dll` and without its last hyphenated number (the version's
//! last part: `api-ms-win-core-file-l1-2-4` → `api-ms-win-core-file-l1-2`),
//! ignoring case, against the longest matching prefix below.

/// Contract prefixes and their host DLLs; the first (longest) match wins.
/// `None`: the contract family is not hosted (absent).
pub const TABLE: &[(&str, Option<&str>)] = &[
    ("api-ms-win-core-rtlsupport-", Some("ntdll.dll")),
    ("api-ms-win-crt-", Some("ucrtbase.dll")),
    ("api-ms-win-security-base-", Some("kernelbase.dll")),
    ("api-ms-win-eventing-provider-", Some("kernelbase.dll")),
    ("api-ms-win-core-", Some("kernelbase.dll")),
    ("api-ms-win-security-", Some("advapi32.dll")),
    ("api-ms-win-eventing-", Some("advapi32.dll")),
    ("ext-ms-", None),
];

/// What an API-set name resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiSet {
    /// Not an API-set name: a file name to search for.
    NotApiSet,
    /// A contract hosted by this DLL.
    Host(&'static str),
    /// A contract Poly does not host.
    Absent,
}

fn lower(c: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c }
}

/// The contract name to match: lower case, without `.dll`, without the
/// last `-<number>`.
fn contract(name: &[u16]) -> alloc::vec::Vec<u16> {
    let mut n: alloc::vec::Vec<u16> = name.iter().map(|&c| lower(c)).collect();
    let dll: alloc::vec::Vec<u16> = ".dll".encode_utf16().collect();
    if n.ends_with(&dll) {
        n.truncate(n.len() - 4);
    }
    if let Some(dash) = n.iter().rposition(|&c| c == u16::from(b'-'))
        && dash + 1 < n.len()
        && n[dash + 1..].iter().all(|&c| (b'0' as u16..=b'9' as u16).contains(&c))
    {
        n.truncate(dash);
    }
    n
}

fn starts(name: &[u16], prefix: &str) -> bool {
    let p: alloc::vec::Vec<u16> = prefix.encode_utf16().collect();
    name.starts_with(&p)
}

/// Resolve a module name (a base name, no path) that may be an API set.
pub fn resolve(name: &[u16]) -> ApiSet {
    let low: alloc::vec::Vec<u16> = name.iter().map(|&c| lower(c)).collect();
    if !starts(&low, "api-") && !starts(&low, "ext-") {
        return ApiSet::NotApiSet;
    }
    let c = contract(name);
    for &(prefix, host) in TABLE {
        if starts(&c, prefix) {
            return host.map_or(ApiSet::Absent, ApiSet::Host);
        }
    }
    ApiSet::Absent
}

/// `ApiSetQueryApiSetPresence`: whether a contract is hosted.
pub fn present(name: &[u16]) -> bool {
    matches!(resolve(name), ApiSet::Host(_))
}

extern crate alloc;

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> alloc::vec::Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn contracts_resolve_to_their_hosts() {
        assert_eq!(resolve(&w("api-ms-win-crt-runtime-l1-1-0.dll")), ApiSet::Host("ucrtbase.dll"));
        assert_eq!(resolve(&w("API-MS-WIN-CRT-STDIO-L1-1-0")), ApiSet::Host("ucrtbase.dll"));
        assert_eq!(resolve(&w("api-ms-win-core-file-l1-2-4.dll")), ApiSet::Host("kernelbase.dll"));
        assert_eq!(resolve(&w("api-ms-win-core-rtlsupport-l1-1-0.dll")), ApiSet::Host("ntdll.dll"));
        assert_eq!(resolve(&w("api-ms-win-security-base-l1-2-0.dll")), ApiSet::Host("kernelbase.dll"));
        assert_eq!(resolve(&w("api-ms-win-security-lsalookup-l1-1-0.dll")), ApiSet::Host("advapi32.dll"));
        assert_eq!(resolve(&w("ext-ms-win-ntuser-window-l1-1-0.dll")), ApiSet::Absent);
        assert_eq!(resolve(&w("api-ms-win-unknown-l1-1-0.dll")), ApiSet::Absent);
        assert_eq!(resolve(&w("kernel32.dll")), ApiSet::NotApiSet);
        assert_eq!(resolve(&w("apiary.dll")), ApiSet::NotApiSet);
        assert!(present(&w("api-ms-win-crt-math-l1-1-0")));
        assert!(!present(&w("ext-ms-win-foo-l1-1-0")));
    }

    #[test]
    fn the_last_version_number_is_ignored() {
        assert_eq!(contract(&w("api-ms-win-core-file-l1-2-4.dll")), w("api-ms-win-core-file-l1-2"));
        assert_eq!(contract(&w("api-ms-win-core-file-l1-2-x")), w("api-ms-win-core-file-l1-2-x"));
        assert_eq!(contract(&w("api-")), w("api-"));
    }

    #[test]
    fn longest_prefixes_come_first() {
        for (i, (a, _)) in TABLE.iter().enumerate() {
            for (b, _) in &TABLE[i + 1..] {
                assert!(!b.starts_with(a), "{b} is shadowed by {a}");
            }
        }
    }
}
