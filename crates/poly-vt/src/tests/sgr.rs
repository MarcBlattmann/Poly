//! SGR: styles and the colour forms.

use super::*;
use crate::{Attrs, Color};

fn styled(seq: &str) -> Cell {
    let t = term(4, 1, &[seq, "x"].concat());
    cell(&t, 0, 0)
}

#[test]
fn attributes_on_and_off() {
    let c = styled("\x1b[1;2;3;4;5;7;8;9m");
    let all = Attrs::BOLD
        | Attrs::FAINT
        | Attrs::ITALIC
        | Attrs::UNDERLINE
        | Attrs::BLINK
        | Attrs::INVERSE
        | Attrs::HIDDEN
        | Attrs::STRIKE;
    assert_eq!(c.attrs, all);
    let c = styled("\x1b[1;2;3;4;5;7;8;9m\x1b[22;23;24;25;27;28;29m");
    assert!(c.attrs.is_empty());
    assert!(styled("\x1b[21m").attrs.contains(Attrs::UNDERLINE));
    assert!(styled("\x1b[1m\x1b[m").attrs.is_empty());
    assert!(styled("\x1b[1m\x1b[0m").attrs.is_empty());
    assert!(styled("\x1b[1;31m\x1b[;m").attrs.is_empty());
}

#[test]
fn underline_subparameters() {
    assert!(styled("\x1b[4:3m").attrs.contains(Attrs::UNDERLINE));
    assert!(styled("\x1b[4:1m").attrs.contains(Attrs::UNDERLINE));
    assert!(!styled("\x1b[4m\x1b[4:0m").attrs.contains(Attrs::UNDERLINE));
    // A sub-parameter does not start a new attribute: `4:3` is not 3.
    assert!(!styled("\x1b[4:3m").attrs.contains(Attrs::ITALIC));
}

#[test]
fn basic_colours() {
    let c = styled("\x1b[31;42m");
    assert_eq!((c.fg, c.bg), (Color::Indexed(1), Color::Indexed(2)));
    let c = styled("\x1b[97;104m");
    assert_eq!((c.fg, c.bg), (Color::Indexed(15), Color::Indexed(12)));
    let c = styled("\x1b[31;42m\x1b[39;49m");
    assert_eq!((c.fg, c.bg), (Color::Default, Color::Default));
}

#[test]
fn indexed_and_rgb_colours() {
    assert_eq!(styled("\x1b[38;5;196m").fg, Color::Indexed(196));
    assert_eq!(styled("\x1b[48;5;21m").bg, Color::Indexed(21));
    assert_eq!(styled("\x1b[38;2;10;20;30m").fg, Color::Rgb(10, 20, 30));
    assert_eq!(styled("\x1b[48;2;1;2;3m").bg, Color::Rgb(1, 2, 3));
    assert_eq!(styled("\x1b[38:5:9m").fg, Color::Indexed(9));
    assert_eq!(styled("\x1b[38:2::10:20:30m").fg, Color::Rgb(10, 20, 30));
    assert_eq!(styled("\x1b[38:2:0:10:20:30m").fg, Color::Rgb(10, 20, 30));
    assert_eq!(styled("\x1b[38:2:10:20:30m").fg, Color::Rgb(10, 20, 30));
    assert_eq!(styled("\x1b[48:2::7:8:9m").bg, Color::Rgb(7, 8, 9));
    // Out-of-range values saturate.
    assert_eq!(styled("\x1b[38;5;999m").fg, Color::Indexed(255));
    assert_eq!(styled("\x1b[38;2;300;0;0m").fg, Color::Rgb(255, 0, 0));
}

#[test]
fn colour_arguments_are_consumed() {
    // `38;5;1` must not also turn on bold (1); `48;2;4;4;4` not underline.
    let c = styled("\x1b[38;5;1;48;2;4;4;4;3m");
    assert_eq!(c.fg, Color::Indexed(1));
    assert_eq!(c.bg, Color::Rgb(4, 4, 4));
    assert_eq!(c.attrs, Attrs::ITALIC);
    // Truncated forms do nothing harmful.
    assert_eq!(styled("\x1b[38;5m").fg, Color::Default);
    assert_eq!(styled("\x1b[38;2;1;2m").fg, Color::Default);
    assert_eq!(styled("\x1b[38m").fg, Color::Default);
    // Underline colour is skipped with its arguments.
    let c = styled("\x1b[58;2;1;2;3;1m");
    assert_eq!(c.attrs, Attrs::BOLD);
    assert_eq!(c.fg, Color::Default);
    let c = styled("\x1b[58:5:3;9m");
    assert_eq!(c.attrs, Attrs::STRIKE);
}

#[test]
fn private_sgr_like_sequences_are_not_sgr() {
    // XTMODKEYS `CSI > 4 ; 1 m` must not underline.
    assert!(styled("\x1b[>4;1m").attrs.is_empty());
}
