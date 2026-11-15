//! The rules of `CreateEllipticRgn.yaml` and `CreateRoundRectRgn.yaml`, from
//! the sheets' own examples and the visible tests' recorded rows.

extern crate std;
use super::*;
use crate::region::{COMPLEXREGION, NULLREGION, Rect, Region, SIMPLEREGION};
use std::string::String;

/// The rows as the sheets write them: `top+rows:left-right` for every run
/// of consecutive rows with the same run.
fn text(rows: &[(i64, i64, i64)]) -> String {
    let mut out: Vec<(i64, i64, i64, i64)> = Vec::new();
    for &(y, l, r) in rows {
        match out.last_mut() {
            Some(last) if last.0 + last.1 == y && last.2 == l && last.3 == r => last.1 += 1,
            _ => out.push((y, 1, l, r)),
        }
    }
    out.iter().map(|(y, n, l, r)| std::format!("{y}+{n}:{l}-{r}")).collect::<Vec<_>>().join(" ")
}

fn ell(w: i64, h: i64) -> String {
    text(&ellipse_rows(w, h))
}

fn rr(w: i64, h: i64, cw: i64, ch: i64) -> String {
    text(&round_rect_rows(w, h, cw, ch))
}

fn pts(s: &str) -> Vec<Pt> {
    s.split_whitespace()
        .map(|p| {
            let (x, y) = p.split_once(',').unwrap();
            (x.parse().unwrap(), y.parse().unwrap())
        })
        .collect()
}

/// `step4_flatten`, `worked_example` and `examples`: the polygons.
#[test]
fn ellipse_polygons_follow_the_steps() {
    assert_eq!(polygon(4, 4, radius(4), radius(4)), pts("44,24 24,4 4,24 24,44"));
    assert_eq!(polygon(5, 5, radius(5), radius(5)), pts("60,32 52,12 32,4 12,12 4,32 12,52 32,60 52,52"));
    assert_eq!(
        polygon(22, 6, radius(22), radius(6)),
        pts("332,40 319,26 284,15 232,7 168,4 104,7 52,15 17,26 4,40 17,54 52,65 104,73 168,76 232,73 284,65 319,54")
    );
    assert_eq!(
        polygon(50, 8, radius(50), radius(8)),
        pts("780,56 772,46 750,36 667,20 543,8 392,4 241,8 117,20 34,36 12,46 4,56 12,66 34,76 117,93 241,104 \
             392,108 543,104 667,93 750,76 772,66")
    );
    // 22x6 has four pieces per arc, a circle of 1996 pixels 32.
    assert_eq!(polygon(22, 6, radius(22), radius(6)).len(), 16);
    assert_eq!(polygon(1996, 1996, radius(1996), radius(1996)).len(), 4 * 32);
}

/// `CreateRoundRectRgn.yaml` `worked_example`: 26x9 with corner 7x4.
#[test]
fn round_rectangle_polygon_follows_the_worked_example() {
    assert_eq!((radius(26), radius(9)), (196, 60));
    assert_eq!((corner(7, 26), corner(4, 9)), (53, 27));
    assert_eq!(
        polygon(26, 9, 53, 27),
        pts("396,31 381,12 343,4 57,4 19,12 4,31 4,97 19,116 57,124 343,124 381,116 396,97")
    );
    assert_eq!(rr(26, 9, 7, 4), "0+1:2-23 1+1:1-24 2+4:0-25 6+1:1-24 7+1:2-23");
}

/// `CreateEllipticRgn.yaml` `worked_example` and `examples`.
#[test]
fn ellipse_rows_of_the_sheet_examples() {
    assert_eq!(ell(22, 6), "0+1:6-15 1+1:1-20 2+1:0-21 3+1:1-20 4+1:6-15");
    assert_eq!(ell(24, 10), "0+1:8-15 1+1:4-19 2+1:2-21 3+1:1-22 4+1:0-23 5+1:1-22 6+1:2-21 7+1:4-19 8+1:8-15");
    assert_eq!(
        ell(22, 14),
        "0+1:8-12 1+1:5-16 2+1:3-18 3+1:2-19 4+2:1-20 6+1:0-21 7+2:1-20 9+1:2-19 10+1:3-18 11+1:5-16 12+1:8-12"
    );
    assert_eq!(
        ell(23, 16),
        "0+1:9-13 1+1:5-17 2+1:3-19 3+1:2-20 4+3:1-21 7+1:0-22 8+3:1-21 11+1:2-20 12+1:3-19 13+1:5-17 14+1:9-13"
    );
    assert_eq!(ell(3, 22), "3+15:0-2");
    assert_eq!(ell(50, 8), "0+1:15-34 1+1:6-43 2+1:2-47 3+1:0-49 4+1:2-47 5+1:6-43 6+1:15-34");
    assert_eq!(
        ell(31, 21),
        "0+1:13-17 1+1:8-22 2+1:6-24 3+1:4-26 4+1:3-27 5+1:2-28 6+3:1-29 9+2:0-30 11+3:1-29 14+1:2-28 \
         15+1:3-27 16+1:4-26 17+1:6-24 18+1:8-22 19+1:13-17"
    );
    assert_eq!(ell(3, 3), "");
    assert_eq!(ell(4, 4), "0+1:1-2 1+1:0-3 2+1:1-2");
}

