//! The expressions of `set /a` (Learn: "set"): 32-bit integers; from the loosest binding,
//! `,` `= *= /= %= += -= &= ^= |= <<= >>=` `|` `^` `&` `<< >>` `+ -` `* / %` and the unary
//! `! ~ -`; parentheses group. A name is a variable (not defined or not a number: 0);
//! numbers are decimal, `0x` hexadecimal or `0` octal.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The variables an expression reads and assigns.
pub trait Vars {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&mut self, name: &str, value: &str);
}

/// Why an expression has no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Syntax,
    DivideByZero,
}

/// A number as `set /a` and `if` read it: an optional sign, then decimal, `0x` hexadecimal
/// or `0` octal digits.
pub fn number(text: &str) -> Option<i64> {
    let t = text.trim();
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let v = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()?
    } else if t.len() > 1 && t.starts_with('0') {
        i64::from_str_radix(&t[1..], 8).ok()?
    } else {
        if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        t.parse::<i64>().ok()?
    };
    Some(if neg { -v } else { v })
}

/// Evaluate an expression; quotes in it are ignored.
pub fn eval(expr: &str, vars: &mut dyn Vars) -> Result<i32, Error> {
    let s: Vec<char> = expr.chars().filter(|&c| c != '"').collect();
    let mut a = Arith { s, i: 0, vars };
    let v = a.comma()?;
    a.blank();
    if a.i != a.s.len() {
        return Err(Error::Syntax);
    }
    Ok(v)
}

struct Arith<'a> {
    s: Vec<char>,
    i: usize,
    vars: &'a mut dyn Vars,
}

/// The binary operators by level, loosest first.
const LEVELS: &[&[&str]] = &[&["|"], &["^"], &["&"], &["<<", ">>"], &["+", "-"], &["*", "/", "%"]];

const ASSIGN: &[&str] = &["<<=", ">>=", "*=", "/=", "%=", "+=", "-=", "&=", "^=", "|=", "="];

fn apply(op: &str, a: i32, b: i32) -> Result<i32, Error> {
    Ok(match op {
        "|" => a | b,
        "^" => a ^ b,
        "&" => a & b,
        "<<" => a.wrapping_shl(b as u32),
        ">>" => a.wrapping_shr(b as u32),
        "+" => a.wrapping_add(b),
        "-" => a.wrapping_sub(b),
        "*" => a.wrapping_mul(b),
        "/" | "%" if b == 0 => return Err(Error::DivideByZero),
        "/" => a.wrapping_div(b),
        "%" => a.wrapping_rem(b),
        _ => return Err(Error::Syntax),
    })
}

