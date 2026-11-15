//! `type_info` and `std::exception` helpers of `vcruntime140.dll` (ADR 0199) that work on
//! bytes, not on the process.
//!
//! The layouts are the public `vcruntime_typeinfo.h` and `vcruntime_exception.h`
//! declarations: `__std_type_info_data { const char* _UndecoratedName; const char
//! _DecoratedName[1]; }` (a `type_info` is its virtual table pointer followed by this, so the
//! decorated name is at offset 16 of a type descriptor, as `_CxxThrowException.yaml` has it)
//! and `__std_exception_data { const char* _What; bool _DoFree; }`.
//!
//! - **Compare** (`__std_type_info_compare`, which `type_info::operator==` and `before` use):
//!   the same descriptor is equal; otherwise the decorated names decide, byte by byte as
//!   unsigned characters (`strcmp`), so the same type described in two modules is equal and
//!   every other pair has one order for the whole process.
//! - **Hash** (`__std_type_info_hash`, `type_info::hash_code`): equal types have equal
//!   hashes (C++17 [type.info]); Poly's is 64-bit FNV-1a over the decorated name. Which
//!   function Windows uses is to be observed (ADR 0199); no program can rely on the value
//!   across processes.
//! - **Exception text** (`__std_exception_copy`): a copy of an exception whose text it owns
//!   owns a copy of the text; one that points at a literal points at the same literal.

/// `strcmp` of two NUL-free names: negative, zero or positive.
pub fn compare_names(a: &[u8], b: &[u8]) -> i32 {
    match a.cmp(b) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// 64-bit FNV-1a of a decorated name (without its NUL).
pub fn hash_name(name: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    name.iter().fold(OFFSET, |h, &b| (h ^ u64::from(b)).wrapping_mul(PRIME))
}

/// What `__std_exception_copy` does with the source's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCopy {
    /// Point at the same text, not owned (a literal, or no text).
    Share,
    /// Allocate `len + 1` bytes, copy the text with its NUL, own it.
    Duplicate { len: usize },
}

/// The copy of an exception's text: duplicated if the source owns non-null text.
pub fn copy_text(what_is_null: bool, do_free: bool, len: usize) -> TextCopy {
    if do_free && !what_is_null { TextCopy::Duplicate { len } } else { TextCopy::Share }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_compare_like_strcmp() {
        assert_eq!(compare_names(b".?AVA@@", b".?AVA@@"), 0);
        assert_eq!(compare_names(b".?AVA@@", b".?AVB@@"), -1);
        assert_eq!(compare_names(b".?AVB@@", b".?AVA@@"), 1);
        assert_eq!(compare_names(b".H", b".HA"), -1, "a prefix sorts first");
        assert_eq!(compare_names(b".?AV\xe4@@", b".?AVz@@"), 1, "bytes are unsigned");
    }

    #[test]
    fn hash_is_fnv1a_and_follows_the_name() {
        assert_eq!(hash_name(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(hash_name(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(hash_name(b".?AVA@@"), hash_name(b".?AVA@@"));
        assert_ne!(hash_name(b".?AVA@@"), hash_name(b".?AVB@@"));
    }

    #[test]
    fn owned_text_is_duplicated() {
        assert_eq!(copy_text(false, true, 5), TextCopy::Duplicate { len: 5 });
        assert_eq!(copy_text(false, false, 5), TextCopy::Share);
        assert_eq!(copy_text(true, true, 0), TextCopy::Share);
    }
}
