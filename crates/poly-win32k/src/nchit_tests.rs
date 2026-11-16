//! The non-client hit test against the recorded results of `WM_NCHITTEST`'s visible tests
//! (`hit_corners_cap`, `hit_corners_ovl_thick_nocap`, `hit_corners_sizes`).

extern crate std;

use super::*;
use std::format;
use std::string::String;
use std::vec::Vec;

fn name(code: i32) -> &'static str {
    match code {
        1 => "HTCLIENT",
        2 => "HTCAPTION",
        3 => "HTSYSMENU",
        8 => "HTMINBUTTON",
        9 => "HTMAXBUTTON",
        10 => "HTLEFT",
        11 => "HTRIGHT",
        12 => "HTTOP",
        13 => "HTTOPLEFT",
        14 => "HTTOPRIGHT",
        15 => "HTBOTTOM",
        16 => "HTBOTTOMLEFT",
        17 => "HTBOTTOMRIGHT",
        18 => "HTBORDER",
        20 => "HTCLOSE",
        _ => "?",
    }
}

/// A window at (300,200) of this size, its frame 8 wide and its caption 23 high (a client area at
/// 8,31) or 7 wide without a caption.
struct W {
    style: u32,
    ex: u32,
    w: Rect,
    c: Rect,
}

fn win(style: u32, ex: u32, width: i32, height: i32) -> W {
    let captioned = style & WS_CAPTION == WS_CAPTION;
    let (f, cap) = if captioned { (8, 23) } else { (7, 0) };
    let w = Rect::new(300, 200, 300 + width, 200 + height);
    let c = Rect::new(w.left + f, w.top + f + cap, w.right - f, w.bottom - f);
    W { style, ex, w, c }
}

fn code(w: &W, x: i32, y: i32) -> i32 {
    if w.c.contains(x, y) { 1 } else { nc_hit(w.style, w.ex, w.w, w.c, x, y) }
}

/// The runs `a..b=name` along a row (y offset from the top) or a column of the window.
fn runs(w: &W, row: Option<i32>, col: Option<i32>) -> String {
    let n = if row.is_some() { w.w.width() } else { w.w.height() };
    let at = |i: i32| match (row, col) {
        (Some(y), _) => code(w, w.w.left + i, w.w.top + y),
        (_, Some(x)) => code(w, w.w.left + x, w.w.top + i),
        _ => unreachable!(),
    };
    let mut out: Vec<String> = Vec::new();
    let mut start = 0;
    for i in 1..=n {
        if i == n || at(i) != at(start) {
            out.push(format!("{start}..{}={}", i - 1, name(at(start))));
            start = i;
        }
    }
    out.join(" ")
}

const OVERLAPPED: u32 = 0x00CF_0000; // WS_OVERLAPPEDWINDOW
const POPUP_THICK: u32 = 0x8004_0000;

#[test]
fn captioned_rows_per_hit_corners_cap() {
    let w = win(OVERLAPPED, 0, 400, 300);
    let row = |y| runs(&w, Some(y), None);
    assert_eq!(row(0), "0..18=HTTOPLEFT 19..381=HTTOP 382..399=HTTOPRIGHT");
    assert_eq!(row(7), "0..18=HTTOPLEFT 19..381=HTTOP 382..399=HTTOPRIGHT");
    assert_eq!(
        row(8),
        "0..7=HTTOPLEFT 8..26=HTSYSMENU 27..288=HTTOP 289..321=HTMINBUTTON 322..323=HTTOP 324..356=HTMAXBUTTON 357..391=HTCLOSE 392..399=HTTOPRIGHT"
    );
    for y in [9, 10, 12, 16, 24] {
        assert_eq!(
            row(y),
            "0..7=HTLEFT 8..26=HTSYSMENU 27..288=HTCAPTION 289..321=HTMINBUTTON 322..323=HTCAPTION 324..356=HTMAXBUTTON 357..391=HTCLOSE 392..399=HTRIGHT",
            "row {y}"
        );
    }
    assert_eq!(row(30), "0..18=HTLEFT 19..381=HTCAPTION 382..399=HTRIGHT");
    for y in [299, 298, 297, 296, 295, 294, 293] {
        assert_eq!(row(y), "0..17=HTBOTTOMLEFT 18..382=HTBOTTOM 383..399=HTBOTTOMRIGHT", "row {y}");
    }
    assert_eq!(row(292), "0..8=HTLEFT 9..17=HTBOTTOMLEFT 18..382=HTBOTTOM 383..391=HTBOTTOMRIGHT 392..399=HTRIGHT");
    assert_eq!(row(291), "0..7=HTLEFT 8..391=HTCLIENT 392..399=HTRIGHT");
}

