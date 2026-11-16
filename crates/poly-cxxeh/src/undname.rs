//! The readable name of a type from its Visual C++ decorated name, as `type_info::name()` gives
//! it (`vcruntime140.dll`'s `__std_type_info_name`; the spec sheet of that name, whose
//! `rules` and recorded outputs fix the spelling: `int * __ptr64`, `int const * __ptr64`,
//! `struct Tpl<struct S * __ptr64>`, `int (__cdecl*)(int,char)`, `int (* __ptr64)[4]`, ...).
//!
//! The decorated names are the type descriptors' (`.H`, `.PEAH`, `.?AUS@@`,
//! `.?AV?$Tpl@H@@`, `.$$BY03H`): a type code, pointers and references with their `__ptr64`
//! and cv letters, qualified names written innermost first and ended by `@`, back references
//! (digits) to earlier names and to earlier argument types, templates (`?$name@args@`),
//! function and member types, arrays. What cannot be read gives `None`; the caller then shows
//! the decorated name.

use alloc::string::String;
use alloc::vec::Vec;

/// A type as a declaration splits it: `left` before the declarator, `right` after it
/// (`int (* __ptr64` + `)[4]`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Ty {
    left: String,
    right: String,
    /// Arrays and functions need parentheses around a pointer declarator.
    compound: bool,
    /// A function type: a pointer to it carries no `__ptr64`.
    function: bool,
}

impl Ty {
    fn plain(s: &str) -> Ty {
        Ty { left: String::from(s), ..Ty::default() }
    }

