//! Turning a character stream into printable characters, control
//! characters and escape sequences, and bytes into characters.
//!
//! The parser is the usual VT state machine (ECMA-48 §5.4 for the shape of
//! control sequences): `ESC [` starts a control sequence (CSI) of numeric
//! parameters separated by `;`, an optional private marker (`?`, `>`, `=`,
//! `<`) first, intermediate bytes 0x20-0x2f, and a final byte 0x40-0x7e.
//! `ESC ]` (OSC), `ESC P` (DCS), `ESC X`, `ESC ^` and `ESC _` start strings
//! that run to BEL or `ESC \` and are skipped. Control characters inside a
//! sequence are carried out as if they came before it; ESC starts over, CAN
//! and SUB cancel. Nothing here allocates or can be made to: parameters past
//! [`MAX_PARAMS`] are dropped and values saturate.

/// Parameters kept per control sequence.
pub const MAX_PARAMS: usize = 16;

const ESC: char = '\x1b';
const BEL: char = '\x07';
const CAN: char = '\x18';
const SUB: char = '\x1a';

/// A complete control sequence: `ESC [ <private> <params> <intermediates> <final>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Csi {
    params: [u16; MAX_PARAMS],
    len: usize,
    /// `?`, `>`, `=` or `<` right after `ESC [`, else 0.
    pub private: u8,
    /// Whether intermediate bytes came before the final byte (none of the
    /// sequences carried out have any).
    pub intermediates: bool,
    pub final_byte: u8,
}

impl Csi {
    /// How many parameters were given (an empty one counts, as 0).
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Parameter `i`, 0 if absent or empty.
    pub fn get(&self, i: usize) -> u16 {
        if i < self.len { self.params[i] } else { 0 }
    }

    /// Parameter `i`, or `default` if it is absent or 0 (cursor movements
    /// treat 0 as 1).
    pub fn or(&self, i: usize, default: u16) -> u16 {
        match self.get(i) {
            0 => default,
            n => n,
        }
    }
}

/// What a character means, as far as the parser can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Part of a sequence, or skipped.
    None,
    /// Draw it.
    Print(char),
    /// A control character (C0, DEL or C1) to carry out.
    Execute(char),
    /// `ESC` followed by this final character.
    Esc(char),
    Csi(Csi),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    /// `ESC` and a character-set or similar intermediate: one more
    /// character ends it.
    EscapeArgument,
    Csi,
    /// A control sequence that cannot be carried out: skip to its end.
    CsiIgnore,
    /// A control string (OSC, DCS, ...): skip to BEL or `ESC \`.
    String,
    /// `ESC` inside a control string.
    StringEscape,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Parser {
    state: State,
    csi: Csi,
    /// Whether the parameter being read has a digit yet.
    digits: bool,
}

impl Parser {
    pub fn reset(&mut self) {
        *self = Parser::default();
    }

    pub fn advance(&mut self, c: char) -> Action {
        // CAN and SUB cancel any sequence; ESC starts a new one, except
        // inside a control string, where it may be the start of ST.
        match c {
            CAN | SUB => {
                self.state = State::Ground;
                return Action::Execute(c);
            }
            ESC if self.state == State::String => {
                self.state = State::StringEscape;
                return Action::None;
            }
            ESC => {
                self.state = State::Escape;
                return Action::None;
            }
            _ => {}
        }
        match self.state {
            State::Ground => {
                if is_control(c) {
                    Action::Execute(c)
                } else {
                    Action::Print(c)
                }
            }
            State::Escape => self.escape(c),
            State::EscapeArgument => {
                if is_control(c) {
                    return Action::Execute(c);
                }
                self.state = State::Ground;
                Action::None
            }
            State::Csi | State::CsiIgnore => self.csi(c),
            State::String => {
                if c == BEL {
                    self.state = State::Ground;
                }
                Action::None
            }
            // `ESC \` ends the string; any other `ESC x` is an escape
            // sequence after an unterminated string.
            State::StringEscape if c == '\\' => {
                self.state = State::Ground;
                Action::None
            }
            State::StringEscape => self.escape(c),
        }
    }

    fn escape(&mut self, c: char) -> Action {
        match c {
            '[' => {
                self.state = State::Csi;
                self.csi = Csi::default();
                self.digits = false;
                Action::None
            }
            ']' | 'P' | 'X' | '^' | '_' => {
                self.state = State::String;
                Action::None
            }
            ' '..='/' => {
                self.state = State::EscapeArgument;
                Action::None
            }
            c if is_control(c) => Action::Execute(c),
            '0'..='~' => {
                self.state = State::Ground;
                Action::Esc(c)
            }
            _ => {
                self.state = State::Ground;
                Action::None
            }
        }
    }

