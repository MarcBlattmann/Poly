//! The font dialog's arithmetic: point sizes and `lfHeight`, the sizes
//! and styles it offers.

/// `MulDiv` with rounding (as `kernel32`'s).
pub fn muldiv(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return -1;
    }
    let p = i64::from(a) * i64::from(b);
    let c = i64::from(c);
    let half = c.abs() / 2;
    let r = if (p < 0) == (c < 0) { (p + half) / c } else { (p - half) / c };
    r as i32
}

/// `lfHeight` of a font of `points` at `dpi` (negative: the character height).
pub fn height_for_points(points: i32, dpi: i32) -> i32 {
    -muldiv(points, dpi, 72)
}

/// The point size of `lfHeight` at `dpi`.
pub fn points_for_height(height: i32, dpi: i32) -> i32 {
    muldiv(height.abs(), 72, dpi)
}

/// The sizes the size list shows.
pub const SIZES: [i32; 16] = [8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 26, 28, 36, 48, 72];

/// A style of the style list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub name: &'static str,
    pub weight: i32,
    pub italic: bool,
}

pub const FW_NORMAL: i32 = 400;
pub const FW_BOLD: i32 = 700;

pub const STYLES: [Style; 4] = [
    Style { name: "Regular", weight: FW_NORMAL, italic: false },
    Style { name: "Italic", weight: FW_NORMAL, italic: true },
    Style { name: "Bold", weight: FW_BOLD, italic: false },
    Style { name: "Bold Italic", weight: FW_BOLD, italic: true },
];

/// The style closest to a weight and slant.
pub fn style_index(weight: i32, italic: bool) -> usize {
    let bold = weight >= 600;
    STYLES.iter().position(|s| (s.weight >= 600) == bold && s.italic == italic).unwrap_or(0)
}

/// The size list's index of `points`, or the nearest at most.
pub fn size_index(points: i32) -> usize {
    SIZES.iter().rposition(|&s| s <= points).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_and_points() {
        assert_eq!(height_for_points(12, 96), -16);
        assert_eq!(height_for_points(9, 96), -12);
        assert_eq!(points_for_height(-12, 96), 9);
        assert_eq!(points_for_height(16, 96), 12);
        assert_eq!(muldiv(1, 1, 0), -1);
        assert_eq!(muldiv(-3, 1, 2), -2);
    }

    #[test]
    fn style_and_size_lists() {
        assert_eq!(style_index(400, false), 0);
        assert_eq!(style_index(700, true), 3);
        assert_eq!(style_index(900, false), 2);
        assert_eq!(STYLES[style_index(400, true)].name, "Italic");
        assert_eq!(SIZES[size_index(10)], 10);
        assert_eq!(SIZES[size_index(13)], 12);
        assert_eq!(SIZES[size_index(1)], 8);
    }
}