#[test]
fn captioned_columns_per_hit_corners_cap() {
    let w = win(OVERLAPPED, 0, 400, 300);
    let col = |x| runs(&w, None, Some(x));
    for x in 0..8 {
        assert_eq!(col(x), "0..8=HTTOPLEFT 9..292=HTLEFT 293..299=HTBOTTOMLEFT", "column {x}");
        assert_eq!(col(399 - x), "0..8=HTTOPRIGHT 9..292=HTRIGHT 293..299=HTBOTTOMRIGHT", "column {}", 399 - x);
    }
    assert_eq!(
        col(8),
        "0..7=HTTOPLEFT 8..29=HTSYSMENU 30..30=HTLEFT 31..291=HTCLIENT 292..292=HTLEFT 293..299=HTBOTTOMLEFT"
    );
    assert_eq!(col(391), "0..7=HTTOPRIGHT 8..29=HTCLOSE 30..30=HTRIGHT 31..291=HTCLIENT 292..299=HTBOTTOMRIGHT");
}

#[test]
fn caption_without_buttons_per_hit_corners_ovl_thick_nocap() {
    // WS_OVERLAPPED | WS_THICKFRAME | WS_CAPTION: no system menu, so no buttons.
    let w = win(0x00C4_0000, 0, 400, 300);
    let row = |y| runs(&w, Some(y), None);
    assert_eq!(row(8), "0..18=HTTOPLEFT 19..381=HTTOP 382..399=HTTOPRIGHT");
    assert_eq!(row(9), "0..18=HTLEFT 19..381=HTCAPTION 382..399=HTRIGHT");
    assert_eq!(row(30), "0..18=HTLEFT 19..381=HTCAPTION 382..399=HTRIGHT");
}

#[test]
fn corners_do_not_depend_on_the_size() {
    for (width, height) in [(200, 100), (1000, 600), (160, 160), (300, 500)] {
        let w = win(OVERLAPPED, 0, width, height);
        assert_eq!(
            runs(&w, Some(2), None),
            format!("0..18=HTTOPLEFT 19..{}=HTTOP {}..{}=HTTOPRIGHT", width - 19, width - 18, width - 1)
        );
        assert_eq!(
            runs(&w, None, Some(2)),
            format!("0..8=HTTOPLEFT 9..{}=HTLEFT {}..{}=HTBOTTOMLEFT", height - 8, height - 7, height - 1)
        );
    }
}

#[test]
fn tool_window_corners() {
    let w = win(OVERLAPPED, WS_EX_TOOLWINDOW, 400, 300);
    assert_eq!(runs(&w, Some(2), None), "0..14=HTTOPLEFT 15..385=HTTOP 386..399=HTTOPRIGHT");
    assert_eq!(runs(&w, Some(296), None), "0..17=HTBOTTOMLEFT 18..382=HTBOTTOM 383..399=HTBOTTOMRIGHT");
    assert_eq!(runs(&w, None, Some(2)), "0..16=HTTOPLEFT 17..292=HTLEFT 293..299=HTBOTTOMLEFT");
    assert_eq!(runs(&w, None, Some(397)), "0..16=HTTOPRIGHT 17..292=HTRIGHT 293..299=HTBOTTOMRIGHT");
}

#[test]
fn captionless_frame_corners_are_the_frame_plus_the_caption_size() {
    let w = win(POPUP_THICK, 0, 400, 300);
    assert_eq!(runs(&w, Some(0), None), "0..28=HTTOPLEFT 29..370=HTTOP 371..399=HTTOPRIGHT");
    assert_eq!(runs(&w, Some(7), None), "0..6=HTTOPLEFT 7..392=HTCLIENT 393..399=HTTOPRIGHT");
    assert_eq!(runs(&w, Some(30), None), "0..6=HTLEFT 7..392=HTCLIENT 393..399=HTRIGHT");
    assert_eq!(runs(&w, Some(299), None), "0..28=HTBOTTOMLEFT 29..370=HTBOTTOM 371..399=HTBOTTOMRIGHT");
    assert_eq!(runs(&w, None, Some(0)), "0..28=HTTOPLEFT 29..270=HTLEFT 271..299=HTBOTTOMLEFT");
    assert_eq!(runs(&w, None, Some(7)), "0..6=HTTOPLEFT 7..292=HTCLIENT 293..299=HTBOTTOMLEFT");
    assert_eq!(runs(&w, None, Some(399)), "0..28=HTTOPRIGHT 29..270=HTRIGHT 271..299=HTBOTTOMRIGHT");
}

#[test]
fn without_a_sizing_frame_there_are_no_corners() {
    let w = win(0x00C8_0000, 0, 400, 300); // WS_CAPTION | WS_SYSMENU
    assert_eq!(code(&w, 300, 200), HTCAPTION);
    assert_eq!(code(&w, 350, 215), HTCAPTION);
    assert_eq!(code(&w, 350, 497), HTBORDER);
}