    fn csi(&mut self, c: char) -> Action {
        let csi = &mut self.csi;
        let first = csi.len == 0 && !self.digits && csi.private == 0;
        match c {
            c if is_control(c) => return Action::Execute(c),
            '0'..='9' if !csi.intermediates => {
                if csi.len == 0 {
                    csi.len = 1;
                }
                if let Some(p) = csi.params.get_mut(csi.len - 1) {
                    *p = p.saturating_mul(10).saturating_add(c as u16 - '0' as u16);
                }
                self.digits = true;
            }
            ';' | ':' if !csi.intermediates => {
                // An empty first parameter still counts.
                csi.len = if csi.len == 0 { 2 } else { csi.len.saturating_add(1) }.min(MAX_PARAMS + 1);
                self.digits = false;
            }
            '<'..='?' if first => csi.private = c as u8,
            ' '..='/' => csi.intermediates = true,
            '@'..='~' => {
                let ignore = self.state == State::CsiIgnore;
                self.state = State::Ground;
                csi.len = csi.len.min(MAX_PARAMS);
                csi.final_byte = c as u8;
                return if ignore { Action::None } else { Action::Csi(*csi) };
            }
            // A private marker in the middle, a digit after an
            // intermediate: malformed, skip to the final byte.
            '0'..='?' => self.state = State::CsiIgnore,
            // Not ASCII: the sequence is broken off.
            _ => {
                self.state = State::Ground;
                return Action::None;
            }
        }
        Action::None
    }
}

/// C0 controls, DEL and C1 controls.
fn is_control(c: char) -> bool {
    matches!(c, '\0'..='\x1f' | '\x7f'..='\u{9f}')
}

/// An incremental UTF-8 decoder: malformed input (stray continuation bytes,
/// truncated or overlong sequences, surrogates, values past U+10FFFF)
/// becomes U+FFFD, one per broken sequence.
#[derive(Clone, Copy, Debug, Default)]
pub struct Utf8 {
    cp: u32,
    /// Continuation bytes still expected.
    need: u8,
    /// Length of the sequence being read.
    len: u8,
}

impl Utf8 {
    /// Feed one byte. Returns the characters it completes: at most a
    /// U+FFFD for a sequence it broke off, then its own.
    pub fn push(&mut self, b: u8) -> [Option<char>; 2] {
        if self.need == 0 {
            return [self.start(b), None];
        }
        if b & 0xc0 != 0x80 {
            self.need = 0;
            return [Some(char::REPLACEMENT_CHARACTER), self.start(b)];
        }
        self.cp = self.cp << 6 | (b & 0x3f) as u32;
        self.need -= 1;
        if self.need > 0 {
            return [None, None];
        }
        let min = match self.len {
            2 => 0x80,
            3 => 0x800,
            _ => 0x1_0000,
        };
        let c = if self.cp < min { None } else { char::from_u32(self.cp) };
        [Some(c.unwrap_or(char::REPLACEMENT_CHARACTER)), None]
    }

    /// U+FFFD if a sequence was left unfinished; the decoder starts over.
    pub fn flush(&mut self) -> Option<char> {
        let broken = self.need > 0;
        self.need = 0;
        broken.then_some(char::REPLACEMENT_CHARACTER)
    }