impl Arith<'_> {
    fn blank(&mut self) {
        while matches!(self.s.get(self.i), Some(' ' | '\t')) {
            self.i += 1;
        }
    }

    fn starts(&self, op: &str) -> bool {
        op.chars().enumerate().all(|(k, c)| self.s.get(self.i + k) == Some(&c))
    }

    fn comma(&mut self) -> Result<i32, Error> {
        let mut v = self.assign()?;
        loop {
            self.blank();
            if self.s.get(self.i) != Some(&',') {
                return Ok(v);
            }
            self.i += 1;
            v = self.assign()?;
        }
    }

    fn name(&mut self) -> Option<String> {
        let start = self.i;
        while self.s.get(self.i).is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '#' | '@' | '.')) {
            self.i += 1;
        }
        let n: String = self.s[start..self.i].iter().collect();
        if n.is_empty() || n.starts_with(|c: char| c.is_ascii_digit()) {
            self.i = start;
            return None;
        }
        Some(n)
    }

    fn read(&self, name: &str) -> i32 {
        self.vars.get(name).and_then(|v| number(&v)).map_or(0, |v| v as i32)
    }

    fn assign(&mut self) -> Result<i32, Error> {
        self.blank();
        let save = self.i;
        if let Some(name) = self.name() {
            self.blank();
            if let Some(op) = ASSIGN.iter().find(|op| self.starts(op) && !(**op == "=" && self.starts("=="))) {
                self.i += op.len();
                let rhs = self.assign()?;
                let v = if *op == "=" { rhs } else { apply(&op[..op.len() - 1], self.read(&name), rhs)? };
                self.vars.set(&name, &format!("{v}"));
                return Ok(v);
            }
        }
        self.i = save;
        self.binary(0)
    }

    fn binary(&mut self, level: usize) -> Result<i32, Error> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut v = self.binary(level + 1)?;
        loop {
            self.blank();
            // an operator followed by `=` is an assignment, not this level's
            let Some(op) =
                LEVELS[level].iter().find(|op| self.starts(op) && self.s.get(self.i + op.len()) != Some(&'='))
            else {
                return Ok(v);
            };
            self.i += op.len();
            let rhs = self.binary(level + 1)?;
            v = apply(op, v, rhs)?;
        }
    }

    fn unary(&mut self) -> Result<i32, Error> {
        self.blank();
        match self.s.get(self.i) {
            Some('-') => {
                self.i += 1;
                Ok(self.unary()?.wrapping_neg())
            }
            Some('+') => {
                self.i += 1;
                self.unary()
            }
            Some('!') => {
                self.i += 1;
                Ok(i32::from(self.unary()? == 0))
            }
            Some('~') => {
                self.i += 1;
                Ok(!self.unary()?)
            }
            Some('(') => {
                self.i += 1;
                let v = self.comma()?;
                self.blank();
                if self.s.get(self.i) != Some(&')') {
                    return Err(Error::Syntax);
                }
                self.i += 1;
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(char::is_ascii_alphanumeric) {
                    self.i += 1;
                }
                let t: String = self.s[start..self.i].iter().collect();
                number(&t).map(|v| v as i32).ok_or(Error::Syntax)
            }
            _ => match self.name() {
                Some(n) => Ok(self.read(&n)),
                None => Err(Error::Syntax),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Env(BTreeMap<String, String>);

    impl Vars for Env {
        fn get(&self, name: &str) -> Option<String> {
            self.0.get(&name.to_ascii_uppercase()).cloned()
        }
        fn set(&mut self, name: &str, value: &str) {
            self.0.insert(name.to_ascii_uppercase(), value.into());
        }
    }

    #[test]
    fn numbers() {
        assert_eq!(number("42"), Some(42));
        assert_eq!(number(" -7 "), Some(-7));
        assert_eq!(number("0x1F"), Some(31));
        assert_eq!(number("010"), Some(8));
        assert_eq!(number("0"), Some(0));
        assert_eq!(number("09"), None);
        assert_eq!(number("4a"), None);
        assert_eq!(number(""), None);
    }

    #[test]
    fn expressions() {
        let mut e = Env::default();
        assert_eq!(eval("1+2*3", &mut e), Ok(7));
        assert_eq!(eval("(1+2)*3", &mut e), Ok(9));
        assert_eq!(eval("7/2, 7%2", &mut e), Ok(1));
        assert_eq!(eval("-3 + ~0 + !0", &mut e), Ok(-3));
        assert_eq!(eval("1<<4 | 1 & 3 ^ 2", &mut e), Ok(16 | ((1 & 3) ^ 2)));
        assert_eq!(eval("\"x=5\"", &mut e), Ok(5));
        assert_eq!(e.get("X").as_deref(), Some("5"));
        assert_eq!(eval("x+=2", &mut e), Ok(7));
        assert_eq!(eval("y = x * 2, y <<= 1", &mut e), Ok(28));
        assert_eq!(e.get("y").as_deref(), Some("28"));
        assert_eq!(eval("undefined + 1", &mut e), Ok(1));
        assert_eq!(eval("1/0", &mut e), Err(Error::DivideByZero));
        assert_eq!(eval("1 +", &mut e), Err(Error::Syntax));
        assert_eq!(eval("(1", &mut e), Err(Error::Syntax));
        assert_eq!(eval("2147483647+1", &mut e), Ok(i32::MIN));
    }
}