    fn full(&self) -> String {
        let mut s = self.left.clone();
        if self.right.starts_with('[') {
            s.push(' ');
        }
        s.push_str(&self.right);
        s
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    names: Vec<String>,
    types: Vec<String>,
    depth: u32,
}

const MAX_DEPTH: u32 = 64;

impl<'a> Parser<'a> {
    fn new(s: &'a [u8]) -> Parser<'a> {
        Parser { s, i: 0, names: Vec::new(), types: Vec::new(), depth: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        Some(c)
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn eat_str(&mut self, p: &[u8]) -> bool {
        if self.s[self.i..].starts_with(p) {
            self.i += p.len();
            true
        } else {
            false
        }
    }

    /// A number: `0`..`9` are 1..10; otherwise hex digits `A`..`P` ended by `@`; `?` negates.
    fn number(&mut self) -> Option<i64> {
        let neg = self.eat(b'?');
        let c = self.next()?;
        let v = if c.is_ascii_digit() {
            i64::from(c - b'0') + 1
        } else {
            let mut v: i64 = 0;
            let mut c = c;
            let mut n = 0;
            while c != b'@' {
                if !(b'A'..=b'P').contains(&c) || n >= 16 {
                    return None;
                }
                v = (v << 4) | i64::from(c - b'A');
                c = self.next()?;
                n += 1;
            }
            v
        };
        Some(if neg { -v } else { v })
    }

    /// A simple name up to `@`.
    fn ident(&mut self) -> Option<String> {
        let start = self.i;
        while self.peek()? != b'@' {
            self.i += 1;
        }
        let s = core::str::from_utf8(&self.s[start..self.i]).ok()?;
        self.i += 1;
        if s.is_empty() {
            return None;
        }
        Some(String::from(s))
    }

    fn remember_name(&mut self, n: &str) {
        if self.names.len() < 10 && !self.names.iter().any(|x| x == n) {
            self.names.push(String::from(n));
        }
    }

    /// One name fragment: a back reference, a template, an anonymous namespace or a name.
    fn fragment(&mut self) -> Option<String> {
        let c = self.peek()?;
        if c.is_ascii_digit() {
            self.i += 1;
            return self.names.get(usize::from(c - b'0')).cloned();
        }
        if self.eat_str(b"?$") {
            let t = self.template()?;
            self.remember_name(&t);
            return Some(t);
        }
        if self.eat_str(b"?A0x") {
            self.ident()?;
            return Some(String::from("`anonymous namespace'"));
        }
        let n = self.ident()?;
        self.remember_name(&n);
        Some(n)
    }

    /// `name@args@`: the template's own back references are its own.
    fn template(&mut self) -> Option<String> {
        let saved = (core::mem::take(&mut self.names), core::mem::take(&mut self.types));
        let r = self.template_inner();
        (self.names, self.types) = saved;
        r
    }

    fn template_inner(&mut self) -> Option<String> {
        let name = self.ident()?;
        self.remember_name(&name);
        let mut args: Vec<String> = Vec::new();
        while !self.eat(b'@') {
            args.push(self.template_arg()?);
        }
        let mut s = name;
        s.push('<');
        s.push_str(&args.join(","));
        if s.ends_with('>') {
            s.push(' ');
        }
        s.push('>');
        Some(s)
    }

    fn template_arg(&mut self) -> Option<String> {
        if self.eat_str(b"$0") {
            let n = self.number()?;
            return Some(alloc::format!("{n}"));
        }
        let t = self.ty()?.full();
        Some(t)
    }

    /// A qualified name: fragments innermost first, ended by `@`.
    fn qualified(&mut self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        while !self.eat(b'@') {
            parts.push(self.fragment()?);
            if parts.len() > 64 {
                return None;
            }
        }
        if parts.is_empty() {
            return None;
        }
        parts.reverse();
        Some(parts.join("::"))
    }

    /// A cv letter of a pointee or object: `A` none, `B` const, `C` volatile, `D` both.
    fn cv(&mut self) -> Option<&'static str> {
        Some(match self.next()? {
            b'A' => "",
            b'B' => " const",
            b'C' => " volatile",
            b'D' => " const volatile",
            _ => return None,
        })
    }

    /// `__ptr64` and the other pointer modifiers.
    fn modifiers(&mut self) -> String {
        let mut m = String::new();
        loop {
            match self.peek() {
                Some(b'E') => m.push_str(" __ptr64"),
                Some(b'F') => m.push_str(" __unaligned"),
                Some(b'I') => m.push_str(" __restrict"),
                _ => return m,
            }
            self.i += 1;
        }
    }

    fn calling_convention(&mut self) -> Option<&'static str> {
        Some(match self.next()? {
            b'A' | b'B' => "__cdecl",
            b'C' | b'D' => "__pascal",
            b'E' | b'F' => "__thiscall",
            b'G' | b'H' => "__stdcall",
            b'I' | b'J' => "__fastcall",
            b'Q' => "__vectorcall",
            _ => return None,
        })
    }

    /// A function type after its `6`/`8` marker: convention, return, arguments, throw spec.
    fn function(&mut self, declarator: &str) -> Option<Ty> {
        let cc = self.calling_convention()?;
        let ret = self.return_type()?;
        let args = self.arguments()?;
        // The throw specification: `Z` (none).
        if !self.eat(b'Z') {
            self.ty()?;
        }
        Some(Ty {
            left: alloc::format!("{} ({cc}{declarator}", ret.left),
            right: alloc::format!(")({args}){}", ret.right),
            compound: true,
            function: true,
        })
    }

    fn return_type(&mut self) -> Option<Ty> {
        if self.eat(b'?') {
            let cv = self.cv()?;
            let mut t = self.ty()?;
            t.left.push_str(cv);
            return Some(t);
        }
        if self.eat(b'@') {
            return Some(Ty::default());
        }
        self.ty()
    }

    fn arguments(&mut self) -> Option<String> {
        if self.eat(b'X') {
            return Some(String::from("void"));
        }
        let mut args: Vec<String> = Vec::new();
        loop {
            match self.peek()? {
                b'@' => {
                    self.i += 1;
                    break;
                }
                b'Z' => {
                    self.i += 1;
                    args.push(String::from("..."));
                    break;
                }
                c if c.is_ascii_digit() => {
                    self.i += 1;
                    args.push(self.types.get(usize::from(c - b'0'))?.clone());
                }
                _ => {
                    let start = self.i;
                    let t = self.ty()?.full();
                    if self.i - start > 1 && self.types.len() < 10 {
                        self.types.push(t.clone());
                    }
                    args.push(t);
                }
            }
            if args.len() > 256 {
                return None;
            }
        }
        Some(args.join(","))
    }

    /// A pointer or reference (`P` `Q` `R` `S` `A` `B`, `$$Q` `$$R`) to what follows.
    fn pointer(&mut self, sym: &str, own_cv: &str) -> Option<Ty> {
        if self.eat(b'6') {
            // A pointer to a function: no `__ptr64`.
            return self.function(sym);
        }
        if self.eat(b'8') {
            // A pointer to a member function: class, `this` modifiers and cv, the function.
            let class = self.qualified()?;
            let this_mods = self.modifiers();
            let this_cv = self.cv()?;
            let f = self.function(&alloc::format!(" {class}::{sym}"))?;
            let mut t = f;
            t.right.push_str(this_cv);
            t.right.push_str(&this_mods);
            t.function = false;
            return Some(t);
        }
        let mods = self.modifiers();
        let member = match self.peek()? {
            b'Q' | b'R' | b'S' | b'T' => {
                let c = self.next()?;
                let class = self.qualified()?;
                Some((c, class))
            }
            _ => None,
        };
        let cv = match &member {
            Some((c, _)) => match c {
                b'Q' => "",
                b'R' => " const",
                b'S' => " volatile",
                _ => " const volatile",
            },
            None => self.cv()?,
        };
        let inner = self.ty()?;
        let star = match &member {
            Some((_, class)) => alloc::format!("{class}::{sym}"),
            None => String::from(sym),
        };
        let mut t = inner;
        t.left.push_str(cv);
        if t.compound {
            t.left.push_str(" (");
            t.left.push_str(&star);
            t.left.push_str(&mods);
            t.left.push_str(own_cv);
            t.right.insert(0, ')');
        } else {
            t.left.push(' ');
            t.left.push_str(&star);
            t.left.push_str(&mods);
            t.left.push_str(own_cv);
        }
        t.compound = false;
        t.function = false;
        Some(t)
    }

    /// `Y` dims... element.
    fn array(&mut self) -> Option<Ty> {
        let n = self.number()?;
        if !(1..=16).contains(&n) {
            return None;
        }
        let mut dims = String::new();
        for _ in 0..n {
            let d = self.number()?;
            dims.push_str(&alloc::format!("[{d}]"));
        }
        let mut t = self.ty()?;
        t.right.insert_str(0, &dims);
        t.compound = true;
        Some(t)
    }

    fn ty(&mut self) -> Option<Ty> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let r = self.ty_inner();
        self.depth -= 1;
        r
    }

