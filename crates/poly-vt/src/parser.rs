//! Bytes to characters to actions: the UTF-8 decoder and the VT parser.
//!
//! Bytes from the pty are decoded as UTF-8 first ([`Utf8`]); the decoder
//! keeps its state between [`Parser::feed`] calls, so a character split
//! across two reads comes out whole. Malformed input becomes U+FFFD, one
//! per maximal ill-formed subsequence (the Unicode standard's
//! recommendation, §3.9): stray continuation bytes, truncated, overlong
//! and surrogate sequences, and values past U+10FFFF.
//!
//! The characters then go through the usual VT state machine; the shape of
//! the sequences is ECMA-48 §5.4:
//!
//! - `ESC` followed by intermediates (0x20-0x2f) and a final character
//!   (0x30-0x7e) is an escape sequence ([`Perform::esc`]).
//! - `ESC [` starts a control sequence ([`Perform::csi`]): an optional
//!   private marker (`<`, `=`, `>`, `?`) first, numeric parameters
//!   separated by `;` or (sub-parameters, ITU T.416) `:`, intermediates and
//!   a final byte 0x40-0x7e. Parameters saturate at 65535; past
//!   [`MAX_PARAMS`] they are dropped. A malformed sequence (a private
//!   marker in the middle, a digit after an intermediate, a non-ASCII
//!   character) is skipped up to its final byte.
//! - `ESC ]` starts an operating system command ([`Perform::osc`]), up to
//!   BEL or ST (`ESC \`, or the C1 code point U+009C). At most
//!   [`MAX_OSC`] bytes are kept; the rest is dropped.
//! - `ESC P` (DCS), `ESC X` (SOS), `ESC ^` (PM) and `ESC _` (APC) start
//!   strings that are skipped up to BEL or ST.
//! - Inside a sequence, C0 controls are carried out as if they came first;
//!   `ESC` starts a new sequence (inside a string, `ESC` followed by
//!   anything but `\` ends the string and starts that sequence); CAN and
//!   SUB cancel the sequence. DEL is ignored everywhere.
//! - Outside sequences, C0 controls go to [`Perform::execute`] and other
//!   characters to [`Perform::print`]. C1 code points (U+0080-U+009F) are
//!   ignored: in UTF-8 they never come from 8-bit controls by accident.

use alloc::string::String;

/// Parameters kept per control sequence.
pub const MAX_PARAMS: usize = 32;
/// Bytes of an OSC string kept.
pub const MAX_OSC: usize = 4096;

const ESC: char = '\x1b';
const BEL: char = '\x07';
const CAN: char = '\x18';
const SUB: char = '\x1a';
const DEL: char = '\x7f';
const ST_C1: char = '\u{9c}';

/// What the parser reports.
pub(crate) trait Perform {
    /// A character to draw.
    fn print(&mut self, c: char);
    /// A C0 control character (never ESC, CAN or SUB).
    fn execute(&mut self, c: char);
    /// `ESC <intermediates> <final>`.
    fn esc(&mut self, intermediates: &[u8], final_byte: u8);
    /// A complete control sequence.
    fn csi(&mut self, csi: &Csi);
    /// An operating system command and whether BEL (not ST) ended it.
    fn osc(&mut self, data: &str, bel: bool);
}

/// An incremental UTF-8 decoder.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Utf8 {
    cp: u32,
    /// Continuation bytes still expected.
    need: u8,
    /// The range the next continuation byte must be in.
    lo: u8,
    hi: u8,
}

