//! The pointer's pictures (ADR 0252): the **Bibata** cursor theme, variant Modern Ice (white,
//! rounded), the theme the owner's Alpine Shell uses (its `CURSOR_VARIANT`).
//!
//! Bibata is by Abdulkaiz Khatri (ful1e5) under the GNU General Public License 3.0
//! (`third_party/bibata-cursors/`). Like the fonts it is **not committed**: `cargo xtask cursors
//! fetch` downloads the X11 package of the pinned upstream release over HTTPS into the main
//! checkout's `target/cursors/` (shared by its worktrees), checks it against the pinned size and
//! SHA-256, and unpacks only the twelve cursors Poly draws (one per look of ADR 0234). Each is an
//! XCursor file; Poly keeps only its images of nominal sizes 24, 32 and 48 (100 %, 125 to 150 %,
//! 175 to 200 %) and drops the others, nothing else is changed (a modified copy, as the GPL allows,
//! said so in the README beside the files).
//!
//! The desktop's images carry them as data files in `/usr/share/icons/Bibata-Modern-Ice/cursors/`,
//! with the licence, the theme's `index.theme` and a README naming the source beside them; the
//! compositor reads them at start (`poly-compositor`'s `desktop::cursor_theme`). Nothing of them is
//! compiled into Poly: without them the compositor draws its own pointer.

use crate::Result;
use std::path::{Path, PathBuf};

/// The upstream release.
pub const VERSION: &str = "2.0.7";
/// The tag's commit in the upstream repository (the theme's source: `svg/`, `configs/`,
/// `build.sh`, `render.json`).
pub const COMMIT: &str = "35ccfe209a808e40d6c2ca60a46cbe4faf68b690";
const URL: &str = "https://github.com/ful1e5/Bibata_Cursor/releases/download/v2.0.7/Bibata-Modern-Ice.tar.xz";
const SIZE: u64 = 1_701_468;
const SHA256: &str = "a68cae60c4dc706350e194ebc91c5fe48bc7bc9d59e119555834a2a7ee5078ef";
/// The theme's directory name, in the archive and on the images.
pub const THEME: &str = "Bibata-Modern-Ice";
/// Where the images put the theme.
pub const DIR: &str = "usr/share/icons/Bibata-Modern-Ice";

/// The cursors Poly draws: the files of `poly_wayland::compose::CursorTheme::file_name`.
pub const FILES: [&str; 12] = [
    "left_ptr",
    "left_ptr_watch",
    "hand2",
    "xterm",
    "wait",
    "crosshair",
    "move",
    "circle",
    "sb_h_double_arrow",
    "sb_v_double_arrow",
    "bd_double_arrow",
    "fd_double_arrow",
];
/// The nominal sizes kept.
const SIZES: [u32; 3] = [24, 32, 48];

/// `<main checkout>/target/cursors/bibata-<version>/`.
pub fn cache_dir() -> PathBuf {
    let ntdll = crate::ntdll::cache_dir(&crate::root());
    ntdll
        .parent()
        .map_or_else(|| crate::root().join("target"), Path::to_path_buf)
        .join("cursors")
        .join(format!("bibata-{VERSION}"))
}

/// The cache, if every cursor is in it.
pub fn cached() -> Option<PathBuf> {
    let dir = cache_dir();
    (FILES.iter().all(|f| dir.join(f).is_file()) && dir.join("index.theme").is_file()).then_some(dir)
}

