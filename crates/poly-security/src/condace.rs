//! The application data of conditional ACEs and resource attribute ACEs, to
//! and from their SDDL text.
//!
//! Sources: [MS-DTYP] 2.4.4.17 "Conditional ACEs" (the `artx` signature,
//! the literal, attribute, relational and logical operator tokens and
//! their byte codes), 2.4.10.1 `CLAIM_SECURITY_ATTRIBUTE_RELATIVE_V1` and
//! 2.5.1.1 "Syntax" (the ABNF of `cond-expr` and `attribute-data`).
//!
//! An expression is stored in postfix order: the operands' tokens, then the
//! operator's byte; the whole is padded with zero bytes to a multiple of
//! four. The text written back is the canonical spelling the spec sheets
//! observed: keywords and prefixes in one fixed case, one space around
//! operators, every operand of `&&` and `||` in parentheses.

use crate::err::{SecErr, SecResult};
use crate::sddl;
use crate::sid::Sid;
use alloc::string::String;
use alloc::vec::Vec;

/// The first four bytes of a conditional ACE's application data.
pub const SIGNATURE: [u8; 4] = *b"artx";

// Literal tokens.
const TOKEN_INT64: u8 = 0x04;
const TOKEN_STRING: u8 = 0x10;
const TOKEN_OCTETS: u8 = 0x18;
const TOKEN_COMPOSITE: u8 = 0x50;
const TOKEN_SID: u8 = 0x51;
// Attribute tokens.
const TOKEN_LOCAL: u8 = 0xf8;
const TOKEN_USER: u8 = 0xf9;
const TOKEN_RESOURCE: u8 = 0xfa;
const TOKEN_DEVICE: u8 = 0xfb;
// Logical operators.
const OP_EXISTS: u8 = 0x87;
const OP_NOT_EXISTS: u8 = 0x8d;
const OP_AND: u8 = 0xa0;
const OP_OR: u8 = 0xa1;
const OP_NOT: u8 = 0xa2;

/// The operators that take a SID or a set of SIDs.
const MEMBER_OPS: &[(&str, u8)] = &[
    ("Member_of", 0x89),
    ("Device_Member_of", 0x8a),
    ("Member_of_Any", 0x8b),
    ("Device_Member_of_Any", 0x8c),
    ("Not_Member_of", 0x90),
    ("Not_Device_Member_of", 0x91),
    ("Not_Member_of_Any", 0x92),
    ("Not_Device_Member_of_Any", 0x93),
];

/// The binary relational operators written as symbols (the longer first).
const SYMBOL_OPS: &[(&str, u8)] = &[("==", 0x80), ("!=", 0x81), ("<=", 0x83), (">=", 0x85), ("<", 0x82), (">", 0x84)];

/// The binary relational operators written as words.
const WORD_OPS: &[(&str, u8)] = &[("Contains", 0x86), ("Any_of", 0x88), ("Not_Contains", 0x8e), ("Not_Any_of", 0x8f)];

/// The characters an `@Prefixed` attribute name must write as `%XXXX`.
const ENCODED: &[char] = &['!', '&', '(', ')', '>', '<', '=', '|', '%', ' ', '"'];

/// The integer token's sign and base bytes.
const SIGN_PLUS: u8 = 1;
const SIGN_MINUS: u8 = 2;
const SIGN_NONE: u8 = 3;
const BASE_OCTAL: u8 = 1;
const BASE_DECIMAL: u8 = 2;
const BASE_HEX: u8 = 3;

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

type R<T> = Result<T, ()>;

