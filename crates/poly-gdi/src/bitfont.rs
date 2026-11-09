//! Glyphs for the stock fonts out of Spleen 8x16 (`poly-gfx`, BSD 2-Clause,
//! `third_party/spleen/LICENSE`): the fixed faces, the GUI face and the
//! characters the system font's hand-set ASCII glyphs (`sysfont`) lack.
//!
//! A glyph here is 16 rows of `u16`, bit 15 the leftmost pixel, row 0 the
//! top of the face's cell. Spleen's cell has its baseline after row 11
//! (ascent 12); a face with a smaller ascent shows the rows from
//! `12 - ascent` on, so the baselines agree.

use poly_gfx::SPLEEN_8X16;

/// Spleen's ascent: the baseline lies below this many rows.
const SPLEEN_ASCENT: usize = 12;

/// Spleen's rows for `c`, or its box for a character it lacks (`None` for
/// surrogates and the like).
fn spleen(c: u16, fallback: bool) -> Option<[u16; 16]> {
    let ch = char::from_u32(u32::from(c))?;
    let g = if fallback { SPLEEN_8X16.glyph(ch) } else { SPLEEN_8X16.lookup(ch)? };
    let mut rows = [0u16; 16];
    for (y, r) in rows.iter_mut().enumerate() {
        // The pixel row has the leftmost of Spleen's 8 pixels in bit 31.
        *r = (g.row(y as u32) >> 16) as u16;
    }
    Some(rows)
}

/// Spleen's glyph for `c` if it has one.
pub(crate) fn lookup(c: u16) -> Option<[u16; 16]> {
    spleen(c, false)
}

/// The face's cell, cropped from Spleen's: `ascent` rows above the
/// baseline and `height` rows in all; a descender that would pass the cell's
/// end is cut off.
pub(crate) fn cell(c: u16, ascent: usize, height: usize) -> [u16; 16] {
    let rows = spleen(c, true).unwrap_or([0; 16]);
    let top = SPLEEN_ASCENT - ascent;
    let mut out = [0u16; 16];
    for (y, o) in out.iter_mut().enumerate().take(height) {
        *o = rows.get(top + y).copied().unwrap_or(0);
    }
    out
}

/// Spleen's strokes are as heavy as the bold system font's, so a glyph
/// only moves down one row for the system font's ascent of 13.
pub(crate) fn lower_for_system(rows: [u16; 16]) -> [u16; 16] {
    let mut out = [0u16; 16];
    out[1..].copy_from_slice(&rows[..15]);
    out
}

/// `rows` squeezed or stretched to an advance of `adv` pixels, one of them
/// the gap to the next character: the columns the glyph ink spans are
/// thinned (the emptiest column goes first, so stems stay and bars and
/// arcs shorten) or widened (the middle column repeats) to `adv - 1`
/// columns, flush left. An empty glyph or an advance under 2 gives no
/// ink.
pub(crate) fn fit(rows: [u16; 16], adv: i32) -> [u16; 16] {
    let any = rows.iter().fold(0u16, |a, &r| a | r);
    if any == 0 || adv < 2 {
        return [0; 16];
    }
    let target = (adv - 1).min(15) as usize;
    let first = any.leading_zeros() as usize;
    let last = 15 - any.trailing_zeros() as usize;
    // One mask per ink column: bit y set when row y has ink there.
    let mut cols = [0u16; 16];
    let mut n = 0;
    for x in first..=last {
        let mut m = 0u16;
        for (y, &r) in rows.iter().enumerate() {
            if r & (0x8000 >> x) != 0 {
                m |= 1 << y;
            }
        }
        cols[n] = m;
        n += 1;
    }
    while n > target {
        // The emptiest column; among equals the one nearest the middle.
        let mid = n - 1;
        let drop = (0..n).min_by_key(|&i| (cols[i].count_ones(), (2 * i).abs_diff(mid))).unwrap_or(0);
        cols.copy_within(drop + 1..n, drop);
        n -= 1;
    }
    while n < target {
        let at = n / 2;
        cols.copy_within(at..n, at + 1);
        n += 1;
    }
    let mut out = [0u16; 16];
    for (x, &m) in cols.iter().enumerate().take(n) {
        for (y, o) in out.iter_mut().enumerate() {
            if m & (1 << y) != 0 {
                *o |= 0x8000 >> x;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A glyph of 5 rows from strings of `#` and `.`.
    fn g(rows: &[&str]) -> [u16; 16] {
        let mut out = [0u16; 16];
        for (y, r) in rows.iter().enumerate() {
            for (x, c) in r.chars().enumerate() {
                if c == '#' {
                    out[y] |= 0x8000 >> x;
                }
            }
        }
        out
    }

    #[test]
    fn fitting_thins_the_emptiest_column_first() {
        // An `i`: a serif column, the stem and a foot column.
        let i = g(&[".#.", "##.", ".#.", ".#.", ".#.", "###"]);
        assert_eq!(fit(i, 2), g(&["#", "#", "#", "#", "#", "#"]));
        // An `o` loses a column of its arcs, not a side.
        let o = g(&[".###.", "#...#", "#...#", "#...#", ".###."]);
        assert_eq!(fit(o, 5), g(&[".##.", "#..#", "#..#", "#..#", ".##."]));
    }

    #[test]
    fn fitting_widens_by_repeating_the_middle() {
        let t = g(&["###", ".#.", ".#."]);
        assert_eq!(fit(t, 6), g(&["#####", ".###.", ".###."]));
        // The same width changes nothing but position: flush left.
        assert_eq!(fit(g(&[".##", ".#."]), 3), g(&["##", "#."]));
    }

    #[test]
    fn no_ink_without_room_or_glyph() {
        assert_eq!(fit([0; 16], 8), [0; 16]);
        assert_eq!(fit(g(&["###"]), 1), [0; 16]);
        assert_eq!(fit(g(&["###"]), 0), [0; 16]);
        // A huge advance stays inside the 16 columns.
        let wide = fit(g(&["#"]), 99);
        assert_eq!(wide[0], 0xFFFE);
    }

    #[test]
    fn cells_are_cropped_to_the_baseline() {
        // Spleen's `H` is ten rows tall, ending on its baseline (row 11).
        let h = lookup(u16::from(b'H')).unwrap();
        let first = h.iter().position(|&r| r != 0).unwrap();
        let last = h.iter().rposition(|&r| r != 0).unwrap();
        assert_eq!(last, 11);
        let c = cell(u16::from(b'H'), 11, 13);
        assert_eq!(c[last - 1], h[last], "one row up for ascent 11");
        assert_eq!(c[first - 1], h[first]);
        assert_eq!(&c[13..], &[0, 0, 0], "nothing below the cell");
        assert_eq!(lower_for_system(h)[last + 1], h[last]);
        assert!(lookup(0x4E2D).is_none());
        assert!(lookup(0xD800).is_none());
    }
}
