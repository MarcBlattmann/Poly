//! The colour dialog's arithmetic: the basic colours it offers, and the
//! hue / saturation / luminance scale its edit fields use (hue 0..=239,
//! saturation and luminance 0..=240, as the dialog has shown them).
//! `COLORREF`s are `0x00BBGGRR`.

/// `RGB(r, g, b)`.
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | (g as u32) << 8 | (b as u32) << 16
}

const fn hex(c: u32) -> u32 {
    rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// The 48 basic colours, row by row, eight to a row (as `0xRRGGBB` written, stored as `COLORREF`).
pub const BASIC: [u32; 48] = [
    hex(0xFF8080),
    hex(0xFFFF80),
    hex(0x80FF80),
    hex(0x00FF80),
    hex(0x80FFFF),
    hex(0x0080FF),
    hex(0xFF80C0),
    hex(0xFF80FF),
    hex(0xFF0000),
    hex(0xFFFF00),
    hex(0x80FF00),
    hex(0x00FF40),
    hex(0x00FFFF),
    hex(0x0080C0),
    hex(0x8080C0),
    hex(0xFF00FF),
    hex(0x804040),
    hex(0xFF8040),
    hex(0x00FF00),
    hex(0x008080),
    hex(0x004080),
    hex(0x8080FF),
    hex(0x800040),
    hex(0xFF0080),
    hex(0x800000),
    hex(0xFF8000),
    hex(0x008000),
    hex(0x008040),
    hex(0x0000FF),
    hex(0x0000A0),
    hex(0x800080),
    hex(0x8000FF),
    hex(0x400000),
    hex(0x804000),
    hex(0x004000),
    hex(0x004040),
    hex(0x000080),
    hex(0x000040),
    hex(0x400040),
    hex(0x400080),
    hex(0x000000),
    hex(0x808000),
    hex(0x808040),
    hex(0x808080),
    hex(0x408080),
    hex(0xC0C0C0),
    hex(0x404040),
    hex(0xFFFFFF),
];

/// Scale of the saturation and luminance fields (and one more than the hue's).
pub const HSL_MAX: u32 = 240;
/// Hue wraps at this value.
pub const HUE_MAX: u32 = 240;

/// `(hue 0..=239, saturation 0..=240, luminance 0..=240)` of a `COLORREF`.
pub fn to_hsl(c: u32) -> (u32, u32, u32) {
    let (r, g, b) = ((c & 255) as i32, (c >> 8 & 255) as i32, (c >> 16 & 255) as i32);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let sum = max + min;
    // Luminance in 0..=240.
    let l = (sum * 240 + 255) / 510;
    if max == min {
        return (0, 0, l as u32);
    }
    let d = max - min;
    let s = if sum <= 255 { d * 240 / sum } else { d * 240 / (510 - sum) };
    // Hue in sixths of the circle, then scaled to 240.
    let h6 = if max == r {
        (g - b) * 240 / d
    } else if max == g {
        480 + (b - r) * 240 / d
    } else {
        960 + (r - g) * 240 / d
    };
    // h6 is in 0..1440 for the six sectors of 240; the circle is 1440 -> 240.
    let h = (h6.rem_euclid(1440)) / 6;
    (h as u32 % HUE_MAX, s as u32, l as u32)
}

/// A `COLORREF` from hue, saturation and luminance of the dialog's scales (values are clamped).
pub fn from_hsl(h: u32, s: u32, l: u32) -> u32 {
    let h = h % HUE_MAX;
    let s = s.min(HSL_MAX) as i32;
    let l = l.min(HSL_MAX) as i32;
    if s == 0 {
        let v = (l * 255 + 120) / 240;
        return rgb(v as u8, v as u8, v as u8);
    }
    // In 0..=1000 fixed point.
    let ll = l * 1000 / 240;
    let ss = s * 1000 / 240;
    let q = if ll < 500 { ll * (1000 + ss) / 1000 } else { ll + ss - ll * ss / 1000 };
    let p = 2 * ll - q;
    let hue = |t: i32| -> i32 {
        let t = t.rem_euclid(240);
        if t < 40 {
            p + (q - p) * t / 40
        } else if t < 120 {
            q
        } else if t < 160 {
            p + (q - p) * (160 - t) / 40
        } else {
            p
        }
    };
    let h = h as i32;
    let c = |v: i32| ((v * 255 + 500) / 1000).clamp(0, 255) as u8;
    rgb(c(hue(h + 80)), c(hue(h)), c(hue(h - 80)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette() {
        assert_eq!(BASIC.len(), 48);
        assert_eq!(BASIC[0], rgb(0xFF, 0x80, 0x80));
        assert_eq!(BASIC[47], 0x00FF_FFFF);
        assert_eq!(BASIC[40], 0);
        // Every colour appears once.
        for (i, a) in BASIC.iter().enumerate() {
            for (j, b) in BASIC.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "{i} {j}");
            }
        }
    }

    #[test]
    fn primary_hues() {
        assert_eq!(to_hsl(rgb(255, 0, 0)), (0, 240, 120));
        assert_eq!(to_hsl(rgb(0, 255, 0)), (80, 240, 120));
        assert_eq!(to_hsl(rgb(0, 0, 255)), (160, 240, 120));
        assert_eq!(to_hsl(rgb(0, 0, 0)), (0, 0, 0));
        assert_eq!(to_hsl(rgb(255, 255, 255)), (0, 0, 240));
        let (_, s, l) = to_hsl(rgb(128, 128, 128));
        assert_eq!((s, l), (0, 120));
    }

    #[test]
    fn round_trips_stay_close() {
        for c in
            [rgb(255, 0, 0), rgb(0, 255, 0), rgb(0, 0, 255), rgb(255, 128, 0), rgb(10, 200, 90), rgb(128, 128, 128)]
        {
            let (h, s, l) = to_hsl(c);
            let back = from_hsl(h, s, l);
            for sh in [0, 8, 16] {
                let a = (c >> sh & 255) as i32;
                let b = (back >> sh & 255) as i32;
                assert!((a - b).abs() <= 6, "{c:#x} -> {h},{s},{l} -> {back:#x}");
            }
        }
    }
}