    fn start(&mut self, b: u8) -> Option<char> {
        let (len, bits) = match b {
            0x00..=0x7f => return Some(b as char),
            0xc2..=0xdf => (2, b & 0x1f),
            0xe0..=0xef => (3, b & 0x0f),
            0xf0..=0xf4 => (4, b & 0x07),
            // Continuation bytes, 0xc0/0xc1 (always overlong), 0xf5-0xff.
            _ => return Some(char::REPLACEMENT_CHARACTER),
        };
        self.cp = bits as u32;
        self.len = len;
        self.need = len - 1;
        None
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    fn actions(s: &str) -> Vec<Action> {
        let mut p = Parser::default();
        s.chars().map(|c| p.advance(c)).filter(|a| *a != Action::None).collect()
    }

    fn csi(s: &str) -> Csi {
        match actions(s).as_slice() {
            [Action::Csi(c)] => *c,
            other => panic!("{s:?} gave {other:?}"),
        }
    }

    #[test]
    fn text_and_controls() {
        assert_eq!(
            actions("a\nä\x7f\u{85}"),
            [
                Action::Print('a'),
                Action::Execute('\n'),
                Action::Print('ä'),
                Action::Execute('\x7f'),
                Action::Execute('\u{85}')
            ]
        );
    }

    #[test]
    fn control_sequences() {
        let c = csi("\x1b[1;31m");
        assert_eq!((c.final_byte, c.len(), c.get(0), c.get(1), c.get(2)), (b'm', 2, 1, 31, 0));
        let c = csi("\x1b[m");
        assert_eq!((c.final_byte, c.len()), (b'm', 0));
        let c = csi("\x1b[;5H");
        assert_eq!((c.len(), c.get(0), c.or(0, 1), c.get(1)), (2, 0, 1, 5));
        let c = csi("\x1b[?25l");
        assert_eq!((c.private, c.get(0), c.final_byte), (b'?', 25, b'l'));
        let c = csi("\x1b[99999999A");
        assert_eq!(c.get(0), u16::MAX);
        let c = csi("\x1b[38:2:1:2:3m");
        assert_eq!((c.len(), c.get(4)), (5, 3));
        let c = csi("\x1b[ q");
        assert!(c.intermediates);
        // More parameters than kept: the rest are dropped.
        let long: std::string::String = (0..40).map(|i| std::format!("{i};")).collect();
        let c = csi(&std::format!("\x1b[{long}m"));
        assert_eq!((c.len(), c.get(15)), (MAX_PARAMS, 15));
    }

    #[test]
    fn controls_inside_a_sequence_run_first() {
        assert_eq!(actions("\x1b[1\n2H")[0], Action::Execute('\n'));
        assert!(matches!(actions("\x1b[1\n2H")[1], Action::Csi(c) if c.get(0) == 12));
    }

    #[test]
    fn malformed_and_cancelled_sequences() {
        // A private marker after a parameter: skipped, including the final.
        assert_eq!(actions("\x1b[1?2hx"), [Action::Print('x')]);
        // CAN cancels; ESC restarts.
        assert_eq!(actions("\x1b[12\x18x"), [Action::Execute('\x18'), Action::Print('x')]);
        assert!(matches!(actions("\x1b[12\x1b[3Ax").as_slice(), [Action::Csi(c), Action::Print('x')] if c.get(0) == 3));
        // A non-ASCII character breaks the sequence off.
        assert_eq!(actions("\x1b[1éx"), [Action::Print('x')]);
    }

    #[test]
    fn escape_sequences_and_strings() {
        assert_eq!(actions("\x1b7\x1bMx"), [Action::Esc('7'), Action::Esc('M'), Action::Print('x')]);
        // Character-set designations take one more character.
        assert_eq!(actions("\x1b(Bx"), [Action::Print('x')]);
        // OSC to BEL, and DCS to ESC \, are skipped entirely.
        assert_eq!(actions("\x1b]0;title\x07x"), [Action::Print('x')]);
        assert_eq!(actions("\x1bPq#0;1\x1b\\x"), [Action::Print('x')]);
        // An escape sequence right after an unterminated string still works.
        assert_eq!(actions("\x1b]0;t\x1b7x"), [Action::Esc('7'), Action::Print('x')]);
    }

    fn decode(bytes: &[u8]) -> std::string::String {
        let mut d = Utf8::default();
        let mut s: std::string::String = bytes.iter().flat_map(|&b| d.push(b)).flatten().collect();
        s.extend(d.flush());
        s
    }

    #[test]
    fn utf8_decodes_and_replaces_garbage() {
        assert_eq!(decode("Grüezi € 𝄞".as_bytes()), "Grüezi € 𝄞");
        assert_eq!(decode(b"a\x80b"), "a\u{fffd}b");
        assert_eq!(decode(b"\xc3"), "\u{fffd}");
        assert_eq!(decode(b"\xc3a"), "\u{fffd}a");
        assert_eq!(decode(b"\xc0\xaf"), "\u{fffd}\u{fffd}"); // overlong '/'
        assert_eq!(decode(b"\xe0\x80\xaf"), "\u{fffd}"); // overlong
        assert_eq!(decode(b"\xed\xa0\x80"), "\u{fffd}"); // surrogate
        assert_eq!(decode(b"\xf4\x90\x80\x80"), "\u{fffd}"); // past U+10FFFF
        assert_eq!(decode(b"\xff\xfe"), "\u{fffd}\u{fffd}");
        // Any byte sequence decodes to as many characters as std's lossy
        // decoder finds, give or take how it splits broken sequences.
        let mut rng = 0x1234_5678u32;
        for _ in 0..2000 {
            let bytes: Vec<u8> = (0..8)
                .map(|_| {
                    rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (rng >> 16) as u8
                })
                .collect();
            let ours = decode(&bytes);
            let std = std::string::String::from_utf8_lossy(&bytes);
            let valid = |s: &str| s.chars().filter(|&c| c != char::REPLACEMENT_CHARACTER).collect::<Vec<_>>();
            assert_eq!(valid(&ours), valid(&std), "{bytes:x?}");
        }
    }
}
