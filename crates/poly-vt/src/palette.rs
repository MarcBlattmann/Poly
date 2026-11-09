//! The 256-colour palette of `xterm-256color`.
//!
//! Colours 0-15 are the theme's (the renderer picks them; [`xterm_256`]
//! returns xterm's documented defaults for them as a fallback). 16-231 are
//! a 6x6x6 colour cube with the channel levels 0, 95, 135, 175, 215, 255;
//! 232-255 are 24 greys from 8 to 238 in steps of 10.

/// xterm's default values for the 16 ANSI colours.
const ANSI: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// Palette colour `i` as RGB.
pub fn xterm_256(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => ANSI[usize::from(i)],
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        _ => {
            let g = 8 + 10 * (i - 232);
            (g, g, g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_greys() {
        assert_eq!(xterm_256(16), (0, 0, 0));
        assert_eq!(xterm_256(196), (255, 0, 0));
        assert_eq!(xterm_256(21), (0, 0, 255));
        assert_eq!(xterm_256(67), (95, 135, 175));
        assert_eq!(xterm_256(231), (255, 255, 255));
        assert_eq!(xterm_256(232), (8, 8, 8));
        assert_eq!(xterm_256(255), (238, 238, 238));
        assert_eq!(xterm_256(1), (205, 0, 0));
    }
}
