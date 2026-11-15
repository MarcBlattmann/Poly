//! The fail-only exports of `msvcp140.dll` (ADR 0155, ADR 0199): every x64 name the DLL
//! exports that Poly does not implement yet. A function logs `stub: msvcp140!<name>` and
//! returns 0; an exported variable is a zeroed object, so that a program that imports it
//! loads and finds writable memory there.
//!
//! The names come from a list checked in beside the DLL (one decorated name per line, `#`
//! comments), made from the coverage map's x64 names minus the implemented ones. A Visual
//! C++ decorated name is a variable when it is not a function: a function's encoding always
//! ends with its exception specification `Z`, a variable's with its storage class (`A`, `B`,
//! ...) and a virtual table's (`??_7...6B@`) or virtual base table's (`??_8...7B@`) with `@`.
//! Sizes: no sheet gives the size of these objects yet, so each is a generous block: 256
//! bytes for a virtual table, 512 for a standard stream object (`std::cout` and friends),
//! 64 for everything else (locale ids, counters, pointers).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// Whether a decorated name is a variable (data), not a function.
pub fn is_data(name: &str) -> bool {
    name.starts_with('?') && !name.ends_with('Z')
}

/// The size of the zeroed object of an exported variable.
pub fn data_size(name: &str) -> usize {
    let stream = ["@std@@3V?$basic_istream@", "@std@@3V?$basic_ostream@", "@std@@3V?$basic_iostream@"];
    if name.starts_with("??_7") || name.starts_with("??_8") {
        256
    } else if stream.iter().any(|s| name.contains(s)) {
        512
    } else {
        64
    }
}

/// The names of a list file: one per line; blank lines and `#` comments skipped.
pub fn parse_list(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
}

/// Whether a name can be written into a Rust string literal and an export table as it is.
pub fn is_plain(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
}

/// The Rust source of the fail-only exports of `names` for the DLL `dll`: one
/// `#[unsafe(export_name = ...)]` function or static each. The statics are of the type
/// `crate::Zeroed<N>` the DLL defines (an interior-mutable zeroed block, so it is writable; `Zeroed::new()`).
/// Panics on a name that [`is_plain`] refuses.
pub fn stubs_rust(dll: &str, names: &[&str]) -> String {
    let mut s = String::new();
    for (i, name) in names.iter().enumerate() {
        assert!(is_plain(name), "export name {name:?}");
        if is_data(name) {
            let _ = writeln!(
                s,
                "#[unsafe(export_name = \"{name}\")]\npub static POLY_DATA_{i}: crate::Zeroed<{}> = crate::Zeroed::new();",
                data_size(name)
            );
        } else {
            let _ = writeln!(
                s,
                "#[unsafe(export_name = \"{name}\")]\npub extern \"system\" fn poly_stub_{i}() -> u64 {{\n    poly_win32_rt::unimplemented_export(\"stub: {dll}!{name}\", false, 0)\n}}"
            );
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_or_function() {
        assert!(!is_data("?_Xlength_error@std@@YAXPEBD@Z"));
        assert!(!is_data("??0_Lockit@std@@QEAA@H@Z"));
        assert!(!is_data("_Mtx_init"), "C names are functions");
        assert!(is_data("?_Id_cnt@id@locale@std@@0HA"));
        assert!(is_data("?id@?$ctype@D@std@@2V0locale@2@A"));
        assert!(is_data("?_Raise_handler@std@@3P6AXAEBVexception@stdext@@@ZEA"));
        assert!(is_data("??_7ios_base@std@@6B@"));
        assert!(is_data("?_BADOFF@std@@3_JB"));
    }

    #[test]
    fn sizes() {
        assert_eq!(data_size("??_7ios_base@std@@6B@"), 256);
        assert_eq!(data_size("?cout@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A"), 512);
        assert_eq!(data_size("?wcin@std@@3V?$basic_istream@_WU?$char_traits@_W@std@@@1@A"), 512);
        assert_eq!(data_size("?_Ptr_cout@std@@3PEAV?$basic_ostream@DU?$char_traits@D@std@@@1@EA"), 64, "a pointer");
        assert_eq!(data_size("?_Id_cnt@id@locale@std@@0HA"), 64);
    }

    #[test]
    fn list_and_source() {
        let names = parse_list("# comment\n\n_Mtx_timedlock\r\n?_Id_cnt@id@locale@std@@0HA\n");
        assert_eq!(names, ["_Mtx_timedlock", "?_Id_cnt@id@locale@std@@0HA"]);
        let src = stubs_rust("msvcp140", &names);
        assert!(
            src.contains("#[unsafe(export_name = \"_Mtx_timedlock\")]\npub extern \"system\" fn poly_stub_0() -> u64")
        );
        assert!(src.contains("\"stub: msvcp140!_Mtx_timedlock\""));
        assert!(src.contains("pub static POLY_DATA_1: crate::Zeroed<64> = crate::Zeroed::new();"));
        assert!(!is_plain("a\"b") && !is_plain("") && !is_plain("a b"));
    }
}