fn put_counted(out: &mut Vec<u8>, token: u8, body: &[u8]) {
    out.push(token);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(|c| c == ' ' || ('\t'..='\r').contains(&c)) {
            self.i += 1;
        }
    }

    /// Consume `lit` (letters in either case) if it is next.
    fn eat(&mut self, lit: &str) -> bool {
        let n = lit.chars().count();
        let next = self.s.get(self.i..self.i + n);
        if next.is_some_and(|t| t.iter().zip(lit.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b))) {
            self.i += n;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> R<()> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(())
        }
    }

    /// The word (letters, digits, `_`) that is next, not consumed.
    fn word(&self) -> String {
        self.s[self.i..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '_').collect()
    }

    /// Consume the word of `table` that is next (whole word, either case).
    fn keyword(&mut self, table: &[(&str, u8)]) -> Option<u8> {
        let w = self.word();
        let &(name, code) = table.iter().find(|(n, _)| n.eq_ignore_ascii_case(&w))?;
        self.i += name.len();
        Some(code)
    }

    fn expr(&mut self, out: &mut Vec<u8>) -> R<()> {
        self.super_term(out)?;
        loop {
            self.ws();
            if !self.eat("||") {
                return Ok(());
            }
            self.super_term(out)?;
            out.push(OP_OR);
        }
    }

    fn super_term(&mut self, out: &mut Vec<u8>) -> R<()> {
        self.factor(out)?;
        loop {
            self.ws();
            if !self.eat("&&") {
                return Ok(());
            }
            self.factor(out)?;
            out.push(OP_AND);
        }
    }

    fn factor(&mut self, out: &mut Vec<u8>) -> R<()> {
        self.ws();
        match self.peek() {
            Some('!') => {
                self.i += 1;
                self.factor(out)?;
                out.push(OP_NOT);
                Ok(())
            }
            Some('(') => {
                self.i += 1;
                self.expr(out)?;
                self.ws();
                self.expect(')')
            }
            _ => self.term(out),
        }
    }

    fn term(&mut self, out: &mut Vec<u8>) -> R<()> {
        if let Some(op) = self.keyword(MEMBER_OPS) {
            self.ws();
            self.sid_operand(out)?;
            out.push(op);
            return Ok(());
        }
        if let Some(op) = self.keyword(&[("Exists", OP_EXISTS), ("Not_Exists", OP_NOT_EXISTS)]) {
            self.ws();
            // Learn, "SDDL for Conditional ACEs": for resource attributes
            // (and names without a prefix); `Exists @User.x` is refused.
            let at = out.len();
            self.attribute(out)?;
            if !matches!(out[at], TOKEN_RESOURCE | TOKEN_LOCAL) {
                return Err(());
            }
            out.push(op);
            return Ok(());
        }
        self.attribute(out)?;
        self.ws();
        let op = SYMBOL_OPS.iter().find(|(s, _)| self.eat(s)).map(|&(_, c)| c).or_else(|| self.keyword(WORD_OPS));
        if let Some(op) = op {
            self.ws();
            match self.peek() {
                Some('@') => self.attribute(out)?,
                Some('{') => self.composite(out, Self::value)?,
                _ => self.value(out)?,
            }
            out.push(op);
        }
        Ok(())
    }

    /// An attribute name: `@User.`, `@Device.` or `@Resource.` and a name,
    /// or a simple (local) name.
    fn attribute(&mut self, out: &mut Vec<u8>) -> R<()> {
        let mut name = String::new();
        let token = if self.peek() == Some('@') {
            let token = if self.eat("@user.") {
                TOKEN_USER
            } else if self.eat("@device.") {
                TOKEN_DEVICE
            } else if self.eat("@resource.") {
                TOKEN_RESOURCE
            } else {
                return Err(());
            };
            while let Some(c) = self.peek() {
                if c == '%' {
                    let digits: String = self.s.get(self.i + 1..self.i + 5).ok_or(())?.iter().collect();
                    let v = u32::from_str_radix(&digits, 16).map_err(|_| ())?;
                    name.push(char::from_u32(v).filter(|c| *c != '\0').ok_or(())?);
                    self.i += 5;
                } else if ENCODED.contains(&c) || c.is_ascii_control() || c == ',' {
                    break;
                } else {
                    name.push(c);
                    self.i += 1;
                }
            }
            token
        } else {
            while let Some(c) = self.peek() {
                let ok =
                    c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '/' | '_') || (c == '@' && !name.is_empty());
                if !ok {
                    break;
                }
                name.push(c);
                self.i += 1;
            }
            TOKEN_LOCAL
        };
        if name.is_empty() {
            return Err(());
        }
        put_counted(out, token, &utf16_bytes(&name));
        Ok(())
    }

    /// `{ item, item... }` as a composite token.
    fn composite(&mut self, out: &mut Vec<u8>, item: fn(&mut Self, &mut Vec<u8>) -> R<()>) -> R<()> {
        self.expect('{')?;
        let mut body = Vec::new();
        loop {
            self.ws();
            item(self, &mut body)?;
            self.ws();
            match self.peek() {
                Some(',') => self.i += 1,
                Some('}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(()),
            }
        }
        put_counted(out, TOKEN_COMPOSITE, &body);
        Ok(())
    }

    /// The operand of a `Member_of` operator: a SID literal or a set of them.
    fn sid_operand(&mut self, out: &mut Vec<u8>) -> R<()> {
        if self.peek() == Some('{') { self.composite(out, Self::sid_literal) } else { self.sid_literal(out) }
    }

    /// `SID(<sid-string>)`.
    fn sid_literal(&mut self, out: &mut Vec<u8>) -> R<()> {
        if !self.eat("SID(") {
            return Err(());
        }
        let end = self.s[self.i..].iter().position(|&c| c == ')').ok_or(())? + self.i;
        let text: String = self.s[self.i..end].iter().collect();
        let wide: Vec<u16> = text.trim().encode_utf16().collect();
        let sid = sddl::parse(&wide).ok_or(())?;
        self.i = end + 1;
        put_counted(out, TOKEN_SID, &sid.to_bytes());
        Ok(())
    }

    /// A literal: an integer, a string in double quotes, `#` and
    /// hexadecimal digit pairs, or a SID literal.
    fn value(&mut self, out: &mut Vec<u8>) -> R<()> {
        match self.peek().ok_or(())? {
            '"' => {
                let s = self.quoted()?;
                put_counted(out, TOKEN_STRING, &utf16_bytes(&s));
            }
            '#' => {
                let b = self.octets()?;
                put_counted(out, TOKEN_OCTETS, &b);
            }
            'S' | 's' => self.sid_literal(out)?,
            _ => {
                let (v, sign, base) = self.integer()?;
                out.push(TOKEN_INT64);
                out.extend_from_slice(&v.to_le_bytes());
                out.extend_from_slice(&[sign, base]);
            }
        }
        Ok(())
    }

    fn quoted(&mut self) -> R<String> {
        self.expect('"')?;
        let end = self.s[self.i..].iter().position(|&c| c == '"').ok_or(())? + self.i;
        let s = self.s[self.i..end].iter().collect();
        self.i = end + 1;
        Ok(s)
    }

    fn octets(&mut self) -> R<Vec<u8>> {
        self.expect('#')?;
        let mut b = Vec::new();
        while let Some(hi) = self.peek().and_then(|c| c.to_digit(16)) {
            let lo = self.s.get(self.i + 1).and_then(|c| c.to_digit(16)).ok_or(())?;
            b.push((hi << 4 | lo) as u8);
            self.i += 2;
        }
        Ok(b)
    }

    /// The octets of a resource attribute value: hexadecimal digits (a `#`
    /// before them is accepted), an odd number read with a leading 0.
    fn hex_octets(&mut self) -> R<Vec<u8>> {
        if self.peek() == Some('#') {
            self.i += 1;
        }
        let mut digits: Vec<u8> = Vec::new();
        while let Some(d) = self.peek().and_then(|c| c.to_digit(16)) {
            digits.push(d as u8);
            self.i += 1;
        }
        if digits.len() % 2 == 1 {
            digits.insert(0, 0);
        }
        Ok(digits.chunks(2).map(|p| p[0] << 4 | p[1]).collect())
    }

    /// An integer: the value in two's complement, the sign byte and the
    /// base byte. `0x` is hexadecimal, a leading `0` octal.
    fn integer(&mut self) -> R<(u64, u8, u8)> {
        let sign = match self.peek() {
            Some('+') => SIGN_PLUS,
            Some('-') => SIGN_MINUS,
            _ => SIGN_NONE,
        };
        if sign != SIGN_NONE {
            self.i += 1;
        }
        let (radix, base) = if self.eat("0x") {
            (16, BASE_HEX)
        } else if self.peek() == Some('0') && self.s.get(self.i + 1).is_some_and(char::is_ascii_digit) {
            self.i += 1;
            (8, BASE_OCTAL)
        } else {
            (10, BASE_DECIMAL)
        };
        let start = self.i;
        let mut v: u64 = 0;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(radix)) {
            v = v.checked_mul(u64::from(radix)).and_then(|v| v.checked_add(u64::from(d))).ok_or(())?;
            self.i += 1;
        }
        if self.i == start {
            return Err(());
        }
        Ok((if sign == SIGN_MINUS { v.wrapping_neg() } else { v }, sign, base))
    }
}