    fn ty_inner(&mut self) -> Option<Ty> {
        let c = self.next()?;
        let basic = |s: &str| Some(Ty::plain(s));
        match c {
            b'C' => basic("signed char"),
            b'D' => basic("char"),
            b'E' => basic("unsigned char"),
            b'F' => basic("short"),
            b'G' => basic("unsigned short"),
            b'H' => basic("int"),
            b'I' => basic("unsigned int"),
            b'J' => basic("long"),
            b'K' => basic("unsigned long"),
            b'M' => basic("float"),
            b'N' => basic("double"),
            b'O' => basic("long double"),
            b'X' => basic("void"),
            b'_' => match self.next()? {
                b'D' => basic("__int8"),
                b'E' => basic("unsigned __int8"),
                b'F' => basic("__int16"),
                b'G' => basic("unsigned __int16"),
                b'H' => basic("__int32"),
                b'I' => basic("unsigned __int32"),
                b'J' => basic("__int64"),
                b'K' => basic("unsigned __int64"),
                b'L' => basic("__int128"),
                b'M' => basic("unsigned __int128"),
                b'N' => basic("bool"),
                b'Q' => basic("char8_t"),
                b'S' => basic("char16_t"),
                b'U' => basic("char32_t"),
                b'W' => basic("wchar_t"),
                _ => None,
            },
            b'T' | b'U' | b'V' => {
                let kw = match c {
                    b'T' => "union",
                    b'U' => "struct",
                    _ => "class",
                };
                let n = self.qualified()?;
                Some(Ty::plain(&alloc::format!("{kw} {n}")))
            }
            b'W' => {
                // The underlying type's digit (4: int) is not shown.
                self.next()?;
                let n = self.qualified()?;
                Some(Ty::plain(&alloc::format!("enum {n}")))
            }
            b'P' => self.pointer("*", ""),
            b'Q' => self.pointer("*", " const"),
            b'R' => self.pointer("*", " volatile"),
            b'S' => self.pointer("*", " const volatile"),
            b'A' => self.pointer("&", ""),
            b'B' => self.pointer("&", " volatile"),
            b'Y' => self.array(),
            b'?' => {
                // A type with a cv qualifier of its own (`?A`, `?B`): shown after it.
                let cv = self.cv()?;
                let mut t = self.ty()?;
                t.left.push_str(cv);
                Some(t)
            }
            b'$' => {
                if self.eat_str(b"$T") {
                    return basic("std::nullptr_t");
                }
                if self.eat_str(b"$Q") {
                    return self.pointer("&&", "");
                }
                if self.eat_str(b"$R") {
                    return self.pointer("&&", " volatile");
                }
                if self.eat_str(b"$A6") {
                    // A function type itself.
                    let mut t = self.function("")?;
                    t.left = t.left.replacen(" (", " ", 1);
                    t.right = String::from(t.right.trim_start_matches(')'));
                    return Some(t);
                }
                if self.eat_str(b"$B") {
                    // An array type itself (`$$BY03H`).
                    return self.ty();
                }
                if self.eat_str(b"$C") {
                    let cv = self.cv()?;
                    let mut t = self.ty()?;
                    t.left.push_str(cv);
                    return Some(t);
                }
                None
            }
            _ => None,
        }
    }
}

/// The readable name of the type whose type descriptor name is `decorated` (with or without
/// its leading `.`), or `None` when it cannot be read.
pub fn type_name(decorated: &[u8]) -> Option<String> {
    let d = decorated.strip_prefix(b".").unwrap_or(decorated);
    if d.is_empty() {
        return None;
    }
    let mut p = Parser::new(d);
    let t = p.ty()?;
    if p.i != d.len() {
        return None;
    }
    Some(t.full())
}

/// `__std_type_info_hash`'s value: 64-bit FNV-1a over the decorated name without its leading
/// `.`, folded `h ^ (h >> 32)` (the sheet's `values`).
pub fn type_hash(decorated: &[u8]) -> u64 {
    let d = decorated.strip_prefix(b".").unwrap_or(decorated);
    let h = crate::typeinfo::hash_name(d);
    h ^ (h >> 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> String {
        type_name(s.as_bytes()).unwrap_or_else(|| panic!("{s}: no name"))
    }

    #[test]
    fn fundamental_types() {
        for (d, want) in [
            (".H", "int"),
            (".I", "unsigned int"),
            (".J", "long"),
            (".K", "unsigned long"),
            ("._J", "__int64"),
            ("._K", "unsigned __int64"),
            (".F", "short"),
            (".D", "char"),
            (".C", "signed char"),
            (".E", "unsigned char"),
            ("._W", "wchar_t"),
            ("._S", "char16_t"),
            ("._U", "char32_t"),
            ("._N", "bool"),
            (".M", "float"),
            (".N", "double"),
            (".O", "long double"),
            (".X", "void"),
            (".$$T", "std::nullptr_t"),
        ] {
            assert_eq!(n(d), want, "{d}");
        }
    }

    #[test]
    fn pointers_arrays_and_functions() {
        for (d, want) in [
            (".PEAH", "int * __ptr64"),
            (".PEBH", "int const * __ptr64"),
            (".PEAPEAH", "int * __ptr64 * __ptr64"),
            (".PEAX", "void * __ptr64"),
            (".PEBD", "char const * __ptr64"),
            (".$$BY03H", "int [4]"),
            (".PEAY03H", "int (* __ptr64)[4]"),
            (".P6AHHD@Z", "int (__cdecl*)(int,char)"),
            (".P6AXXZ", "void (__cdecl*)(void)"),
            (".PEQS@@H", "int S::* __ptr64"),
            (".P8S@@EAAXH@Z", "void (__cdecl S::*)(int) __ptr64"),
            (".PEBPEAH", "int * __ptr64 const * __ptr64"),
            (".AEAH", "int & __ptr64"),
            (".$$QEAH", "int && __ptr64"),
            (".P6AHPEAD0@Z", "int (__cdecl*)(char * __ptr64,char * __ptr64)"),
        ] {
            assert_eq!(n(d), want, "{d}");
        }
    }

    #[test]
    fn classes_namespaces_templates() {
        for (d, want) in [
            (".?AUS@@", "struct S"),
            (".?AVK@@", "class K"),
            (".?ATU@@", "union U"),
            (".?AW4En@@", "enum En"),
            (".?AUN@ns@@", "struct ns::N"),
            (".?AVM@inner@ns@@", "class ns::inner::M"),
            (".?AU?$Tpl@H@@", "struct Tpl<int>"),
            (".?AU?$Tpl@PEAUS@@@@", "struct Tpl<struct S * __ptr64>"),
            (".?AU?$Tpl@UN@ns@@@@", "struct Tpl<struct ns::N>"),
            (".?AU?$Tpl@U?$Tpl@H@@@@", "struct Tpl<struct Tpl<int> >"),
            (".?AU?$Two@HD@@", "struct Two<int,char>"),
            (".?AU?$Int@$02@@", "struct Int<3>"),
            (".?AU?$Int@$0A@@@", "struct Int<0>"),
            (".?AVlength_error@std@@", "class std::length_error"),
            (
                ".?AV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@",
                "class std::basic_string<char,struct std::char_traits<char>,class std::allocator<char> >",
            ),
        ] {
            assert_eq!(n(d), want, "{d}");
        }
    }

    #[test]
    fn bad_names_give_none() {
        for d in [".", "", ".Z9", ".?AU", ".PEA", ".?AUS@", ".H1", ".Y0H", ".?AU?$T@"] {
            assert_eq!(type_name(d.as_bytes()), None, "{d}");
        }
        // Deep nesting is refused, not overflowed.
        let deep = alloc::format!(".{}H", "PEA".repeat(500));
        assert_eq!(type_name(deep.as_bytes()), None);
    }

    #[test]
    fn hash_values_as_recorded() {
        assert_eq!(type_hash(b".H"), 0xaf64054c2966377b);
        assert_eq!(type_hash(b".?AUS@@"), 0xdad102f8198d3b21);
        assert_eq!(type_hash(b".?AUN@ns@@"), 0x19cbb8ff73787726);
        assert_eq!(type_hash(b".?AU?$Tpl@H@@"), 0x4e08efd5dbfe7384);
        let e = crate::typeinfo::hash_name(b"");
        assert_eq!(type_hash(b"."), e ^ (e >> 32));
    }
}