/// An XCursor file with only the images of the nominal sizes in `keep` (and nothing else: the
/// comments go too). `None` when it is not an XCursor file or keeps no image.
pub fn trim(b: &[u8], keep: &[u32]) -> Option<Vec<u8>> {
    let u32_at = |at: usize| -> Option<u32> { Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?)) };
    if b.get(..4)? != b"Xcur" {
        return None;
    }
    let (header, version, count) = (u32_at(4)? as usize, u32_at(8)?, u32_at(12)? as usize);
    // (subtype, chunk bytes) of the images kept, in the file's order.
    let mut chunks: Vec<(u32, &[u8])> = Vec::new();
    for i in 0..count {
        let e = header + i * 12;
        let (kind, sub, pos) = (u32_at(e)?, u32_at(e + 4)?, u32_at(e + 8)? as usize);
        if kind != 0xfffd_0002 || !keep.contains(&sub) {
            continue;
        }
        let (len, w, h) = (u32_at(pos)? as usize, u32_at(pos + 16)? as usize, u32_at(pos + 20)? as usize);
        chunks.push((sub, b.get(pos..pos + len + 4 * w * h)?));
    }
    if chunks.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"Xcur");
    for v in [16u32, version, chunks.len() as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut pos = 16 + 12 * chunks.len();
    for (sub, c) in &chunks {
        for v in [0xfffd_0002u32, *sub, pos as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        pos += c.len();
    }
    for (_, c) in &chunks {
        out.extend_from_slice(c);
    }
    Some(out)
}

/// `cursors fetch`: download, check and unpack the pinned theme (see the module documentation).
pub fn fetch() -> Result<PathBuf> {
    if let Some(dir) = cached() {
        eprintln!("==> {} holds Bibata {VERSION} ({THEME})", dir.display());
        return Ok(dir);
    }
    let dir = cache_dir();
    let parent = dir.parent().ok_or("no cache parent")?.to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let archive = parent.join(format!("{THEME}-{VERSION}.tar.xz"));
    if !std::fs::metadata(&archive).is_ok_and(|m| m.len() == SIZE) {
        crate::fonts::download(URL, &archive)?;
    }
    let data = std::fs::read(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    if data.len() as u64 != SIZE || crate::sha256::hex(&data) != SHA256 {
        let _ = std::fs::remove_file(&archive);
        return Err(format!(
            "{URL}: not the pinned archive (size {}, expected {SIZE} and SHA-256 {SHA256})",
            data.len()
        ));
    }
    let mut tar = Vec::new();
    lzma_rs::xz_decompress(&mut std::io::BufReader::new(&data[..]), &mut tar).map_err(|e| format!("xz: {e:?}"))?;
    let entries = crate::chromium::tar_entries(&tar)?;
    // A fresh, empty directory, filled and then moved into place.
    let fresh = parent.join(format!(".bibata-{VERSION}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&fresh);
    std::fs::create_dir_all(&fresh).map_err(|e| format!("{}: {e}", fresh.display()))?;
    let file = |name: &str| -> Result<Vec<u8>> {
        let path = format!("{THEME}/{name}");
        entries
            .iter()
            .find(|e| e.path == path)
            .and_then(|e| match &e.kind {
                crate::chromium::TarKind::File(d) => Some(d.clone()),
                _ => None,
            })
            .ok_or_else(|| format!("{URL}: no file {path}"))
    };
    for f in FILES {
        let trimmed =
            trim(&file(&format!("cursors/{f}"))?, &SIZES).ok_or_else(|| format!("{f}: no image of 24, 32 or 48"))?;
        std::fs::write(fresh.join(f), trimmed).map_err(|e| e.to_string())?;
    }
    std::fs::write(fresh.join("index.theme"), file("index.theme")?).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&fresh, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&archive);
    eprintln!(
        "==> Bibata {VERSION} ({THEME}, source at upstream commit {COMMIT}): {} cursors in {}",
        FILES.len(),
        dir.display()
    );
    Ok(dir)
}

/// The theme's files for a desktop image, `(path, contents)`: the cursors, `index.theme`, the
/// licence and the README beside them; empty when the theme is not fetched, or with
/// `POLY_NO_CURSOR_THEME` set (Poly's own pointer, to compare).
pub fn tree() -> Vec<(String, Vec<u8>)> {
    if std::env::var_os("POLY_NO_CURSOR_THEME").is_some() {
        return Vec::new();
    }
    let Some(dir) = cached() else { return Vec::new() };
    let third = crate::root().join("third_party/bibata-cursors");
    let mut out = Vec::new();
    for f in FILES {
        if let Ok(d) = std::fs::read(dir.join(f)) {
            out.push((format!("{DIR}/cursors/{f}"), d));
        }
    }
    for (from, to) in [
        (dir.join("index.theme"), "index.theme"),
        (third.join("LICENSE"), "LICENSE"),
        (third.join("README.md"), "README.md"),
    ] {
        if let Ok(d) = std::fs::read(&from) {
            out.push((format!("{DIR}/{to}"), d));
        }
    }
    out
}

/// `cargo xtask cursors fetch`.
pub fn command(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("fetch") => fetch().map(|_| ()),
        _ => Err("usage: cargo xtask cursors fetch".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(sizes: &[u32]) -> Vec<u8> {
        let mut b = b"Xcur".to_vec();
        for v in [16u32, 0x1_0000, sizes.len() as u32 + 1] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // A comment entry first, then one 2x2 image per size.
        let mut pos = 16 + 12 * (sizes.len() as u32 + 1);
        for v in [0xfffe_0001u32, 1, pos] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        pos += 20;
        for s in sizes {
            for v in [0xfffd_0002u32, *s, pos] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            pos += 36 + 16;
        }
        b.extend_from_slice(&[0u8; 20]);
        for s in sizes {
            for v in [36u32, 0xfffd_0002, *s, 1, 2, 2, 1, 1, 40] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            for _ in 0..4 {
                b.extend_from_slice(&s.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn trimming_keeps_the_sizes_asked_for() {
        let t = trim(&file(&[16, 24, 32, 48, 96]), &SIZES).unwrap();
        assert_eq!(&t[..4], b"Xcur");
        assert_eq!(u32::from_le_bytes(t[12..16].try_into().unwrap()), 3);
        assert_eq!(t.len(), 16 + 3 * 12 + 3 * (36 + 16));
        let subs: Vec<u32> =
            (0..3).map(|i| u32::from_le_bytes(t[16 + 12 * i + 4..][..4].try_into().unwrap())).collect();
        assert_eq!(subs, [24, 32, 48]);
        // Each entry points at its chunk, whose pixels are its size.
        let pos = u32::from_le_bytes(t[16 + 12 + 8..][..4].try_into().unwrap()) as usize;
        assert_eq!(u32::from_le_bytes(t[pos + 8..][..4].try_into().unwrap()), 32);
        assert_eq!(u32::from_le_bytes(t[pos + 36..][..4].try_into().unwrap()), 32);
        assert_eq!(trim(&file(&[16]), &SIZES), None);
        assert_eq!(trim(b"nope", &SIZES), None);
    }
}
