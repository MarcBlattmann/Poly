//! The standard exception classes the `_X*` helpers and `_Throw_Cpp_error` throw, and the
//! records a program's code and `vcruntime140` read about them.
//!
//! **Objects.** Every class here is a `std::exception`: a virtual table pointer followed by
//! `__std_exception_data { const char* _What; bool _DoFree; }` (`vcruntime_exception.h`, as
//! `poly_cxxeh::typeinfo` has it), 24 bytes; `std::system_error` adds its `error_code`
//! (`int` value, then the category pointer at 32), 40 bytes. The virtual table has the
//! standard's two virtual functions of `std::exception` in declaration order (C++
//! [exception]): the destructor (Visual C++'s deleting destructor `void* (this, unsigned
//! flags)`, bit 0 = free the memory) and `what()`; the derived classes add none.
//!
//! **Records** (x64, image-relative addresses: `.rva`): beside each virtual table, the slot
//! before its first entry points at the Complete Object Locator, which names the class's
//! TypeDescriptor and Class Hierarchy Descriptor (`__RTDynamicCast.yaml`,
//! `rtti_locator_layout`); the throw descriptors are `_CxxThrowException.yaml`'s: a ThrowInfo
//! names the destructor and a CatchableTypeArray that lists the class and then each base
//! (all public and unambiguous: single inheritance), each a CatchableType of 28 bytes. The
//! TypeDescriptors are writable (their second field caches the readable name); their first
//! field (`type_info`'s virtual table) is null, which the throw sheet observed to be fine.
//!
//! [`records_asm`] writes all of it as assembly for the DLL (`global_asm!`), whose Rust
//! functions it names: `poly_msvcp_exception_dtor` (deleting destructor),
//! `poly_msvcp_exception_what`, `poly_msvcp_exception_destroy` (the ThrowInfo destructor) and
//! `poly_msvcp_copy_<class>` (the copy constructor of each class). It defines the globals
//! `poly_msvcp_vt_<class>` (the virtual table, first entry) and `poly_msvcp_ti_<class>`.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// `sizeof(std::exception)` and of the classes that add no field.
pub const EXCEPTION_SIZE: u32 = 24;
/// `sizeof(std::system_error)`.
pub const SYSTEM_ERROR_SIZE: u32 = 40;
/// Offset of `system_error`'s `error_code` (value, then the category pointer 8 bytes later).
pub const SYSTEM_ERROR_CODE: usize = 24;
/// CatchableType property of `std::bad_alloc` (`_CxxThrowException.yaml`).
pub const CT_IS_BAD_ALLOC: u32 = 0x10;
/// Base class descriptor attribute "has a class hierarchy descriptor" (always set).
pub const BCD_HAS_CHD: u32 = 0x40;

/// One class: its name in `std`, its direct base, its size and its CatchableType properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Class {
    pub name: &'static str,
    pub base: Option<&'static str>,
    pub size: u32,
    pub properties: u32,
}

const fn class(name: &'static str, base: Option<&'static str>, size: u32, properties: u32) -> Class {
    Class { name, base, size, properties }
}

/// The classes, bases before the classes derived from them.
pub const CLASSES: &[Class] = &[
    class("exception", None, EXCEPTION_SIZE, 0),
    class("bad_alloc", Some("exception"), EXCEPTION_SIZE, CT_IS_BAD_ALLOC),
    class("bad_function_call", Some("exception"), EXCEPTION_SIZE, 0),
    class("logic_error", Some("exception"), EXCEPTION_SIZE, 0),
    class("length_error", Some("logic_error"), EXCEPTION_SIZE, 0),
    class("out_of_range", Some("logic_error"), EXCEPTION_SIZE, 0),
    class("invalid_argument", Some("logic_error"), EXCEPTION_SIZE, 0),
    class("runtime_error", Some("exception"), EXCEPTION_SIZE, 0),
    class("overflow_error", Some("runtime_error"), EXCEPTION_SIZE, 0),
    class("system_error", Some("runtime_error"), SYSTEM_ERROR_SIZE, 0),
];

/// The class of that name.
pub fn find(name: &str) -> Option<&'static Class> {
    CLASSES.iter().find(|c| c.name == name)
}

/// The class and its bases, the class first (the order of a CatchableTypeArray and of a base
/// class array).
pub fn chain(name: &str) -> Vec<&'static Class> {
    let mut out = Vec::new();
    let mut next = find(name);
    while let Some(c) = next {
        out.push(c);
        next = c.base.and_then(find);
    }
    out
}

/// The decorated name a TypeDescriptor holds: `.?AV<name>@std@@` (a class in `std`).
pub fn decorated_name(name: &str) -> String {
    let mut s = String::new();
    let _ = write!(s, ".?AV{name}@std@@");
    s
}