impl Utf8 {
    /// Feed one byte; `out` receives the characters it completes.
    pub fn push(&mut self, b: u8, out: &mut impl FnMut(char)) {
        if self.need > 0 {
            if (self.lo..=self.hi).contains(&b) {
                self.cp = self.cp << 6 | u32::from(b & 0x3f);
                self.need -= 1;
                self.lo = 0x80;
                self.hi = 0xbf;
                if self.need == 0 {
                    out(char::from_u32(self.cp).unwrap_or(char::REPLACEMENT_CHARACTER));
                }
                return;
            }
            // The sequence is broken off; `b` starts afresh.
            self.need = 0;
            out(char::REPLACEMENT_CHARACTER);
        }
        let (need, lo, hi, bits) = match b {
            0x00..=0x7f => return out(char::from(b)),
            0xc2..=0xdf => (1, 0x80, 0xbf, b & 0x1f),
            0xe0 => (2, 0xa0, 0xbf, 0),
            0xe1..=0xec | 0xee..=0xef => (2, 0x80, 0xbf, b & 0x0f),
            0xed => (2, 0x80, 0x9f, b & 0x0f),
            0xf0 => (3, 0x90, 0xbf, 0),
            0xf1..=0xf3 => (3, 0x80, 0xbf, b & 0x07),
            0xf4 => (3, 0x80, 0x8f, 4),
            _ => return out(char::REPLACEMENT_CHARACTER),
        };
        *self = Utf8 { cp: u32::from(bits), need, lo, hi };
    }
}

/// A complete control sequence: `CSI <private> <params> <intermediates> <final>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Csi {
    params: [u16; MAX_PARAMS],
    /// Whether parameter `i` came after a `:` (is a sub-parameter of the
    /// one before).
    sub: [bool; MAX_PARAMS],
    len: usize,
    /// `<`, `=`, `>` or `?` right after `CSI`, else 0.
    pub private: u8,
    inter: [u8; 2],
    inter_len: usize,
    pub final_byte: u8,
}

impl Default for Csi {
    fn default() -> Csi {
        Csi {
            params: [0; MAX_PARAMS],
            sub: [false; MAX_PARAMS],
            len: 0,
            private: 0,
            inter: [0; 2],
            inter_len: 0,
            final_byte: 0,
        }
    }
}

impl Csi {
    /// How many parameters were given (an empty one counts, as 0).
    pub fn len(&self) -> usize {
        self.len
    }

    /// Parameter `i`, 0 if absent or empty.
    pub fn get(&self, i: usize) -> u16 {
        if i < self.len { self.params[i] } else { 0 }
    }

    /// Whether parameter `i` is a `:` sub-parameter.
    pub fn is_sub(&self, i: usize) -> bool {
        i < self.len && self.sub[i]
    }

    /// Parameter `i`, or `default` if it is absent or 0.
    pub fn or(&self, i: usize, default: u16) -> u16 {
        match self.get(i) {
            0 => default,
            n => n,
        }
    }

    /// The intermediate bytes before the final byte.
    pub fn intermediates(&self) -> &[u8] {
        &self.inter[..self.inter_len]
    }

    /// The parameters, at least one (0) even when none were given, as
    /// `CSI m` means `CSI 0 m`.
    pub fn params(&self) -> &[u16] {
        if self.len == 0 { &self.params[..1] } else { &self.params[..self.len] }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    /// A control sequence that will not be carried out: skip to its end.
    CsiIgnore,
    Osc,
    /// DCS, SOS, PM, APC: skip to the end.
    SkipString,
    /// `ESC` inside a string: `\` ends it.
    OscEscape,
    SkipEscape,
}

/// The VT parser with its UTF-8 decoder.
#[derive(Clone, Debug, Default)]
pub(crate) struct Parser {
    utf8: Utf8,
    state: State,
    csi: Csi,
    /// The parameter being read has overflowed [`MAX_PARAMS`].
    overflow: bool,
    esc_inter: [u8; 2],
    esc_inter_len: usize,
    osc: String,
}

impl Parser {
    /// Decode and carry out `bytes`.
    pub fn feed(&mut self, bytes: &[u8], p: &mut impl Perform) {
        let mut utf8 = self.utf8;
        for &b in bytes {
            utf8.push(b, &mut |c| self.advance(c, p));
        }
        self.utf8 = utf8;
    }

    /// Forget any partial sequence (not the UTF-8 state).
    pub fn reset(&mut self) {
        self.state = State::Ground;
        self.osc.clear();
    }