/// `degenerate`: no special case, every size goes through the steps.
#[test]
fn zero_and_one_wide_ellipses() {
    assert_eq!(ell(1, 1), "");
    assert_eq!(ell(1, 40), "");
    assert_eq!(ell(40, 1), "");
    assert_eq!(ell(9, 0), "-1+1:0-8");
    assert_eq!(ell(0, 27), "0+26:-1-0");
    assert_eq!(ell(0, 0), "-1+1:-1-0");
    assert_eq!(ell(2, 9), "0+8:0-1");
    assert_eq!(ell(9, 2), "0+1:0-8");
}

/// `CreateRoundRectRgn.yaml` `examples`, `zero_corner`, `degenerate`.
#[test]
fn round_rectangle_rows_of_the_sheet_examples() {
    assert_eq!(rr(50, 30, 14, 9), "0+1:6-43 1+1:2-47 2+2:1-48 4+21:0-49 25+2:1-48 27+1:2-47 28+1:6-43");
    assert_eq!(rr(40, 20, 9, 9), "0+1:4-35 1+1:1-37 2+2:1-38 4+11:0-39 15+2:1-38 17+1:1-37 18+1:4-35");
    assert_eq!(rr(40, 20, 10, 10), "0+1:4-35 1+1:2-37 2+2:1-38 4+11:0-39 15+2:1-38 17+1:2-37 18+1:4-35");
    assert_eq!(
        rr(45, 45, 16, 16),
        "0+1:7-37 1+1:4-40 2+1:3-41 3+1:2-42 4+3:1-43 7+30:0-44 37+3:1-43 40+1:2-42 41+1:2-41 42+1:4-40 43+1:7-37"
    );
    assert_eq!(corner(16, 45), 124);
    assert_eq!(
        rr(20, 20, -6, 30),
        "0+1:3-16 1+1:2-17 2+5:1-18 7+1:0-18 8+3:0-19 11+1:0-18 12+5:1-18 17+1:2-17 18+1:3-16"
    );
    assert_eq!(rr(36, 36, 100, 100), ell(36, 36));
    assert_eq!(
        rr(36, 36, 100, 100),
        "0+1:16-19 1+1:11-24 2+1:9-26 3+1:8-27 4+1:7-28 5+2:5-30 7+1:4-31 8+2:3-32 10+1:2-33 11+5:1-34 \
         16+3:0-35 19+5:1-34 24+1:2-33 25+2:3-32 27+1:4-31 28+2:5-30 30+1:7-28 31+1:8-27 32+1:9-26 33+1:11-24 34+1:16-19"
    );
    // A corner of 19 on 20 is not the ellipse; 20, 21, 200 and -20 are.
    assert_ne!(rr(20, 20, 19, 19), ell(20, 20));
    for c in [20, 21, 200, -20, i64::from(i32::MIN), i64::from(i32::MAX)] {
        assert_eq!(rr(20, 20, c, c), ell(20, 20), "{c}");
    }
    // Corner 0 is the plain rectangle 0..W-2 x 0..H-2, whatever the other corner.
    for ch in [0, 6, 17, 40] {
        assert_eq!(rr(33, 17, 0, ch), "0+16:0-32", "{ch}");
    }
    // A corner of 1 is not always a plain rectangle.
    assert_eq!(rr(60, 20, 1, 1), "0+19:0-59");
    assert_eq!(rr(80, 51, 1, 1), "0+1:0-78 1+48:0-79 49+1:0-78");
    // A side of 0 or 1: as for the ellipse.
    assert_eq!(rr(1, 30, 5, 5), "");
    assert_eq!(rr(30, 1, 5, 5), "");
    assert_eq!(rr(9, 0, 3, 3), "-1+1:0-8");
    assert_eq!(rr(0, 27, 3, 3), "0+26:-1-0");
}

/// Every ellipse and round rectangle of the visible tests' `windows.json`
/// (recorded on Windows 11 build 26200).
#[test]
fn rows_of_every_visible_test_region() {
    let mut checked = 0;
    for line in include_str!("../tests/rgn_rows.txt").lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split(' ').collect();
        let n = |i: usize| f[i].parse::<i64>().unwrap();
        let (got, rest) = match f[0] {
            "e" => (ell(n(1), n(2)), &f[3..]),
            _ => (rr(n(1), n(2), n(3), n(4)), &f[5..]),
        };
        assert_eq!(got, rest.join(" "), "{line}");
        checked += 1;
    }
    assert!(checked > 600, "{checked}");
}