fn chars(s: &[u16]) -> Vec<char> {
    char::decode_utf16(s.iter().copied()).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
}

/// The application data of a conditional ACE for the expression `text`
/// (what stands between the outer parentheses of the ACE's seventh field).
pub fn compile(text: &[u16]) -> SecResult<Vec<u8>> {
    let s = chars(text);
    let mut p = Parser { s: &s, i: 0 };
    let mut out = SIGNATURE.to_vec();
    p.expr(&mut out).map_err(|()| SecErr::InvalidAcl)?;
    p.ws();
    if p.i != s.len() {
        return Err(SecErr::InvalidAcl);
    }
    out.resize(out.len().next_multiple_of(4), 0);
    Ok(out)
}

fn counted(b: &[u8], at: usize) -> Option<(&[u8], usize)> {
    let n = u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize;
    Some((b.get(at + 4..at + 4 + n)?, at + 4 + n))
}

fn utf16_text(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    chars(&units).into_iter().collect()
}

fn sid_literal_text(b: &[u8]) -> Option<String> {
    let s = Sid::from_bytes(b)?;
    Some(alloc::format!("SID({})", sddl::alias_of(&s).map_or_else(|| sddl::to_string(&s), String::from)))
}

fn integer_text(v: u64, sign: u8, base: u8) -> String {
    let (prefix, magnitude) = match sign {
        SIGN_MINUS => ("-", v.wrapping_neg()),
        SIGN_PLUS => ("+", v),
        _ => ("", v),
    };
    match base {
        BASE_OCTAL => alloc::format!("{prefix}0{magnitude:o}"),
        BASE_HEX => alloc::format!("{prefix}0x{magnitude:x}"),
        _ => alloc::format!("{prefix}{magnitude}"),
    }
}

fn octets_text(b: &[u8]) -> String {
    let mut s = String::from("#");
    for x in b {
        s.push_str(&alloc::format!("{x:02x}"));
    }
    s
}