    /// Carry out one character.
    pub fn advance(&mut self, c: char, p: &mut impl Perform) {
        match c {
            CAN | SUB => {
                self.state = State::Ground;
                self.osc.clear();
                return;
            }
            DEL => return,
            ESC => {
                self.state = match self.state {
                    State::Osc => State::OscEscape,
                    State::SkipString => State::SkipEscape,
                    State::OscEscape => {
                        // `ESC ESC`: the string ends; a new escape starts.
                        self.end_osc(p, false);
                        self.esc_inter_len = 0;
                        State::Escape
                    }
                    _ => {
                        self.esc_inter_len = 0;
                        State::Escape
                    }
                };
                return;
            }
            _ => {}
        }
        match self.state {
            State::Ground => {
                if c < ' ' {
                    p.execute(c);
                } else if !('\u{80}'..='\u{9f}').contains(&c) {
                    p.print(c);
                }
            }
            State::Escape => self.escape(c, p),
            State::EscapeIntermediate => match c {
                '\0'..='\x1f' => p.execute(c),
                ' '..='/' => self.push_esc_inter(c),
                '0'..='~' => {
                    self.state = State::Ground;
                    p.esc(&self.esc_inter[..self.esc_inter_len], c as u8);
                }
                _ => self.state = State::Ground,
            },
            State::Csi | State::CsiIgnore => self.csi_char(c, p),
            State::Osc => match c {
                BEL => self.end_osc(p, true),
                ST_C1 => self.end_osc(p, false),
                '\0'..='\x1f' => {}
                _ => {
                    if self.osc.len() + c.len_utf8() <= MAX_OSC {
                        self.osc.push(c);
                    }
                }
            },
            State::SkipString => {
                if c == BEL || c == ST_C1 {
                    self.state = State::Ground;
                }
            }
            State::OscEscape => {
                self.end_osc(p, false);
                if c != '\\' {
                    self.escape(c, p);
                }
            }
            State::SkipEscape => {
                self.state = State::Ground;
                if c != '\\' {
                    self.escape(c, p);
                }
            }
        }
    }

    fn push_esc_inter(&mut self, c: char) {
        if self.esc_inter_len < self.esc_inter.len() {
            self.esc_inter[self.esc_inter_len] = c as u8;
            self.esc_inter_len += 1;
        }
        self.state = State::EscapeIntermediate;
    }

    fn end_osc(&mut self, p: &mut impl Perform, bel: bool) {
        self.state = State::Ground;
        let data = core::mem::take(&mut self.osc);
        p.osc(&data, bel);
        self.osc = data;
        self.osc.clear();
    }

    /// The character after `ESC`.
    fn escape(&mut self, c: char, p: &mut impl Perform) {
        self.state = State::Ground;
        match c {
            '[' => {
                self.state = State::Csi;
                self.csi = Csi::default();
                self.overflow = false;
            }
            ']' => {
                self.state = State::Osc;
                self.osc.clear();
            }
            'P' | 'X' | '^' | '_' => self.state = State::SkipString,
            '\0'..='\x1f' => {
                self.state = State::Escape;
                p.execute(c);
            }
            ' '..='/' => {
                self.esc_inter_len = 0;
                self.push_esc_inter(c);
            }
            '0'..='~' => p.esc(&[], c as u8),
            _ => {}
        }
    }

