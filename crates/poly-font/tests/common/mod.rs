//! Golden-image helpers for the rasterisation tests.
//!
//! An expected image is a plain-text PGM (grey) or PPM (LCD) file in
//! `tests/golden/`, written by this crate's own rasteriser and checked by
//! eye as a PNG before it was committed. A test renders again and
//! compares sample by sample within a tolerance (float rounding may differ
//! between machines; a real change of the rasteriser does not fit in it).
//!
//! - `POLY_FONT_BLESS=1` writes the expected files from what the tests
//!   render now (review the PNGs before committing them).
//! - `POLY_FONT_PNG_DIR=<dir>` also writes each rendering there as a PNG,
//!   and four times enlarged as `<name>-x4.png`, to look at.

#![allow(dead_code)]

use poly_font::{Bitmap, Image, RenderMode, png};
use std::path::{Path, PathBuf};

/// The largest difference allowed in any one sample, and on average.
const MAX_DIFF: u8 = 8;
const MAX_MEAN_DIFF: f64 = 0.5;

pub fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Where the bundled fonts are: `$POLY_FONT_DIR` (xtask sets it to its
/// cache), else this checkout's `target/fonts/dejavu-<version>`; `None`
/// (and the caller skips) until `cargo xtask fonts fetch` has run.
pub fn font_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("POLY_FONT_DIR") {
        Some(d) => PathBuf::from(d),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fonts/dejavu-2.37"),
    };
    dir.join("DejaVuSans.ttf").is_file().then_some(dir)
}

/// A glyph bitmap as an image of its own size.
pub fn glyph_image(b: &Bitmap) -> Image {
    let mut img = Image::new(b.width, b.height, b.mode);
    img.draw(b, -b.left, -b.top);
    img
}

fn to_text(img: &Image, what: &str) -> String {
    let lcd = img.mode == RenderMode::Lcd;
    let ch = if lcd { 3 } else { 1 };
    let mut s = format!(
        "{}\n# poly-font golden image: {what}\n{} {}\n255\n",
        if lcd { "P3" } else { "P2" },
        img.width,
        img.height
    );
    for row in img.data.chunks(img.width as usize * ch) {
        let line: Vec<String> = row.iter().map(|v| v.to_string()).collect();
        s.push_str(&line.join(" "));
        s.push('\n');
    }
    s
}

fn from_text(text: &str) -> Option<Image> {
    let mut tokens = text.lines().filter(|l| !l.starts_with('#')).flat_map(str::split_whitespace);
    let mode = match tokens.next()? {
        "P2" => RenderMode::Gray,
        "P3" => RenderMode::Lcd,
        _ => return None,
    };
    let w: u32 = tokens.next()?.parse().ok()?;
    let h: u32 = tokens.next()?.parse().ok()?;
    let _max = tokens.next()?;
    let data: Vec<u8> = tokens.map(|t| t.parse().ok()).collect::<Option<_>>()?;
    let img = Image { width: w, height: h, mode, data };
    (img.data.len() == w as usize * h as usize * mode.channels()).then_some(img)
}

fn write_pngs(name: &str, img: &Image) {
    let Some(dir) = std::env::var_os("POLY_FONT_PNG_DIR") else { return };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.png")), png::encode_image(img)).unwrap();
    let ch = img.mode.channels();
    let (w, h) = (img.width * 4, img.height * 4);
    let mut big = Image::new(w, h, img.mode);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let src = ((y / 4) * img.width as usize + x / 4) * ch;
            let dst = (y * w as usize + x) * ch;
            big.data[dst..dst + ch].copy_from_slice(&img.data[src..src + ch]);
        }
    }
    std::fs::write(dir.join(format!("{name}-x4.png")), png::encode_image(&big)).unwrap();
}

/// Compare `img` with `tests/golden/<name>.pgm` (or `.ppm`).
pub fn check(name: &str, what: &str, img: &Image) {
    write_pngs(name, img);
    let ext = if img.mode == RenderMode::Lcd { "ppm" } else { "pgm" };
    let path = golden_dir().join(format!("{name}.{ext}"));
    if std::env::var_os("POLY_FONT_BLESS").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, to_text(img, what)).unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (POLY_FONT_BLESS=1 writes it)", path.display()));
    let want = from_text(&text).unwrap_or_else(|| panic!("{}: not a golden image", path.display()));
    assert_eq!((img.width, img.height, img.mode), (want.width, want.height, want.mode), "{name}: size or mode differs");
    let mut worst = 0u8;
    let mut total = 0u64;
    for (a, b) in img.data.iter().zip(&want.data) {
        let d = a.abs_diff(*b);
        worst = worst.max(d);
        total += u64::from(d);
    }
    let mean = total as f64 / img.data.len().max(1) as f64;
    assert!(
        worst <= MAX_DIFF && mean <= MAX_MEAN_DIFF,
        "{name}: differs from {} (largest difference {worst}, mean {mean:.3})",
        path.display()
    );
}