/// The text of one operand token at `at`: the text and the position after.
fn operand(b: &[u8], at: usize) -> Option<(String, usize)> {
    let token = *b.get(at)?;
    match token {
        0x01..=TOKEN_INT64 => {
            let v = u64::from_le_bytes(b.get(at + 1..at + 9)?.try_into().ok()?);
            Some((integer_text(v, *b.get(at + 9)?, *b.get(at + 10)?), at + 11))
        }
        TOKEN_STRING => counted(b, at + 1).map(|(body, next)| (alloc::format!("\"{}\"", utf16_text(body)), next)),
        TOKEN_OCTETS => counted(b, at + 1).map(|(body, next)| (octets_text(body), next)),
        TOKEN_SID => {
            let (body, next) = counted(b, at + 1)?;
            Some((sid_literal_text(body)?, next))
        }
        TOKEN_COMPOSITE => {
            let (body, next) = counted(b, at + 1)?;
            let mut items = Vec::new();
            let mut i = 0;
            while i < body.len() {
                let (t, n) = operand(body, i)?;
                items.push(t);
                i = n;
            }
            Some((alloc::format!("{{{}}}", items.join(", ")), next))
        }
        TOKEN_LOCAL..=TOKEN_DEVICE => {
            let (body, next) = counted(b, at + 1)?;
            let name = utf16_text(body);
            let text = match token {
                TOKEN_LOCAL => name,
                _ => {
                    let prefix = match token {
                        TOKEN_USER => "@USER.",
                        TOKEN_DEVICE => "@DEVICE.",
                        _ => "@RESOURCE.",
                    };
                    let mut t = String::from(prefix);
                    for c in name.chars() {
                        if ENCODED.contains(&c) || c.is_ascii_control() || c == ',' {
                            t.push_str(&alloc::format!("%{:04x}", c as u32));
                        } else {
                            t.push(c);
                        }
                    }
                    t
                }
            };
            Some((text, next))
        }
        _ => None,
    }
}

/// The operators as the text form spells them: `Member_of_any` and its
/// relatives with a small `a`.
const CANONICAL_UNARY: &[(&str, u8)] = &[
    ("Member_of", 0x89),
    ("Device_Member_of", 0x8a),
    ("Member_of_any", 0x8b),
    ("Device_Member_of_any", 0x8c),
    ("Not_Member_of", 0x90),
    ("Not_Device_Member_of", 0x91),
    ("Not_Member_of_any", 0x92),
    ("Not_Device_Member_of_any", 0x93),
    ("Exists", OP_EXISTS),
    ("Not_Exists", OP_NOT_EXISTS),
];

