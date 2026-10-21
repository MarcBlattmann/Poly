//! VT sequences (Learn: "Console Virtual Terminal Sequences"; ECMA-48):
//! a parser for what a program writes with
//! `ENABLE_VIRTUAL_TERMINAL_PROCESSING`, and the mapping between SGR
//! renditions and console attributes in both directions.

use crate::consts::*;
use alloc::vec::Vec;

/// What the parser recognised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// A character to print (one UTF-16 unit).
    Print(u16),
    /// A C0 control (0x00–0x1F, 0x7F).
    Control(u16),
    /// `CSI [private] params final`.
    Csi { private: Option<u8>, params: Vec<u16>, final_byte: u8 },
    /// `ESC final` (`ESC 7`, `ESC 8`, ...).
    Esc(u8),
    /// `OSC number ; text (BEL | ST)`.
    Osc { number: u16, text: Vec<u16> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
}

/// The parser's state between writes (a sequence may be split).
#[derive(Clone, Debug, Default)]
pub struct Parser {
    state: State,
    private: Option<u8>,
    params: Vec<u16>,
    current: Option<u16>,
    osc: Vec<u16>,
}

/// The most parameters kept (more are dropped).
const MAX_PARAMS: usize = 16;

impl Parser {
    /// Feed one UTF-16 unit; what it completes, if anything.
    pub fn feed(&mut self, u: u16) -> Option<Action> {
        match self.state {
            State::Ground => match u {
                0x1B => {
                    self.state = State::Escape;
                    None
                }
                0x00..=0x1F | 0x7F => Some(Action::Control(u)),
                _ => Some(Action::Print(u)),
            },
            State::Escape => match u {
                0x5B => {
                    self.state = State::Csi;
                    self.private = None;
                    self.params.clear();
                    self.current = None;
                    None
                }
                0x5D => {
                    self.state = State::Osc;
                    self.osc.clear();
                    None
                }
                0x1B => None,
                0x20..=0x7E => {
                    self.state = State::Ground;
                    Some(Action::Esc(u as u8))
                }
                _ => {
                    self.state = State::Ground;
                    None
                }
            },
            State::Csi => match u {
                0x30..=0x39 => {
                    let d = u - 0x30;
                    self.current = Some(self.current.unwrap_or(0).saturating_mul(10).saturating_add(d));
                    None
                }
                0x3B => {
                    if self.params.len() < MAX_PARAMS {
                        self.params.push(self.current.unwrap_or(0));
                    }
                    self.current = None;
                    None
                }
                0x3C..=0x3F if self.params.is_empty() && self.current.is_none() => {
                    self.private = Some(u as u8);
                    None
                }
                0x20..=0x2F | 0x3A..=0x3F => None,
                0x40..=0x7E => {
                    if (self.current.is_some() || !self.params.is_empty()) && self.params.len() < MAX_PARAMS {
                        self.params.push(self.current.unwrap_or(0));
                    }
                    self.state = State::Ground;
                    Some(Action::Csi {
                        private: self.private,
                        params: core::mem::take(&mut self.params),
                        final_byte: u as u8,
                    })
                }
                0x1B => {
                    self.state = State::Escape;
                    None
                }
                _ => {
                    self.state = State::Ground;
                    None
                }
            },
            State::Osc => match u {
                0x07 => self.end_osc(),
                0x1B => {
                    self.state = State::OscEscape;
                    None
                }
                _ => {
                    if self.osc.len() < 4096 {
                        self.osc.push(u);
                    }
                    None
                }
            },
            State::OscEscape => {
                if u == 0x5C {
                    self.end_osc()
                } else {
                    self.state = State::Ground;
                    None
                }
            }
        }
    }

    fn end_osc(&mut self) -> Option<Action> {
        self.state = State::Ground;
        let s = core::mem::take(&mut self.osc);
        let semi = s.iter().position(|&c| c == u16::from(b';'))?;
        let mut number = 0u16;
        for &d in &s[..semi] {
            if !(0x30..=0x39).contains(&d) {
                return None;
            }
            number = number.saturating_mul(10).saturating_add(d - 0x30);
        }
        Some(Action::Osc { number, text: s[semi + 1..].to_vec() })
    }

    /// In the middle of a sequence.
    pub fn busy(&self) -> bool {
        self.state != State::Ground
    }
}