/// The assembly of every class's TypeDescriptor, RTTI records, virtual table and throw
/// descriptors (see the module documentation); it leaves the assembler in `.text`.
pub fn records_asm() -> String {
    let mut s = String::new();
    // TypeDescriptors: writable.
    s.push_str("    .data\n");
    for c in CLASSES {
        let n = c.name;
        let _ = writeln!(
            s,
            "    .p2align 4\npoly_msvcp_td_{n}:\n    .quad 0\n    .quad 0\n    .asciz \"{}\"",
            decorated_name(n)
        );
    }
    s.push_str("    .section .rdata,\"dr\"\n");
    for c in CLASSES {
        let n = c.name;
        let bases = chain(n);
        // Base class descriptor of this class (shared by every array it appears in).
        let _ = writeln!(
            s,
            "    .p2align 2\npoly_msvcp_bcd_{n}:\n    .rva poly_msvcp_td_{n}\n    .long {}\n    .long 0, -1, 0\n    .long {BCD_HAS_CHD:#x}\n    .rva poly_msvcp_chd_{n}",
            bases.len() - 1
        );
        // Class hierarchy descriptor and its base class array.
        let _ = writeln!(
            s,
            "poly_msvcp_chd_{n}:\n    .long 0, 0, {}\n    .rva poly_msvcp_bca_{n}\npoly_msvcp_bca_{n}:",
            bases.len()
        );
        for b in &bases {
            let _ = writeln!(s, "    .rva poly_msvcp_bcd_{}", b.name);
        }
        // Complete object locator, then the virtual table it precedes.
        let _ = writeln!(
            s,
            "poly_msvcp_col_{n}:\n    .long 1, 0, 0\n    .rva poly_msvcp_td_{n}\n    .rva poly_msvcp_chd_{n}\n    .rva poly_msvcp_col_{n}"
        );
        let _ = writeln!(
            s,
            "    .p2align 3\n    .quad poly_msvcp_col_{n}\n    .globl poly_msvcp_vt_{n}\npoly_msvcp_vt_{n}:\n    .quad poly_msvcp_exception_dtor\n    .quad poly_msvcp_exception_what"
        );
        // Catchable type, the array of the class and its bases, the ThrowInfo.
        let _ = writeln!(
            s,
            "    .p2align 2\npoly_msvcp_ct_{n}:\n    .long {:#x}\n    .rva poly_msvcp_td_{n}\n    .long 0, -1, 0\n    .long {}\n    .rva poly_msvcp_copy_{n}",
            c.properties, c.size
        );
        let _ = writeln!(s, "poly_msvcp_cta_{n}:\n    .long {}", bases.len());
        for b in &bases {
            let _ = writeln!(s, "    .rva poly_msvcp_ct_{}", b.name);
        }
        let _ = writeln!(
            s,
            "    .globl poly_msvcp_ti_{n}\npoly_msvcp_ti_{n}:\n    .long 0\n    .rva poly_msvcp_exception_destroy\n    .long 0\n    .rva poly_msvcp_cta_{n}"
        );
    }
    s.push_str("    .text\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn chains() {
        let names = |n: &str| chain(n).iter().map(|c| c.name).collect::<Vec<_>>();
        assert_eq!(names("length_error"), vec!["length_error", "logic_error", "exception"]);
        assert_eq!(names("system_error"), vec!["system_error", "runtime_error", "exception"]);
        assert_eq!(names("bad_alloc"), vec!["bad_alloc", "exception"]);
        assert_eq!(names("exception"), vec!["exception"]);
        assert!(chain("nothing").is_empty());
    }

    #[test]
    fn bases_come_first_and_exist() {
        for (i, c) in CLASSES.iter().enumerate() {
            if let Some(b) = c.base {
                assert!(CLASSES[..i].iter().any(|d| d.name == b), "{} before its base {b}", c.name);
            }
            assert_eq!(c.size, if c.name == "system_error" { SYSTEM_ERROR_SIZE } else { EXCEPTION_SIZE });
        }
        assert_eq!(find("bad_alloc").unwrap().properties, CT_IS_BAD_ALLOC);
        assert_eq!(find("length_error").unwrap().properties, 0);
    }

    #[test]
    fn names() {
        assert_eq!(decorated_name("length_error"), ".?AVlength_error@std@@");
    }

    #[test]
    fn records() {
        let a = records_asm();
        assert!(a.ends_with("    .text\n"));
        // length_error: three bases in the hierarchy and three catchable types.
        let after = |label: &str| a.split(label).nth(1).unwrap().lines().take(4).collect::<Vec<_>>();
        assert_eq!(after("poly_msvcp_chd_length_error:\n")[0], "    .long 0, 0, 3");
        assert_eq!(
            after("poly_msvcp_cta_length_error:\n"),
            vec![
                "    .long 3",
                "    .rva poly_msvcp_ct_length_error",
                "    .rva poly_msvcp_ct_logic_error",
                "    .rva poly_msvcp_ct_exception"
            ]
        );
        assert!(a.contains("poly_msvcp_bcd_length_error:\n    .rva poly_msvcp_td_length_error\n    .long 2\n"));
        assert!(a.contains("poly_msvcp_ct_bad_alloc:\n    .long 0x10\n"));
        assert!(a.contains("    .long 40\n    .rva poly_msvcp_copy_system_error"));
        assert!(a.contains("    .asciz \".?AVsystem_error@std@@\""));
        // Every label is defined once.
        for c in CLASSES {
            for kind in ["td", "bcd", "chd", "bca", "col", "vt", "ct", "cta", "ti"] {
                let label = alloc::format!("poly_msvcp_{kind}_{}:\n", c.name);
                assert_eq!(a.matches(&label).count(), 1, "{label}");
            }
        }
    }
}