/// The same table through `Region`: placement, reversed corners, kinds.
#[test]
fn regions_are_moved_to_the_corner_of_the_rectangle() {
    let e = Region::ellipse(0, 0, 22, 6).unwrap();
    assert_eq!(e.kind(), COMPLEXREGION);
    assert_eq!(
        e.rects(),
        &[
            Rect::new(6, 0, 15, 1),
            Rect::new(1, 1, 20, 2),
            Rect::new(0, 2, 21, 3),
            Rect::new(1, 3, 20, 4),
            Rect::new(6, 4, 15, 5)
        ]
    );
    assert_eq!(e.bounds(), Rect::new(0, 0, 21, 5));
    // Only the size counts; reversed corners give the same region moved.
    for (l, t, r, b) in [(5, 7, 27, 13), (27, 13, 5, 7), (5, 13, 27, 7), (-40, -9, -18, -3)] {
        let (x, y) = (l.min(r), t.min(b));
        assert_eq!(Region::ellipse(l, t, r, b).unwrap(), e.offset_by(x, y), "{l},{t},{r},{b}");
    }
    assert_eq!(Region::ellipse(0, 0, 60, 40).unwrap().bounds(), Rect::new(0, 0, 59, 39));
    assert_eq!(Region::ellipse(0, 0, 3, 3).unwrap().kind(), NULLREGION);
    assert_eq!(Region::ellipse(0, 0, 1, 40).unwrap().bounds(), Rect::default());
    assert_eq!(Region::ellipse(0, 0, 2, 9).unwrap().kind(), SIMPLEREGION);
    // 61,39,0,0 and 0,39,61,0 (crrr_placed): the same round rectangle.
    let p = Region::round_rect(61, 39, 0, 0, 12, 9).unwrap();
    assert_eq!(p.bounds(), Rect::new(0, 0, 60, 38));
    assert_eq!(p, Region::round_rect(0, 39, 61, 0, -12, 9).unwrap());
    // Zero width or height: one row at y = -1, one column at x = -1.
    assert_eq!(Region::ellipse(0, 0, 9, 0).unwrap().bounds(), Rect::new(0, -1, 8, 0));
    assert_eq!(Region::ellipse(-7, -7, -7, 20).unwrap().bounds(), Rect::new(-8, -7, -7, 19));
    assert_eq!(Region::ellipse(0, 0, 0, 0).unwrap().rects(), &[Rect::new(-1, -1, 0, 0)]);
    // 20x20 corner 19 is not the ellipse; 20 is.
    assert_ne!(Region::round_rect(0, 0, 20, 20, 19, 19), Region::ellipse(0, 0, 20, 20));
    assert_eq!(Region::round_rect(0, 0, 20, 20, 20, 20), Region::ellipse(0, 0, 20, 20));
    assert_eq!(Region::round_rect(5, 5, 20, 15, 0, 0).unwrap(), Region::from_rect(Rect::new(5, 5, 19, 14)));
    assert_eq!(Region::round_rect(7, -9, 30, 30, 0, 7).unwrap().rects(), &[Rect::new(7, -9, 29, 29)]);
}

/// `errors.range`: -2^27 .. 2^27-1 are accepted, anything else is refused.
#[test]
fn coordinates_outside_the_limits_are_refused() {
    let (lo, hi) = (-(1 << 27), (1 << 27) - 1);
    for bad in [hi + 1, lo - 1, i32::MAX, i32::MIN] {
        assert!(Region::ellipse(0, 0, bad, 10).is_none(), "{bad}");
        assert!(Region::ellipse(bad, 0, 10, 10).is_none(), "{bad}");
        assert!(Region::ellipse(0, bad, 10, 10).is_none(), "{bad}");
        assert!(Region::ellipse(0, 0, 10, bad).is_none(), "{bad}");
        assert!(Region::round_rect(0, 0, bad, 10, 5, 5).is_none(), "{bad}");
    }
    assert!(Region::ellipse(lo, lo, lo + 40, lo + 30).is_some());
    assert!(Region::ellipse(hi - 40, hi - 30, hi, hi).is_some());
    // The corner sizes have no limit.
    for c in [i32::MAX, i32::MIN, -i32::MAX] {
        assert!(Region::round_rect(0, 0, 30, 20, c, c).is_some());
    }
    assert_eq!(Region::round_rect(0, 0, 30, 20, i32::MIN, 5), Region::round_rect(0, 0, 30, 20, i32::MAX, 5));
}

/// A big ellipse is one run per row, whatever its position.
#[test]
fn a_big_ellipse_is_made_of_one_run_per_row() {
    let e = Region::ellipse(0, 0, 1000, 800).unwrap();
    assert_eq!(e.bounds(), Rect::new(0, 0, 999, 799));
    assert!(e.contains_point(500, 400) && !e.contains_point(0, 0) && !e.contains_point(998, 798));
    let rows = ellipse_rows(1000, 800);
    assert_eq!(rows.len(), 799);
    assert!(rows.windows(2).all(|w| w[1].0 == w[0].0 + 1));
}