/// ANSI colour index (red 1, green 2, blue 4) of an attribute's three
/// colour bits (blue 1, green 2, red 4).
fn ansi_of(bits: u16) -> u16 {
    ((bits & 4) >> 2) | (bits & 2) | ((bits & 1) << 2)
}

/// The attribute colour bits of an ANSI colour index.
fn bits_of(ansi: u16) -> u16 {
    ((ansi & 1) << 2) | (ansi & 2) | ((ansi & 4) >> 2)
}

/// The nearest of the 16 console colours to an RGB colour (Poly's
/// thresholds: each channel on above half, bright when any channel is
/// above three quarters).
fn nearest(r: u16, g: u16, b: u16) -> u16 {
    let bits = (if r > 0x7F { 4 } else { 0 }) | (if g > 0x7F { 2 } else { 0 }) | (if b > 0x7F { 1 } else { 0 });
    let bright = r > 0xBF || g > 0xBF || b > 0xBF;
    bits | if bright { FOREGROUND_INTENSITY } else { 0 }
}

/// A 256-colour index as a console colour (bits + intensity).
fn of_256(n: u16) -> u16 {
    match n {
        0..=7 => bits_of(n),
        8..=15 => bits_of(n - 8) | FOREGROUND_INTENSITY,
        16..=231 => {
            let n = n - 16;
            let level = |v: u16| if v == 0 { 0 } else { 55 + v * 40 };
            nearest(level(n / 36), level((n / 6) % 6), level(n % 6))
        }
        _ => {
            let grey = 8 + (n - 232) * 10;
            nearest(grey, grey, grey)
        }
    }
}

