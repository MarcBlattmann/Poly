//! Alpine Shell's wallpapers for Poly's desktop (ADR 0127): the two
//! pictures Alpine Shell bundles (`third_party/alpine-shell/wallpapers/`,
//! the owner's to give, THIRD_PARTY.md), decoded from JPEG here on the
//! host with the `jpeg-decoder` crate (pure Rust, so it works on Windows
//! and Linux hosts), scaled to cover 1600 × 900 by averaging, and stored
//! in `poly-shell`'s plain picture format (`PWAL`, `poly_shell::picture`)
//! so the native compositor needs no JPEG decoder. Kept in `target/` and
//! made again only when the JPEG changes.

use crate::Result;
use std::path::PathBuf;

/// (preset ID, source in `third_party/alpine-shell/wallpapers/`).
pub const WALLPAPERS: [(&str, &str); 2] =
    [("grainy-valley", "grainy-valley.jpeg"), ("quiet-moment", "quiet-moment-in-nature.jpg")];

/// The size stored: the pictures are drawn to cover the screen from it.
const SIZE: (u32, u32) = (1600, 900);

/// Where the desktop's images put them; the compositor reads them there.
pub const STORE_DIR: &str = "usr/share/poly/wallpapers";

/// Scale RGB pixels `w` × `h` to cover `tw` × `th`: crop the overflow
/// evenly, then average each target pixel's source area.
pub fn cover(rgb: &[u8], w: u32, h: u32, tw: u32, th: u32) -> Vec<u8> {
    let k = (f64::from(tw) / f64::from(w)).max(f64::from(th) / f64::from(h));
    let (cw, ch) = (f64::from(tw) / k, f64::from(th) / k);
    let (ox, oy) = ((f64::from(w) - cw) / 2.0, (f64::from(h) - ch) / 2.0);
    let mut out = Vec::with_capacity((tw * th * 3) as usize);
    for ty in 0..th {
        let y0 = (oy + f64::from(ty) / k) as u32;
        let y1 = ((oy + f64::from(ty + 1) / k).ceil() as u32).clamp(y0 + 1, h);
        for tx in 0..tw {
            let x0 = (ox + f64::from(tx) / k) as u32;
            let x1 = ((ox + f64::from(tx + 1) / k).ceil() as u32).clamp(x0 + 1, w);
            let mut sum = [0u64; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 3) as usize;
                    for c in 0..3 {
                        sum[c] += u64::from(rgb[i + c]);
                    }
                }
            }
            let n = u64::from((y1 - y0) * (x1 - x0));
            out.extend(sum.iter().map(|s| ((s + n / 2) / n) as u8));
        }
    }
    out
}

/// Decode a JPEG to (width, height, RGB).
fn decode(jpeg: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut d = jpeg_decoder::Decoder::new(jpeg);
    let pixels = d.decode().map_err(|e| format!("JPEG: {e}"))?;
    let info = d.info().ok_or("JPEG: no image information")?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels,
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&g| [g, g, g]).collect(),
        f => return Err(format!("JPEG: pixel format {f:?} is not supported")),
    };
    Ok((w, h, rgb))
}

/// The pictures for a desktop image: (path on Poly, contents).
pub fn tree() -> Result<Vec<(String, Vec<u8>)>> {
    let root = crate::root();
    let cache = root.join("target").join("wallpapers");
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let mut out = Vec::new();
    for (id, file) in WALLPAPERS {
        let src = root.join("third_party/alpine-shell/wallpapers").join(file);
        let jpeg = std::fs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?;
        let stamp = format!("{:08x}-{}", crate::unzip::crc32(&jpeg), jpeg.len());
        let cached: PathBuf = cache.join(format!("{id}-{stamp}.pwal"));
        let data = match std::fs::read(&cached) {
            Ok(d) => d,
            Err(_) => {
                let (w, h, rgb) = decode(&jpeg)?;
                let scaled = cover(&rgb, w, h, SIZE.0, SIZE.1);
                let d = poly_shell::picture::encode(SIZE.0, SIZE.1, &scaled);
                std::fs::write(&cached, &d).map_err(|e| format!("{}: {e}", cached.display()))?;
                d
            }
        };
        out.push((format!("{STORE_DIR}/{id}.pwal"), data));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crops_evenly_and_averages() {
        // 4 × 2: left half black, right half white, to 2 × 2: the middle
        // two columns are kept, one of each.
        let mut rgb = Vec::new();
        for _ in 0..2 {
            rgb.extend([0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255, 255]);
        }
        let out = cover(&rgb, 4, 2, 2, 2);
        assert_eq!(out, [0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255]);
        // Halving averages 2 × 2 blocks.
        let out = cover(&[0, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 0], 2, 2, 1, 1);
        assert_eq!(out, [128, 128, 128]);
    }

    #[test]
    fn the_bundled_wallpapers_decode() {
        let pictures = tree().unwrap();
        assert_eq!(pictures.len(), 2);
        for (path, data) in pictures {
            let p = poly_shell::picture::parse(&data).unwrap_or_else(|| panic!("{path}"));
            assert_eq!((p.width, p.height), SIZE);
        }
    }
}
