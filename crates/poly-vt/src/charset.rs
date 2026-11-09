//! The character sets a program can switch G0 and G1 to.
//!
//! `ESC ( F` designates G0 and `ESC ) F` G1; SI (0x0f) and SO (0x0e) pick
//! which one is in use. `F` is `B` for US ASCII (the default), `0` for the
//! DEC Special Graphics set (the VT100 line-drawing characters, used by
//! `ncurses` when it cannot rely on UTF-8 box drawing) and `A` for the
//! UK set (`#` is `£`). Any other set is treated as ASCII.
//!
//! The DEC Special Graphics glyphs (VT100 User Guide, table 3-9) map to the
//! Unicode characters that look the same.

/// A 94-character set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Charset {
    #[default]
    Ascii,
    DecGraphics,
    Uk,
}

impl Charset {
    /// The set an `ESC (` / `ESC )` final byte names.
    pub fn from_final(f: u8) -> Charset {
        match f {
            b'0' => Charset::DecGraphics,
            b'A' => Charset::Uk,
            _ => Charset::Ascii,
        }
    }

    /// What `c` shows as in this set.
    pub fn map(self, c: char) -> char {
        match self {
            Charset::Ascii => c,
            Charset::Uk => {
                if c == '#' {
                    '£'
                } else {
                    c
                }
            }
            Charset::DecGraphics => dec_graphics(c),
        }
    }
}

fn dec_graphics(c: char) -> char {
    match c {
        '_' => ' ',
        '`' => '◆',
        'a' => '▒',
        'b' => '␉',
        'c' => '␌',
        'd' => '␍',
        'e' => '␊',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_drawing() {
        let g = Charset::from_final(b'0');
        let s: alloc::string::String = "lqkxjmtuvwn".chars().map(|c| g.map(c)).collect();
        assert_eq!(s, "┌─┐│┘└├┤┴┬┼");
        assert_eq!(g.map('A'), 'A');
        assert_eq!(Charset::from_final(b'B').map('q'), 'q');
        assert_eq!(Charset::from_final(b'A').map('#'), '£');
    }
}