/// The expression text of a conditional ACE's application data (without
/// the outer parentheses); `None` if the data is not an expression. Every
/// operand of `&&` and `||` is put in parentheses, `!` is `!(x)`.
pub fn decompile(data: &[u8]) -> Option<String> {
    let mut stack: Vec<String> = Vec::new();
    let mut at = data.strip_prefix(&SIGNATURE).map(|_| SIGNATURE.len())?;
    while at < data.len() {
        let token = data[at];
        let name = |table: &[(&'static str, u8)]| table.iter().find(|(_, c)| *c == token).map(|&(n, _)| n);
        if token == 0 {
            // Padding: only zero bytes may follow.
            if data[at..].iter().any(|&b| b != 0) {
                return None;
            }
            break;
        } else if let Some(op) = name(CANONICAL_UNARY) {
            let x = stack.pop()?;
            stack.push(alloc::format!("{op} {x}"));
            at += 1;
        } else if let Some(op) = name(SYMBOL_OPS).or(name(WORD_OPS)) {
            let r = stack.pop()?;
            let l = stack.pop()?;
            stack.push(alloc::format!("{l} {op} {r}"));
            at += 1;
        } else if token == OP_NOT {
            let x = stack.pop()?;
            stack.push(alloc::format!("!({x})"));
            at += 1;
        } else if token == OP_AND || token == OP_OR {
            let op = if token == OP_AND { "&&" } else { "||" };
            let r = stack.pop()?;
            let l = stack.pop()?;
            stack.push(alloc::format!("({l}) {op} ({r})"));
            at += 1;
        } else {
            let (t, next) = operand(data, at)?;
            stack.push(t);
            at = next;
        }
    }
    let t = stack.pop()?;
    stack.is_empty().then_some(t)
}

// ----- resource attributes ---------------------------------------------------------------

/// `CLAIM_SECURITY_ATTRIBUTE_TYPE_*` ([MS-DTYP] 2.4.10.1) and their SDDL
/// type strings.
const CLAIM_TYPES: &[(&str, u8)] = &[("TI", 1), ("TU", 2), ("TS", 3), ("TD", 5), ("TB", 6), ("TX", 0x10)];
const CLAIM_INT64: u8 = 1;
const CLAIM_UINT64: u8 = 2;
const CLAIM_STRING: u8 = 3;
const CLAIM_SID: u8 = 5;
const CLAIM_BOOLEAN: u8 = 6;

/// The fixed part of `CLAIM_SECURITY_ATTRIBUTE_RELATIVE_V1`: the name's
/// offset, the value type, a reserved word, the flags and the value count.
const CLAIM_FIXED: usize = 16;

/// The flag bits an attribute may carry ([MS-DTYP] 2.5.1.1, `attr-flags`):
/// the system flags in the low six bits, custom ones in the high half.
const CLAIM_FLAGS: u64 = 0xFFFF_003F;

/// A resource attribute ACE's data for the SDDL attribute text
/// `"name",<type>,<flags>,<value>[,<value>...]`: a
/// `CLAIM_SECURITY_ATTRIBUTE_RELATIVE_V1`: the fixed part, one offset per
/// value, the name (NUL-terminated UTF-16), the values, zeros to a multiple
/// of four. Integers, booleans and SIDs start at a multiple of four after
/// the name; strings and octets follow one another unaligned (the spec
/// sheet's bytes). What does not parse is [`SecErr::InvalidAcl`], a SID
/// value that is none [`SecErr::InvalidSid`].
pub fn compile_attribute(text: &[u16]) -> SecResult<Vec<u8>> {
    let s = chars(text);
    let mut p = Parser { s: &s, i: 0 };
    let bad = |()| SecErr::InvalidAcl;
    p.ws();
    let name = p.quoted().map_err(bad)?;
    if name.is_empty() {
        return Err(SecErr::InvalidAcl);
    }
    p.ws();
    p.expect(',').map_err(bad)?;
    p.ws();
    let kind = p.keyword(CLAIM_TYPES).ok_or(SecErr::InvalidAcl)?;
    p.ws();
    p.expect(',').map_err(bad)?;
    p.ws();
    let (flags, sign, _) = p.integer().map_err(bad)?;
    if sign != SIGN_NONE || flags & !CLAIM_FLAGS != 0 {
        return Err(SecErr::InvalidAcl);
    }
    let mut values: Vec<Vec<u8>> = Vec::new();
    loop {
        p.ws();
        if p.i == p.s.len() {
            break;
        }
        p.expect(',').map_err(bad)?;
        p.ws();
        let counted_value = |b: &[u8]| {
            let mut v = (b.len() as u32).to_le_bytes().to_vec();
            v.extend_from_slice(b);
            v
        };
        values.push(match kind {
            CLAIM_INT64 => p.integer().map_err(bad)?.0.to_le_bytes().to_vec(),
            CLAIM_UINT64 => match p.integer().map_err(bad)? {
                (_, SIGN_MINUS, _) => return Err(SecErr::InvalidAcl),
                (v, _, _) => v.to_le_bytes().to_vec(),
            },
            CLAIM_STRING => {
                let mut b = utf16_bytes(&p.quoted().map_err(bad)?);
                b.extend_from_slice(&[0, 0]);
                b
            }
            CLAIM_SID => {
                let t: String = p.s[p.i..].iter().take_while(|c| **c != ',' && **c != ' ').collect();
                p.i += t.chars().count();
                let wide: Vec<u16> = t.encode_utf16().collect();
                counted_value(&sddl::parse(&wide).ok_or(SecErr::InvalidSid)?.to_bytes())
            }
            CLAIM_BOOLEAN => match p.integer().map_err(bad)? {
                (v @ (0 | 1), SIGN_NONE, _) => v.to_le_bytes().to_vec(),
                _ => return Err(SecErr::InvalidAcl),
            },
            _ => counted_value(&p.hex_octets().map_err(bad)?),
        });
    }
    // A name, a type and flags alone are not an attribute.
    if values.is_empty() {
        return Err(SecErr::InvalidAcl);
    }
    let aligned = matches!(kind, CLAIM_INT64 | CLAIM_UINT64 | CLAIM_SID | CLAIM_BOOLEAN);
    let mut out = alloc::vec![0u8; CLAIM_FIXED + 4 * values.len()];
    out[4..6].copy_from_slice(&u16::from(kind).to_le_bytes());
    out[8..12].copy_from_slice(&(flags as u32).to_le_bytes());
    out[12..16].copy_from_slice(&(values.len() as u32).to_le_bytes());
    let name_at = out.len() as u32;
    out[0..4].copy_from_slice(&name_at.to_le_bytes());
    out.extend_from_slice(&utf16_bytes(&name));
    out.extend_from_slice(&[0, 0]);
    if aligned {
        out.resize(out.len().next_multiple_of(4), 0);
    }
    for (i, v) in values.iter().enumerate() {
        let at = out.len() as u32;
        out[CLAIM_FIXED + 4 * i..CLAIM_FIXED + 4 * i + 4].copy_from_slice(&at.to_le_bytes());
        out.extend_from_slice(v);
    }
    out.resize(out.len().next_multiple_of(4), 0);
    Ok(out)
}

/// The SDDL attribute text of a resource attribute ACE's data: the flags in
/// hexadecimal, integers in decimal, SIDs as aliases where they have one,
/// octets as hexadecimal digits.
pub fn decompile_attribute(b: &[u8]) -> Option<String> {
    let dword = |at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize);
    let wide_at = |at: usize| -> Option<String> {
        let units: Vec<u16> =
            b.get(at..)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
        Some(chars(&units).into_iter().collect())
    };
    let kind = u8::try_from(u16::from_le_bytes([*b.get(4)?, *b.get(5)?])).ok()?;
    let type_name = CLAIM_TYPES.iter().find(|(_, k)| *k == kind)?.0;
    let mut text = alloc::format!("\"{}\",{type_name},{:#x}", wide_at(dword(0)?)?, dword(8)?);
    for i in 0..dword(12)? {
        let at = dword(CLAIM_FIXED + 4 * i)?;
        let quad = || Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?));
        let value = match kind {
            CLAIM_INT64 => alloc::format!("{}", quad()? as i64),
            CLAIM_UINT64 | CLAIM_BOOLEAN => alloc::format!("{}", quad()?),
            CLAIM_STRING => alloc::format!("\"{}\"", wide_at(at)?),
            CLAIM_SID => {
                let s = Sid::from_bytes(counted(b, at)?.0)?;
                sddl::alias_of(&s).map_or_else(|| sddl::to_string(&s), String::from)
            }
            _ => String::from(&octets_text(counted(b, at)?.0)[1..]),
        };
        text.push(',');
        text.push_str(&value);
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| alloc::format!("{x:02x}")).collect()
    }

    /// The condition's bytes as hexadecimal text.
    fn c(s: &str) -> String {
        hex(&compile(&w(s)).unwrap_or_else(|e| panic!("{s}: {e:?}")))
    }

    #[test]
    fn member_of_bytes() {
        // The signature, a composite of one SID literal, the operator, a pad byte.
        assert_eq!(c("Member_of {SID(BA)}"), "6172747850150000005110000000010200000000000520000000200200008900");
        assert_eq!(c("Member_of {SID(SY)}"), "617274785011000000510c0000000101000000000005120000008900");
        assert_eq!(c("Member_of{SID(SY)}"), c("Member_of {SID(SY)}"));
        assert_eq!(c("member_of {sid(sy)}"), c("Member_of {SID(SY)}"));
        assert_eq!(
            c(" Member_of  { SID(SY) , SID(BA) } "),
            "617274785026000000510c00000001010000000000051200000051100000000102000000000005200000002002000089"
        );
        assert_eq!(c("Member_of {SID(SY),SID(BA)}"), c(" Member_of  { SID(SY) , SID(BA) } "));
        // A single SID without braces is the SID token alone.
        assert_eq!(c("Member_of SID(SY)"), "61727478510c000000010100000000000512000000890000");
        assert_eq!(c("Member_of {SID(S-1-5-32-544)}"), c("Member_of {SID(BA)}"));
        for (name, code) in [
            ("Not_Member_of", "90"),
            ("Member_of_Any", "8b"),
            ("Device_Member_of", "8a"),
            ("Device_Member_of_Any", "8c"),
            ("Not_Device_Member_of", "91"),
        ] {
            let bytes = c(&alloc::format!("{name} {{SID(SY)}}"));
            assert_eq!(&bytes[bytes.len() - 4..], alloc::format!("{code}00"), "{name}");
        }
    }

    #[test]
    fn the_sheets_tokens() {
        for (s, bytes) in [
            ("@User.Title == \"PM\"", "61727478f90a0000005400690074006c006500100400000050004d0080000000"),
            ("@User.Title==\"PM\"", "61727478f90a0000005400690074006c006500100400000050004d0080000000"),
            ("Title == \"PM\"", "61727478f80a0000005400690074006c006500100400000050004d0080000000"),
            ("@Device.Level != 7", "61727478fb0a0000004c006500760065006c0004070000000000000003028100"),
            ("@Resource.Level < 7", "61727478fa0a0000004c006500760065006c0004070000000000000003028200"),
            ("@User.Level <= 0x10", "61727478f90a0000004c006500760065006c0004100000000000000003038300"),
            ("@User.Level > -3", "61727478f90a0000004c006500760065006c0004fdffffffffffffff02028400"),
            ("@User.Level >= 010", "61727478f90a0000004c006500760065006c0004080000000000000003018500"),
            ("@User.Tags Contains \"a\"", "61727478f90800000054006100670073001002000000610086000000"),
            (
                "@User.Tags Any_of {\"a\",\"b\"}",
                "61727478f9080000005400610067007300500e000000100200000061001002000000620088000000",
            ),
            (
                "@User.Tags Not_Contains {1,2}",
                "61727478f90800000054006100670073005016000000040100000000000000030204020000000000000003028e000000",
            ),
            ("@User.Tags Not_Any_of {1}", "61727478f9080000005400610067007300500b00000004010000000000000003028f0000"),
            ("Not_Exists @Resource.Title", "61727478fa0a0000005400690074006c0065008d"),
            ("!(@User.Level == 1)", "61727478f90a0000004c006500760065006c00040100000000000000030280a2"),
            (
                "@User.A == 1 && @User.B == 2",
                "61727478f9020000004100040100000000000000030280f9020000004200040200000000000000030280a000",
            ),
            (
                "@User.A == 1 || @User.B == 2",
                "61727478f9020000004100040100000000000000030280f9020000004200040200000000000000030280a100",
            ),
            (
                "(@User.A == 1) && (@User.B == 2 || @User.C == 3)",
                "61727478f9020000004100040100000000000000030280f9020000004200040200000000000000030280\
                 f9020000004300040300000000000000030280a1a000",
            ),
            ("@User.A == @Resource.B", "61727478f9020000004100fa0200000042008000"),
            ("@User.Blob == #0102ff", "61727478f90800000042006c006f00620018030000000102ff800000"),
            ("1 == 1", "61727478f802000000310004010000000000000003028000"),
            ("@USER.Title == \"PM\"", "61727478f90a0000005400690074006c006500100400000050004d0080000000"),
        ] {
            assert_eq!(c(s), bytes.replace(' ', ""), "{s}");
        }
        // `&&` binds tighter than `||`; extra parentheses add nothing.
        let ops = |s: &str| -> Vec<u8> {
            compile(&w(s)).unwrap().into_iter().filter(|x| (0xa0..=0xa2).contains(x)).collect()
        };
        assert_eq!(ops("a && b || c"), [0xa0, 0xa1]);
        assert_eq!(ops("a || b && c"), [0xa0, 0xa1]);
        assert_eq!(c("a || b && c"), c("a || (b && c)"));
        assert_eq!(c("((a)) && (b)"), c("a && b"));
    }

    #[test]
    fn canonical_text() {
        for (a, b) in [
            ("Member_of {SID(BA)}", "Member_of {SID(BA)}"),
            ("member_of{SID(ba),SID(S-1-5-18)}", "Member_of {SID(BA), SID(SY)}"),
            ("Member_of {SID(S-1-5-21-1-2-3-4)}", "Member_of {SID(S-1-5-21-1-2-3-4)}"),
            ("Member_of_Any {SID(WD)}", "Member_of_any {SID(WD)}"),
            ("Not_Member_of {SID(WD)}", "Not_Member_of {SID(WD)}"),
            ("Device_Member_of {SID(WD)}", "Device_Member_of {SID(WD)}"),
            ("@User.Title==\"PM\"", "@USER.Title == \"PM\""),
            ("  Title  !=  \"x y\" ", "Title != \"x y\""),
            ("@Resource.n >= 5", "@RESOURCE.n >= 5"),
            ("@device.m < 0x10", "@DEVICE.m < 0x10"),
            ("n > -3", "n > -3"),
            ("n <= 010", "n <= 010"),
            ("@User.A == 1 && @User.B == 2", "(@USER.A == 1) && (@USER.B == 2)"),
            ("a || b && c", "(a) || ((b) && (c))"),
            ("(a || b) && c", "((a) || (b)) && (c)"),
            ("!(x)", "!(x)"),
            ("!(a == 1)", "!(a == 1)"),
            ("!(a && b)", "!((a) && (b))"),
            ("Not_Exists @Resource.x", "Not_Exists @RESOURCE.x"),
            ("@User.Tags Contains {\"a\", \"b\"}", "@USER.Tags Contains {\"a\", \"b\"}"),
            ("@User.Tags Any_of @Resource.Tags", "@USER.Tags Any_of @RESOURCE.Tags"),
            ("x Not_Contains {1,-2,010}", "x Not_Contains {1, -2, 010}"),
            ("x == #0102FF", "x == #0102ff"),
            ("flag", "flag"),
        ] {
            let bytes = compile(&w(a)).unwrap_or_else(|e| panic!("{a}: {e:?}"));
            assert_eq!(bytes.len() % 4, 0, "{a}");
            assert_eq!(decompile(&bytes).as_deref(), Some(b), "{a}");
            assert_eq!(compile(&w(b)).as_ref(), Ok(&bytes), "{b} again");
        }
    }

    #[test]
    fn refused_conditions() {
        for s in [
            "",
            " ",
            "Member_of",
            "Member_of {}",
            "Member_of {SID(ZZ)}",
            "Member_of {SY}",
            "Member_of {SID(SY)",
            "Member_of {SID(SY)}}",
            "Member_of {1}",
            "Nonsense {SID(SY)}",
            "@User.Title ==",
            "== 1",
            "@User.Title = 1",
            "@User.A == 1 &&",
            "@User.A == 1 &&& @User.B == 2",
            "@User.A == 1 @User.B == 2",
            "@Nowhere.A == 1",
            "@User.A == 1) extra",
            "\"a\" == \"a\"",
            "@User.a%20b == 1",
            "a == \"x",
            "a == #1",
            "(a == 1",
            "a == 99999999999999999999",
            "Exists @User.Title",
            "Exists @User.Grade && Not_Exists @Device.Kind",
            "Not_Exists @Device.Kind",
        ] {
            assert_eq!(compile(&w(s)), Err(SecErr::InvalidAcl), "{s}");
        }
        assert_eq!(decompile(b"xtra"), None);
        assert_eq!(decompile(&[b'a', b'r', b't', b'x', 0x89]), None);
        assert_eq!(decompile(&[b'a', b'r', b't', b'x', 0x51, 9, 0, 0, 0]), None);
    }

    /// The attribute's bytes as hexadecimal text.
    fn a(s: &str) -> String {
        hex(&compile_attribute(&w(s)).unwrap_or_else(|e| panic!("{s}: {e:?}")))
    }

    #[test]
    fn attribute_bytes() {
        for (s, bytes) in [
            (
                "\"Dept\",TS,0,\"Lab\"",
                "140000000300000000000000010000001e000000440065007000740000004c006100620000000000",
            ),
            (
                "\"Dept\",TS,0,\"Lab\",\"Shop\",\"X\"",
                "1c000000030000000000000003000000260000002e00000038000000440065007000740000004c00610062000000\
                 530068006f007000000058000000",
            ),
            ("\"N\",TI,0,5", "14000000010000000000000001000000180000004e0000000500000000000000"),
            ("\"N\",TI,0,-5", "14000000010000000000000001000000180000004e000000fbffffffffffffff"),
            (
                "\"N\",TI,0,1,2,3",
                "1c0000000100000000000000030000002000000028000000300000004e000000\
                 010000000000000002000000000000000300000000000000",
            ),
            ("\"N\",TU,0,5", "14000000020000000000000001000000180000004e0000000500000000000000"),
            (
                "\"N\",TU,0,0x10,7",
                "180000000200000000000000020000001c000000240000004e00000010000000000000000700000000000000",
            ),
            ("\"Yes\",TB,0,1", "140000000600000000000000010000001c00000059006500730000000100000000000000"),
            (
                "\"Yes\",TB,0,0,1",
                "18000000060000000000000002000000200000002800000059006500730000000000000000000000\
                 0100000000000000",
            ),
            (
                "\"Who\",TD,0,BA",
                "140000000500000000000000010000001c000000570068006f0000001000000001020000000000052000000020020000",
            ),
            (
                "\"Who\",TD,0,SY,S-1-1-0",
                "1800000005000000000000000200000020000000300000 00570068006f000000\
                 0c0000000101000000000005120000000c000000010100000000000100000000",
            ),
            ("\"Bin\",TX,0,0a0b0c", "140000001000000000000000010000001c000000420069006e000000030000000a0b0c00"),
            (
                "\"Bin\",TX,0,01,0203",
                "180000001000000000000000020000002000000025000000420069006e000000010000000102000000020300",
            ),
            ("\"Odd\",TS,0,\"abc\"", "140000000300000000000000010000001c0000004f006400640000006100620063000000"),
            ("\"N\",TI,1,5", "14000000010000000100000001000000180000004e0000000500000000000000"),
            ("\"N\",TI,0x10000,5", "14000000010000000000010001000000180000004e0000000500000000000000"),
            (" \"N\" , TI , 0 , 5 ", "14000000010000000000000001000000180000004e0000000500000000000000"),
            ("\"N\",ti,0,5", "14000000010000000000000001000000180000004e0000000500000000000000"),
            // An odd number of digits is read with a leading 0.
            ("\"N\",TX,0,0a0", "14000000100000000000000001000000180000004e0000000200000000a00000"),
        ] {
            assert_eq!(a(s), bytes.replace(' ', ""), "{s}");
        }
    }

    #[test]
    fn attribute_text_and_errors() {
        for (s, t) in [
            ("\"Dept\",TS,0,\"Lab\",\"Shop\"", "\"Dept\",TS,0x0,\"Lab\",\"Shop\""),
            ("\"n\", TI, 0x10000, 1, -2, 0x10", "\"n\",TI,0x10000,1,-2,16"),
            ("\"u\",TU,0,18446744073709551615", "\"u\",TU,0x0,18446744073709551615"),
            ("\"s\",TD,0,BA,S-1-5-21-1-2-3-4", "\"s\",TD,0x0,BA,S-1-5-21-1-2-3-4"),
            ("\"b\",TB,0,1,0", "\"b\",TB,0x0,1,0"),
            ("\"x\",TX,0,00ff,#0102", "\"x\",TX,0x0,00ff,0102"),
        ] {
            let b = compile_attribute(&w(s)).unwrap_or_else(|e| panic!("{s}: {e:?}"));
            assert_eq!(b.len() % 4, 0, "{s}");
            assert_eq!(decompile_attribute(&b).as_deref(), Some(t), "{s}");
        }
        for s in [
            "",
            "\"N\"",
            "\"N\",TI",
            "\"N\",TI,0",
            "\"N\",TZ,0,5",
            "N,TI,0,5",
            "\"\",TS,0,\"v\"",
            "\"N\",TI,0,\"x\"",
            "\"N\",TS,0,5",
            "\"N\",TB,0,2",
            "\"N\",TU,0,-1",
            "\"N\",TI,0xFFFFFFFF,5",
            "\"N\",TI,0x40,5",
            "\"N\",TI,0,5) (\"M\",TI,0,5",
            "\"N\",TI,0,5 x",
        ] {
            assert_eq!(compile_attribute(&w(s)), Err(SecErr::InvalidAcl), "{s}");
        }
        assert_eq!(compile_attribute(&w("\"N\",TD,0,ZZ")), Err(SecErr::InvalidSid));
    }
}