/// Apply SGR parameters (`CSI ... m`) to `attr`.
pub fn apply_sgr(mut attr: u16, params: &[u16]) -> u16 {
    if params.is_empty() {
        return DEFAULT_ATTRIBUTE;
    }
    let fg_mask = FOREGROUND_RED | FOREGROUND_GREEN | FOREGROUND_BLUE;
    let bg_mask = BACKGROUND_RED | BACKGROUND_GREEN | BACKGROUND_BLUE;
    let mut i = 0;
    while i < params.len() {
        let p = params[i];
        match p {
            0 => attr = DEFAULT_ATTRIBUTE,
            1 => attr |= FOREGROUND_INTENSITY,
            22 => attr &= !FOREGROUND_INTENSITY,
            4 => attr |= COMMON_LVB_UNDERSCORE,
            24 => attr &= !COMMON_LVB_UNDERSCORE,
            7 => attr |= COMMON_LVB_REVERSE_VIDEO,
            27 => attr &= !COMMON_LVB_REVERSE_VIDEO,
            30..=37 => attr = (attr & !fg_mask) | bits_of(p - 30),
            39 => attr = (attr & !(fg_mask | FOREGROUND_INTENSITY)) | DEFAULT_ATTRIBUTE,
            40..=47 => attr = (attr & !bg_mask) | (bits_of(p - 40) << 4),
            49 => attr &= !(bg_mask | BACKGROUND_INTENSITY),
            90..=97 => attr = (attr & !fg_mask) | bits_of(p - 90) | FOREGROUND_INTENSITY,
            100..=107 => attr = (attr & !bg_mask) | (bits_of(p - 100) << 4) | BACKGROUND_INTENSITY,
            38 | 48 => {
                let colour = match params.get(i + 1) {
                    Some(5) => {
                        let c = params.get(i + 2).copied().map(of_256);
                        i += 2;
                        c
                    }
                    Some(2) => {
                        let rgb = (params.get(i + 2), params.get(i + 3), params.get(i + 4));
                        i += 4;
                        match rgb {
                            (Some(&r), Some(&g), Some(&b)) => Some(nearest(r, g, b)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(c) = colour {
                    if p == 38 {
                        attr = (attr & !(fg_mask | FOREGROUND_INTENSITY)) | c;
                    } else {
                        attr = (attr & !(bg_mask | BACKGROUND_INTENSITY)) | (c << 4);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    attr
}

/// The SGR sequence that makes a terminal draw with `attr`. The default
/// attribute is the terminal's own default rendition (`CSI 0 m`); others
/// set both colours.
pub fn sgr_for(attr: u16) -> Vec<u8> {
    let mut s = Vec::from(&b"\x1b[0"[..]);
    let extra = attr & (COMMON_LVB_REVERSE_VIDEO | COMMON_LVB_UNDERSCORE);
    if attr & 0xFF != DEFAULT_ATTRIBUTE {
        let fg = ansi_of(attr & 7) + if attr & FOREGROUND_INTENSITY != 0 { 90 } else { 30 };
        let bg = ansi_of((attr >> 4) & 7) + if attr & BACKGROUND_INTENSITY != 0 { 100 } else { 40 };
        push_num(&mut s, b';', fg);
        push_num(&mut s, b';', bg);
    }
    if extra & COMMON_LVB_UNDERSCORE != 0 {
        s.extend_from_slice(b";4");
    }
    if extra & COMMON_LVB_REVERSE_VIDEO != 0 {
        s.extend_from_slice(b";7");
    }
    s.push(b'm');
    s
}

/// Append `sep` and `n` in decimal.
pub fn push_num(s: &mut Vec<u8>, sep: u8, n: u16) {
    if sep != 0 {
        s.push(sep);
    }
    let mut digits = [0u8; 5];
    let mut len = 0;
    let mut v = n;
    loop {
        digits[len] = b'0' + (v % 10) as u8;
        len += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    s.extend(digits[..len].iter().rev());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str) -> Vec<Action> {
        let mut p = Parser::default();
        s.encode_utf16().filter_map(|u| p.feed(u)).collect()
    }

    #[test]
    fn parses_csi_esc_osc() {
        assert_eq!(
            run("a\x1b[1;31mb"),
            [
                Action::Print(97),
                Action::Csi { private: None, params: alloc::vec![1, 31], final_byte: b'm' },
                Action::Print(98)
            ]
        );
        assert_eq!(run("\x1b[H"), [Action::Csi { private: None, params: alloc::vec![], final_byte: b'H' }]);
        assert_eq!(run("\x1b[?25l"), [Action::Csi { private: Some(b'?'), params: alloc::vec![25], final_byte: b'l' }]);
        assert_eq!(run("\x1b7"), [Action::Esc(b'7')]);
        let title: Vec<u16> = "hi".encode_utf16().collect();
        assert_eq!(run("\x1b]0;hi\x07"), [Action::Osc { number: 0, text: title.clone() }]);
        assert_eq!(run("\x1b]2;hi\x1b\\"), [Action::Osc { number: 2, text: title }]);
        assert_eq!(run("\r"), [Action::Control(13)]);
    }

    #[test]
    fn sgr_to_attributes() {
        assert_eq!(apply_sgr(0x07, &[31]), FOREGROUND_RED);
        assert_eq!(apply_sgr(0x07, &[1, 32, 44]), FOREGROUND_GREEN | FOREGROUND_INTENSITY | BACKGROUND_BLUE);
        assert_eq!(apply_sgr(0x1F, &[0]), DEFAULT_ATTRIBUTE);
        assert_eq!(apply_sgr(0x07, &[]), DEFAULT_ATTRIBUTE);
        assert_eq!(apply_sgr(0x07, &[93]), FOREGROUND_RED | FOREGROUND_GREEN | FOREGROUND_INTENSITY);
        assert_eq!(apply_sgr(0x07, &[7, 4]), 0x07 | COMMON_LVB_REVERSE_VIDEO | COMMON_LVB_UNDERSCORE);
        assert_eq!(apply_sgr(0x07, &[38, 5, 9]), FOREGROUND_RED | FOREGROUND_INTENSITY);
        assert_eq!(apply_sgr(0x07, &[38, 2, 255, 0, 0, 1]), FOREGROUND_RED | FOREGROUND_INTENSITY);
        assert_eq!(apply_sgr(0x07, &[48, 5, 4]), BACKGROUND_BLUE | 0x07);
    }

    #[test]
    fn attributes_to_sgr() {
        assert_eq!(sgr_for(DEFAULT_ATTRIBUTE), b"\x1b[0m");
        assert_eq!(sgr_for(FOREGROUND_RED | FOREGROUND_INTENSITY | BACKGROUND_BLUE), b"\x1b[0;91;44m");
        assert_eq!(sgr_for(0x07 | COMMON_LVB_UNDERSCORE), b"\x1b[0;4m");
        // Round trip through the parser's mapping.
        for attr in 0..=0xFFu16 {
            let s = sgr_for(attr);
            let body = core::str::from_utf8(&s[2..s.len() - 1]).unwrap();
            let params: Vec<u16> = body.split(';').map(|p| p.parse().unwrap()).collect();
            assert_eq!(apply_sgr(0, &params), attr, "{attr:#x}");
        }
    }
}
