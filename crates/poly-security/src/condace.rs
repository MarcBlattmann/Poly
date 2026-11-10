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
//! four. The text written back is one canonical spelling: one space around
//! binary operators, parentheses only where the precedence needs them.

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
            self.attribute(out)?;
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
                        TOKEN_USER => "@User.",
                        TOKEN_DEVICE => "@Device.",
                        _ => "@Resource.",
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

/// The expression text of a conditional ACE's application data (without
/// the outer parentheses); `None` if the data is not an expression.
pub fn decompile(data: &[u8]) -> Option<String> {
    // Precedence of what is on the stack: 3 a term, 2 `&&`, 1 `||`.
    let mut stack: Vec<(String, u8)> = Vec::new();
    let mut at = data.strip_prefix(&SIGNATURE).map(|_| SIGNATURE.len())?;
    let wrap = |(t, p): (String, u8), need: u8| if p < need { alloc::format!("({t})") } else { t };
    while at < data.len() {
        let token = data[at];
        let name = |table: &[(&'static str, u8)]| table.iter().find(|(_, c)| *c == token).map(|&(n, _)| n);
        if token == 0 {
            // Padding: only zero bytes may follow.
            if data[at..].iter().any(|&b| b != 0) {
                return None;
            }
            break;
        } else if let Some(op) = name(MEMBER_OPS).or(name(&[("Exists", OP_EXISTS), ("Not_Exists", OP_NOT_EXISTS)])) {
            let (x, _) = stack.pop()?;
            stack.push((alloc::format!("{op} {x}"), 3));
            at += 1;
        } else if let Some(op) = name(SYMBOL_OPS).or(name(WORD_OPS)) {
            let (r, _) = stack.pop()?;
            let (l, _) = stack.pop()?;
            stack.push((alloc::format!("{l} {op} {r}"), 3));
            at += 1;
        } else if token == OP_NOT {
            let (x, _) = stack.pop()?;
            stack.push((alloc::format!("!({x})"), 3));
            at += 1;
        } else if token == OP_AND || token == OP_OR {
            let (prec, op) = if token == OP_AND { (2, "&&") } else { (1, "||") };
            let r = stack.pop()?;
            let l = stack.pop()?;
            // The right operand of a chain binds one tighter.
            stack.push((alloc::format!("{} {op} {}", wrap(l, prec), wrap(r, prec + 1)), prec));
            at += 1;
        } else {
            let (t, next) = operand(data, at)?;
            stack.push((t, 3));
            at = next;
        }
    }
    let (t, _) = stack.pop()?;
    stack.is_empty().then_some(t)
}

// ----- resource attributes ---------------------------------------------------------------

/// `CLAIM_SECURITY_ATTRIBUTE_TYPE_*` ([MS-DTYP] 2.4.10.1) and their SDDL
/// type strings.
const CLAIM_TYPES: &[(&str, u8)] = &[("TI", 1), ("TU", 2), ("TS", 3), ("TD", 5), ("TB", 6), ("TX", 0x10)];

/// The fixed part of `CLAIM_SECURITY_ATTRIBUTE_RELATIVE_V1`: the name's
/// offset, the value type, a reserved word, the flags and the value count.
const CLAIM_FIXED: usize = 16;

/// A resource attribute ACE's data for the SDDL attribute text
/// `"name",<type>,<flags>[,<value>...]`: a
/// `CLAIM_SECURITY_ATTRIBUTE_RELATIVE_V1` with the value offsets after the
/// fixed part, then the name and the values, each at a multiple of four.
pub fn compile_attribute(text: &[u16]) -> SecResult<Vec<u8>> {
    let s = chars(text);
    let mut p = Parser { s: &s, i: 0 };
    attribute_data(&mut p).map_err(|()| SecErr::InvalidAcl)
}

fn attribute_data(p: &mut Parser) -> R<Vec<u8>> {
    p.ws();
    let name = p.quoted()?;
    p.ws();
    p.expect(',')?;
    p.ws();
    let kind = p.keyword(CLAIM_TYPES).ok_or(())?;
    p.ws();
    p.expect(',')?;
    p.ws();
    let (flags, sign, _) = p.integer()?;
    if sign != SIGN_NONE || flags > u64::from(u32::MAX) {
        return Err(());
    }
    let mut values: Vec<Vec<u8>> = Vec::new();
    loop {
        p.ws();
        if p.i == p.s.len() {
            break;
        }
        p.expect(',')?;
        p.ws();
        let counted_value = |b: &[u8]| {
            let mut v = (b.len() as u32).to_le_bytes().to_vec();
            v.extend_from_slice(b);
            v
        };
        values.push(match kind {
            1 => p.integer()?.0.to_le_bytes().to_vec(),
            2 => {
                let (v, sign, _) = p.integer()?;
                if sign == SIGN_MINUS {
                    return Err(());
                }
                v.to_le_bytes().to_vec()
            }
            3 => {
                let mut b = utf16_bytes(&p.quoted()?);
                b.extend_from_slice(&[0, 0]);
                b
            }
            5 => {
                let t: String = p.s[p.i..].iter().take_while(|c| **c != ',' && **c != ' ').collect();
                p.i += t.chars().count();
                let wide: Vec<u16> = t.encode_utf16().collect();
                counted_value(&sddl::parse(&wide).ok_or(())?.to_bytes())
            }
            6 => match p.integer()? {
                (v @ (0 | 1), SIGN_NONE, _) => v.to_le_bytes().to_vec(),
                _ => return Err(()),
            },
            _ => counted_value(&p.octets()?),
        });
    }
    let mut name_bytes = utf16_bytes(&name);
    name_bytes.extend_from_slice(&[0, 0]);
    let mut out = alloc::vec![0u8; CLAIM_FIXED + 4 * values.len()];
    out[4..6].copy_from_slice(&u16::from(kind).to_le_bytes());
    out[8..12].copy_from_slice(&(flags as u32).to_le_bytes());
    out[12..16].copy_from_slice(&(values.len() as u32).to_le_bytes());
    let name_at = out.len() as u32;
    out[0..4].copy_from_slice(&name_at.to_le_bytes());
    out.extend_from_slice(&name_bytes);
    for (i, v) in values.iter().enumerate() {
        out.resize(out.len().next_multiple_of(4), 0);
        let at = out.len() as u32;
        out[CLAIM_FIXED + 4 * i..CLAIM_FIXED + 4 * i + 4].copy_from_slice(&at.to_le_bytes());
        out.extend_from_slice(v);
    }
    out.resize(out.len().next_multiple_of(4), 0);
    Ok(out)
}

/// The SDDL attribute text of a resource attribute ACE's data.
pub fn decompile_attribute(b: &[u8]) -> Option<String> {
    let dword = |at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize);
    let wide_at = |at: usize| -> Option<String> {
        let units: Vec<u16> =
            b.get(at..)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
        Some(chars(&units).into_iter().collect())
    };
    let kind = u16::from_le_bytes([*b.get(4)?, *b.get(5)?]);
    let type_name = CLAIM_TYPES.iter().find(|(_, k)| u16::from(*k) == kind)?.0;
    let mut text = alloc::format!("\"{}\",{type_name},{:#x}", wide_at(dword(0)?)?, dword(8)?);
    for i in 0..dword(12)? {
        let at = dword(CLAIM_FIXED + 4 * i)?;
        let quad = || Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?));
        let value = match kind {
            1 => alloc::format!("{}", quad()? as i64),
            2 | 6 => alloc::format!("{}", quad()?),
            3 => alloc::format!("\"{}\"", wide_at(at)?),
            5 => {
                let s = Sid::from_bytes(counted(b, at)?.0)?;
                sddl::alias_of(&s).map_or_else(|| sddl::to_string(&s), String::from)
            }
            _ => octets_text(counted(b, at)?.0),
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

    #[test]
    fn member_of_bytes() {
        // The signature, a composite of one SID literal, the operator, a pad byte.
        let b = compile(&w("Member_of {SID(BA)}")).unwrap();
        assert_eq!(hex(&b), "6172747850150000005110000000010200000000000520000000200200008900");
        assert_eq!(b.len(), 32);
        assert_eq!(decompile(&b).as_deref(), Some("Member_of {SID(BA)}"));
        let b = compile(&w("Member_of SID(S-1-5-32-544)")).unwrap();
        assert_eq!(b.len(), 28);
        assert_eq!(decompile(&b).as_deref(), Some("Member_of SID(BA)"));
    }

    #[test]
    fn tokens() {
        // A user attribute, a string, `==`.
        let b = compile(&w("@User.a==\"b\"")).unwrap();
        assert_eq!(hex(&b), "61727478f9020000006100100200000062008000");
        // An integer: value, sign, base.
        let b = compile(&w("n >= -0x10")).unwrap();
        assert_eq!(hex(&b), "61727478f8020000006e0004f0ffffffffffffff02038500");
        let b = compile(&w("n < 017")).unwrap();
        assert_eq!(hex(&b[11..22]), "040f000000000000000301");
        // Postfix order of the logical operators.
        let b = compile(&w("a && (b || !c)")).unwrap();
        let ops: Vec<u8> = b.iter().copied().filter(|x| *x >= 0xa0 && *x <= 0xa2).collect();
        assert_eq!(ops, [0xa2, 0xa1, 0xa0]);
    }

    #[test]
    fn text_round_trips() {
        for (a, b) in [
            ("Member_of {SID(BA)}", "Member_of {SID(BA)}"),
            ("member_of{SID(ba),SID(S-1-5-18)}", "Member_of {SID(BA), SID(SY)}"),
            ("Not_Device_Member_of_Any {SID(WD)}", "Not_Device_Member_of_Any {SID(WD)}"),
            ("@User.Title==\"PM\"", "@User.Title == \"PM\""),
            ("  Title  !=  \"x y\" ", "Title != \"x y\""),
            ("@Resource.n >= 5 && @Device.m < +0x1F", "@Resource.n >= 5 && @Device.m < +0x1f"),
            ("a == 1 || b == 2 && c == 3", "a == 1 || b == 2 && c == 3"),
            ("(a == 1 || b == 2) && c == 3", "(a == 1 || b == 2) && c == 3"),
            ("a == 1 || (b == 2 || c == 3)", "a == 1 || (b == 2 || c == 3)"),
            ("!(a == 1)", "!(a == 1)"),
            ("! Exists x", "!(Exists x)"),
            ("Not_Exists @Resource.x", "Not_Exists @Resource.x"),
            ("@User.Tags Contains {\"a\", \"b\"}", "@User.Tags Contains {\"a\", \"b\"}"),
            ("@User.Tags Any_of @Resource.Tags", "@User.Tags Any_of @Resource.Tags"),
            ("x Not_Any_of {1, -2, 010}", "x Not_Any_of {1, -2, 010}"),
            ("x == #0aFF", "x == #0aff"),
            ("@User.a%0020b == 1", "@User.a%0020b == 1"),
            ("flag", "flag"),
        ] {
            let bytes = compile(&w(a)).unwrap_or_else(|e| panic!("{a}: {e:?}"));
            assert_eq!(bytes.len() % 4, 0, "{a}");
            assert_eq!(decompile(&bytes).as_deref(), Some(b), "{a}");
            assert_eq!(compile(&w(b)).as_ref(), Ok(&bytes), "{b} again");
        }
    }

    #[test]
    fn bad_expressions() {
        for s in [
            "",
            "Member_of",
            "Member_of {",
            "Member_of {SID(ZZ)}",
            "Member_of {1}",
            "a ==",
            "a == \"x",
            "a == #1",
            "(a == 1",
            "a == 1)",
            "a && ",
            "@Nobody.x == 1",
            "a == 99999999999999999999",
            "== 1",
        ] {
            assert_eq!(compile(&w(s)), Err(SecErr::InvalidAcl), "{s}");
        }
        assert_eq!(decompile(b"xtra"), None);
        assert_eq!(decompile(&[b'a', b'r', b't', b'x', 0x89]), None);
        assert_eq!(decompile(&[b'a', b'r', b't', b'x', 0x51, 9, 0, 0, 0]), None);
    }

    #[test]
    fn attributes() {
        let b = compile_attribute(&w("\"Project\",TS,0,\"Poly\"")).unwrap();
        // Name offset 20, type 3, flags 0, one value; the name; the value.
        assert_eq!(hex(&b[..20]), "1400000003000000000000000100000024000000");
        assert_eq!(b.len(), 36 + 12);
        for (a, t) in [
            ("\"Project\",TS,0,\"Poly\"", "\"Project\",TS,0x0,\"Poly\""),
            ("\"n\", TI, 0x10, 1, -2, 0x10", "\"n\",TI,0x10,1,-2,16"),
            ("\"u\",TU,0", "\"u\",TU,0x0"),
            ("\"u\",TU,0,18446744073709551615", "\"u\",TU,0x0,18446744073709551615"),
            ("\"s\",TD,0,BA,S-1-5-21-1-2-3-4", "\"s\",TD,0x0,BA,S-1-5-21-1-2-3-4"),
            ("\"b\",TB,0,1,0", "\"b\",TB,0x0,1,0"),
            ("\"x\",TX,0,#00ff,#", "\"x\",TX,0x0,#00ff,#"),
        ] {
            let b = compile_attribute(&w(a)).unwrap_or_else(|e| panic!("{a}: {e:?}"));
            assert_eq!(b.len() % 4, 0, "{a}");
            assert_eq!(decompile_attribute(&b).as_deref(), Some(t), "{a}");
        }
        for s in ["", "Project,TS,0", "\"p\",TQ,0", "\"p\",TB,0,2", "\"p\",TU,0,-1", "\"p\",TS,0,1", "\"p\",TD,0,ZZ"] {
            assert_eq!(compile_attribute(&w(s)), Err(SecErr::InvalidAcl), "{s}");
        }
    }
}