    fn csi_char(&mut self, c: char, p: &mut impl Perform) {
        let csi = &mut self.csi;
        match c {
            '\0'..='\x1f' => p.execute(c),
            '0'..='9' if csi.inter_len == 0 => {
                if csi.len == 0 {
                    csi.len = 1;
                }
                if !self.overflow {
                    let v = &mut csi.params[csi.len - 1];
                    *v = v.saturating_mul(10).saturating_add(c as u16 - u16::from(b'0'));
                }
            }
            ';' | ':' if csi.inter_len == 0 => {
                if csi.len == 0 {
                    csi.len = 1;
                }
                if csi.len < MAX_PARAMS {
                    csi.params[csi.len] = 0;
                    csi.sub[csi.len] = c == ':';
                    csi.len += 1;
                } else {
                    self.overflow = true;
                }
            }
            '<'..='?' if csi.len == 0 && csi.private == 0 && csi.inter_len == 0 => csi.private = c as u8,
            ' '..='/' => {
                if csi.inter_len < csi.inter.len() {
                    csi.inter[csi.inter_len] = c as u8;
                    csi.inter_len += 1;
                } else {
                    self.state = State::CsiIgnore;
                }
            }
            '@'..='~' => {
                let ignore = self.state == State::CsiIgnore;
                self.state = State::Ground;
                csi.final_byte = c as u8;
                if !ignore {
                    let csi = self.csi;
                    p.csi(&csi);
                }
            }
            // Malformed: skip to the final byte.
            _ => self.state = State::CsiIgnore,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    #[derive(Default)]
    struct Log(Vec<String>);

    impl Perform for Log {
        fn print(&mut self, c: char) {
            self.0.push(format!("p{c}"));
        }
        fn execute(&mut self, c: char) {
            self.0.push(format!("x{:02x}", c as u32));
        }
        fn esc(&mut self, i: &[u8], f: u8) {
            self.0.push(format!("e{}{}", String::from_utf8_lossy(i), f as char));
        }
        fn csi(&mut self, c: &Csi) {
            let params: Vec<String> =
                (0..c.len()).map(|i| format!("{}{}", if c.is_sub(i) { ":" } else { "" }, c.get(i))).collect();
            self.0.push(format!(
                "c{}{}{}{}",
                if c.private != 0 { c.private as char } else { '_' },
                params.join(","),
                String::from_utf8_lossy(c.intermediates()),
                c.final_byte as char
            ));
        }
        fn osc(&mut self, d: &str, bel: bool) {
            self.0.push(format!("o{d}{}", if bel { "^G" } else { "ST" }));
        }
    }

    fn run(chunks: &[&[u8]]) -> Vec<String> {
        let mut p = Parser::default();
        let mut log = Log::default();
        for c in chunks {
            p.feed(c, &mut log);
        }
        log.0
    }

    #[test]
    fn utf8_split_across_feeds() {
        let bytes = "aé中😀".as_bytes();
        for split in 0..bytes.len() {
            let (a, b) = bytes.split_at(split);
            assert_eq!(run(&[a, b]), ["pa", "pé", "p中", "p😀"], "split at {split}");
        }
    }

    #[test]
    fn utf8_malformed() {
        assert_eq!(run(&[b"\x80a"]), ["p\u{FFFD}", "pa"]);
        assert_eq!(run(&[b"\xc3a"]), ["p\u{FFFD}", "pa"]);
        // Overlong, surrogate, past U+10FFFF, invalid lead bytes.
        assert_eq!(run(&[b"\xc0\xafx"]), ["p\u{FFFD}", "p\u{FFFD}", "px"]);
        assert_eq!(run(&[b"\xe0\x80\x80"]), ["p\u{FFFD}", "p\u{FFFD}", "p\u{FFFD}"]);
        assert_eq!(run(&[b"\xed\xa0\x80"]), ["p\u{FFFD}", "p\u{FFFD}", "p\u{FFFD}"]);
        assert_eq!(run(&[b"\xf4\x90\x80\x80"]).len(), 4);
        assert_eq!(run(&[b"\xff"]), ["p\u{FFFD}"]);
        // A truncated sequence followed by ESC still lets ESC through.
        assert_eq!(run(&[b"\xe4\xb8\x1b[A"]), ["p\u{FFFD}", "c_A"]);
    }

    #[test]
    fn csi_params() {
        assert_eq!(run(&[b"\x1b[1;22H"]), ["c_1,22H"]);
        assert_eq!(run(&[b"\x1b[;5H"]), ["c_0,5H"]);
        assert_eq!(run(&[b"\x1b[H"]), ["c_H"]);
        assert_eq!(run(&[b"\x1b[?1049h"]), ["c?1049h"]);
        assert_eq!(run(&[b"\x1b[38:2::1:2:3m"]), ["c_38,:2,:0,:1,:2,:3m"]);
        assert_eq!(run(&[b"\x1b[2 q"]), ["c_2 q"]);
        assert_eq!(run(&[b"\x1b[99999999999999A"]), ["c_65535A"]);
    }

    #[test]
    fn csi_split_across_feeds() {
        let s = b"\x1b[38;5;196mX";
        for split in 0..s.len() {
            let (a, b) = s.split_at(split);
            assert_eq!(run(&[a, b]), ["c_38,5,196m", "pX"]);
        }
    }

    #[test]
    fn too_many_params_are_dropped() {
        let mut s = String::from("\x1b[");
        for i in 0..40 {
            s.push_str(&format!("{i};"));
        }
        s.push('m');
        let log = run(&[s.as_bytes()]);
        assert_eq!(log.len(), 1);
        assert!(log[0].starts_with("c_0,1,2,"));
        assert!(log[0].ends_with(",31m"));
    }

    #[test]
    fn malformed_csi_is_skipped() {
        assert_eq!(run(&[b"\x1b[1?2hX"]), ["pX"]);
        assert_eq!(run(&[b"\x1b[ 1qX"]), ["pX"]);
        assert_eq!(run(&[b"\x1b[1\xc3\xa9AX"]), ["pX"]);
        assert_eq!(run(&[b"\x1b[1!!!qX"]), ["pX"]);
    }

    #[test]
    fn controls_inside_sequences() {
        assert_eq!(run(&[b"\x1b[1\n2H"]), ["x0a", "c_12H"]);
        assert_eq!(run(&[b"\x1b[12\x18AB"]), ["pA", "pB"]);
        assert_eq!(run(&[b"\x1b[12\x1aAB"]), ["pA", "pB"]);
        assert_eq!(run(&[b"\x1b[12\x1b[3A"]), ["c_3A"]);
        assert_eq!(run(&[b"a\x7fb"]), ["pa", "pb"]);
        assert_eq!(run(&[b"\x1b\x08c"]), ["x08", "ec"]);
    }

    #[test]
    fn escape_sequences() {
        assert_eq!(run(&[b"\x1b7\x1b(0\x1b#8\x1bM"]), ["e7", "e(0", "e#8", "eM"]);
        assert_eq!(run(&[b"\x1b\xc3\xa9x"]), ["px"]);
    }

    #[test]
    fn osc_strings() {
        assert_eq!(run(&[b"\x1b]0;hi\x07x"]), ["o0;hi^G", "px"]);
        assert_eq!(run(&[b"\x1b]2;h\xc3", b"\xa9\x1b", b"\\x"]), ["o2;héST", "px"]);
        assert_eq!(run(&[b"\x1b]2;a\x1b[1Ax"]), ["o2;aST", "c_1A", "px"]);
        assert_eq!(run(&[b"\x1b]2;a\x18x"]), ["px"]);
        let mut long = alloc::vec![b'a'; MAX_OSC * 2];
        long.splice(0..0, *b"\x1b]2;");
        long.push(7);
        let log = run(&[&long]);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].len(), 1 + MAX_OSC + 2);
    }

    #[test]
    fn dcs_and_apc_are_skipped() {
        assert_eq!(run(&[b"\x1bP1$r0m\x1b\\x"]), ["px"]);
        assert_eq!(run(&[b"\x1b_Gf=24;AAAA\x1b\\x"]), ["px"]);
        assert_eq!(run(&[b"\x1bPabc\x1b[Ax"]), ["c_A", "px"]);
        assert_eq!(run(&[b"\x1b^pm\x07x"]), ["px"]);
    }

    #[test]
    fn c1_code_points_are_ignored() {
        assert_eq!(run(&["a\u{9b}b".as_bytes()]), ["pa", "pb"]);
    }
}
