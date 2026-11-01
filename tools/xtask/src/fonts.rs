//! The font store (roadmap Phase 6, "Fonts"; ADR 0105): the bundled
//! fonts, fetched and pinned, and `cargo xtask fonts ...` to fetch, list,
//! match and render them with `poly-font`.
//!
//! # The bundled fonts
//!
//! DejaVu 2.37 (Bitstream Vera licence for the Vera glyphs, public domain
//! for the DejaVu changes, the Arev licence for the glyphs from Arev; all
//! permissive, see `third_party/dejavu/LICENSE`): Sans, Serif and Sans
//! Mono, each regular, bold, italic or oblique and bold italic or oblique,
//! twelve files of 5.5 MB together. Like 7-Zip (`sevenzip.rs`) they are
//! not committed: `cargo xtask fonts fetch` downloads the release archive
//! from the project's GitHub releases over HTTPS into the main checkout's
//! `target/fonts/` (shared by its worktrees), checks it against the
//! pinned size and SHA-256, unpacks only those twelve files and the
//! licence into a fresh, empty directory, checks each of them, and
//! deletes the archive. The project publishes no hashes; the SHA-256 sums
//! here are of the files fetched when they were pinned (2026-10-31).
//!
//! # Where they go
//!
//! On Poly's root disk (`rootdisk.rs`, when the fonts are fetched): the
//! files in `/usr/share/fonts/truetype/dejavu/` with the licence beside
//! them, Poly's `fonts.conf` (`poly_font::fc::POLY_FONTS_CONF`) as
//! `/etc/fonts/fonts.conf`, and for Windows programs a relative symbolic
//! link to each file in `C:\Windows\Fonts` (`/poly/nt/c/Windows/Fonts`
//! on the disk, ADR 0073), so both worlds share one copy.
//!
//! # Commands
//!
//! - `fonts fetch`: the download above.
//! - `fonts list [pattern] [element...]`: `fc-list` over the bundled fonts.
//! - `fonts match [--sort] <pattern>`: `fc-match`.
//! - `fonts render [--lcd] [--size <px>] [--width <px>] [--out <file.png>] <pattern> <text...>`:
//!   lay out and render text in the matched font to a PNG (default
//!   `target/poly-font.png`).

use crate::Result;
use poly_font::fc::{Config, FontSet, POLY_FONTS_CONF, Pattern, list_line, match_line};
use poly_font::{Font, GlyphCache, LayoutOptions, RenderMode, layout, render_layout};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The DejaVu release.
pub const VERSION: &str = "2.37";

const ARCHIVE: &str = "dejavu-fonts-ttf-2.37.tar.bz2";
const ARCHIVE_URL: &str =
    "https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.tar.bz2";
const ARCHIVE_SIZE: u64 = 5_429_777;
const ARCHIVE_SHA256: &str = "fa9ca4d13871dd122f61258a80d01751d603b4d3ee14095d65453b4e846e17d7";
/// The directory in the archive.
const ARCHIVE_DIR: &str = "dejavu-fonts-ttf-2.37";

/// The bundled files: name, size, SHA-256.
pub const FILES: [(&str, u64, &str); 12] = [
    ("DejaVuSans.ttf", 757_076, "7da195a74c55bef988d0d48f9508bd5d849425c1770dba5d7bfc6ce9ed848954"),
    ("DejaVuSans-Bold.ttf", 705_684, "e6476c1b80502924294eed40894c5b18e06c181444ca953e5334262df9c27724"),
    ("DejaVuSans-Oblique.ttf", 635_416, "4af75fa16ee6d3ad43e1ecec41862c24954af26a55c6bb1ebb27bd486a50f5f4"),
    ("DejaVuSans-BoldOblique.ttf", 643_292, "eb436dca0c2594b73d8b603b892e374fdfd8d885d25ffb4f18df4c4c0b49e50f"),
    ("DejaVuSerif.ttf", 380_132, "42d1edeb7952f31b1f96d767ed7030b08a39e0c372b0071641518864e2bffb51"),
    ("DejaVuSerif-Bold.ttf", 356_088, "c47b5527bcdc8dcf9ea8c77054454c5a884beaca2f44851a2a823ee639cbf07f"),
    ("DejaVuSerif-Italic.ttf", 345_996, "2e39b1d50f90b933b00c7bb54a96afd3f86419b3d717c7cf202e36f2d4973e47"),
    ("DejaVuSerif-BoldItalic.ttf", 347_460, "8d3dd3d31350309042ed226af82b34539bd773518e6107cb352712853ba80308"),
    ("DejaVuSansMono.ttf", 340_712, "b4a6c3e4faab8773f4ff761d56451646409f29abedd68f05d38c2df667d3c582"),
    ("DejaVuSansMono-Bold.ttf", 331_992, "bce60f1b4421acd9ea51ba6623d7024ecbe6817a953e3654df62a5e6bdf8f769"),
    ("DejaVuSansMono-Oblique.ttf", 251_932, "742097840c541870e8d6dc5c9b37bb1ceeea6c0dedd1d475faf903ef9df734b0"),
    ("DejaVuSansMono-BoldOblique.ttf", 253_580, "91713a71d550bba22c2a6b2bb2a9ad8f9a159e12e4e9f0a5b2677998ba21213e"),
];

/// The licence in the archive; the same text is `third_party/dejavu/LICENSE`.
const LICENSE: &str = "LICENSE";

/// Where the fonts live on Poly's root disk, and Windows' font folder
/// there (`C:\Windows\Fonts`).
pub const STORE_DIR: &str = "usr/share/fonts/truetype/dejavu";
pub const WINDOWS_FONTS_DIR: &str = "poly/nt/c/Windows/Fonts";

/// The environment variable naming the font directory to `poly-font`'s
/// tests (`crates/poly-font/tests/common`).
pub const TEST_ENV: &str = "POLY_FONT_DIR";

/// `<main checkout>/target/fonts/dejavu-<version>/`.
pub fn cache_dir() -> PathBuf {
    let root = crate::root();
    let ntdll = crate::ntdll::cache_dir(&root);
    ntdll
        .parent()
        .map_or_else(|| root.join("target"), Path::to_path_buf)
        .join("fonts")
        .join(format!("dejavu-{VERSION}"))
}

fn sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg(path).stderr(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_string)
}

fn matches(path: &Path, size: u64, sum: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == size) && sha256(path).as_deref() == Some(sum)
}

/// The cache directory, if it holds every pinned file and the licence.
pub fn cached() -> Option<PathBuf> {
    let dir = cache_dir();
    let ok = dir.join(LICENSE).is_file() && FILES.iter().all(|(f, size, sum)| matches(&dir.join(f), *size, sum));
    ok.then_some(dir)
}

/// `cargo xtask fonts fetch`.
pub fn fetch() -> Result<PathBuf> {
    if let Some(dir) = cached() {
        eprintln!("==> {} holds the pinned DejaVu {VERSION} fonts", dir.display());
        return Ok(dir);
    }
    for tool in ["curl", "sha256sum", "tar", "bzip2"] {
        if Command::new(tool).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
            return Err(format!("fetching the fonts needs `{tool}`"));
        }
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let archive = dir.join(ARCHIVE);
    if !matches(&archive, ARCHIVE_SIZE, ARCHIVE_SHA256) {
        eprintln!("==> downloading {ARCHIVE_URL}");
        let partial = archive.with_extension("part");
        let fetched = Command::new("curl")
            .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "3", "-o"])
            .arg(&partial)
            .arg(ARCHIVE_URL)
            .status();
        if !fetched.is_ok_and(|s| s.success()) {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("downloading {ARCHIVE_URL} failed"));
        }
        std::fs::rename(&partial, &archive).map_err(|e| e.to_string())?;
    }
    if !matches(&archive, ARCHIVE_SIZE, ARCHIVE_SHA256) {
        let got = sha256(&archive).unwrap_or_default();
        let _ = std::fs::remove_file(&archive);
        return Err(format!("{ARCHIVE} is not the pinned file (SHA-256 {ARCHIVE_SHA256}, got {got}); removed"));
    }
    // Only the files named, into a fresh, empty directory.
    let unpack = dir.join("unpack");
    let _ = std::fs::remove_dir_all(&unpack);
    std::fs::create_dir_all(&unpack).map_err(|e| format!("{}: {e}", unpack.display()))?;
    let mut members: Vec<String> = FILES.iter().map(|(f, _, _)| format!("{ARCHIVE_DIR}/ttf/{f}")).collect();
    members.push(format!("{ARCHIVE_DIR}/{LICENSE}"));
    let unpacked = Command::new("tar")
        .arg("-xjf")
        .arg(&archive)
        .arg("-C")
        .arg(&unpack)
        .arg("--no-same-owner")
        .arg("--")
        .args(&members)
        .status();
    if !unpacked.is_ok_and(|s| s.success()) {
        return Err(format!("could not unpack the fonts from {}", archive.display()));
    }
    for (f, size, sum) in FILES {
        let got = unpack.join(ARCHIVE_DIR).join("ttf").join(f);
        if !matches(&got, size, sum) {
            return Err(format!("{f} in {ARCHIVE} is not the pinned file"));
        }
        std::fs::rename(&got, dir.join(f)).map_err(|e| format!("{f}: {e}"))?;
    }
    std::fs::rename(unpack.join(ARCHIVE_DIR).join(LICENSE), dir.join(LICENSE))
        .map_err(|e| format!("{LICENSE}: {e}"))?;
    let _ = std::fs::remove_dir_all(&unpack);
    let _ = std::fs::remove_file(&archive);
    eprintln!("==> DejaVu {VERSION}: {} fonts in {}", FILES.len(), dir.display());
    Ok(dir)
}

/// Put the font store into a root tree: the fonts (if fetched), their
/// licence, `fonts.conf`, and the `C:\Windows\Fonts` links. Returns
/// whether the fonts were there.
pub fn add_to_root(tree: &mut crate::rootdisk::RootTree) -> Result<bool> {
    tree.file("etc/fonts/fonts.conf", 0o644, POLY_FONTS_CONF.as_bytes());
    tree.dir("var/cache/fontconfig", 0o755);
    let Some(dir) = cached() else { return Ok(false) };
    for (f, _, _) in FILES {
        let data = std::fs::read(dir.join(f)).map_err(|e| format!("{f}: {e}"))?;
        tree.file(&format!("{STORE_DIR}/{f}"), 0o644, &data);
        // From poly/nt/c/Windows/Fonts up five levels to the disk's root.
        tree.symlink(&format!("{WINDOWS_FONTS_DIR}/{f}"), &format!("../../../../../{STORE_DIR}/{f}"));
    }
    let license = std::fs::read(dir.join(LICENSE)).map_err(|e| format!("{LICENSE}: {e}"))?;
    tree.file(&format!("{STORE_DIR}/{LICENSE}"), 0o644, &license);
    Ok(true)
}

/// The bundled fonts as a font set, under their paths on Poly.
fn font_set() -> Result<FontSet> {
    let dir = cached().ok_or("the fonts are not fetched (cargo xtask fonts fetch)")?;
    let mut set = FontSet::new();
    for (f, _, _) in FILES {
        let data = std::fs::read(dir.join(f)).map_err(|e| format!("{f}: {e}"))?;
        set.add_file(&format!("/{STORE_DIR}/{f}"), &data);
    }
    Ok(set)
}

fn config() -> Result<Config> {
    Config::parse(POLY_FONTS_CONF).map_err(|e| format!("fonts.conf: {e}"))
}

fn pattern(text: &str) -> Result<Pattern> {
    Pattern::parse(text).map_err(|e| format!("pattern {text:?}: {e}"))
}

/// `cargo xtask fonts ...`.
pub fn command(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("fetch") => fetch().map(|_| ()),
        Some("list") => {
            let set = font_set()?;
            let p = pattern(args.get(1).map_or("", String::as_str))?;
            let elements: Vec<&str> = args.iter().skip(2).map(String::as_str).collect();
            let mut lines: Vec<String> = set.list(&p).into_iter().map(|f| list_line(f, &elements)).collect();
            lines.sort();
            lines.dedup();
            for l in lines {
                println!("{l}");
            }
            Ok(())
        }
        Some("match") => {
            let set = font_set()?;
            let config = config()?;
            let sort = args.iter().any(|a| a == "--sort");
            let text = args.iter().skip(1).find(|a| !a.starts_with("--")).map_or("", String::as_str);
            let p = pattern(text)?;
            if sort {
                for f in set.sort(&config, &p) {
                    println!("{}", match_line(f));
                }
            } else if let Some(m) = set.font_match(&config, &p) {
                println!("{}", match_line(&m));
            }
            Ok(())
        }
        Some("render") => render(&args[1..]),
        _ => Err("usage: cargo xtask fonts <fetch | list [pattern] [element...] | match [--sort] <pattern> | render [--lcd] [--size <px>] [--width <px>] [--out <file.png>] <pattern> <text...>>".into()),
    }
}

fn render(args: &[String]) -> Result {
    let mut size = 16.0f32;
    let mut width = None;
    let mut out = crate::root().join("target/poly-font.png");
    let mut mode = RenderMode::Gray;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--lcd" => mode = RenderMode::Lcd,
            "--size" => size = it.next().and_then(|s| s.parse().ok()).ok_or("--size needs a number")?,
            "--width" => width = Some(it.next().and_then(|s| s.parse().ok()).ok_or("--width needs a number")?),
            "--out" => out = PathBuf::from(it.next().ok_or("--out needs a file")?),
            _ => rest.push(a.as_str()),
        }
    }
    let (pat, text) = rest.split_first().ok_or("render needs a pattern and text")?;
    let set = font_set()?;
    let m = set.font_match(&config()?, &pattern(pat)?).ok_or("no font")?;
    let file = m.get_str("file").ok_or("no file")?;
    let name = Path::new(file).file_name().ok_or("no file name")?;
    let data = std::fs::read(cached().ok_or("no fonts")?.join(name)).map_err(|e| format!("{file}: {e}"))?;
    let font = Font::parse(&data).map_err(|e| format!("{file}: {e}"))?;
    let text = text.join(" ").replace("\\n", "\n");
    let l = layout(&font, &text, &LayoutOptions { max_width: width, ..LayoutOptions::new(size) });
    let img = render_layout(&font, 0, &mut GlyphCache::new(4 << 20), &l, mode);
    std::fs::write(&out, poly_font::png::encode_image(&img)).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!(
        "==> {} ({}): {} lines, {}x{} px -> {}",
        match_line(&m),
        size,
        l.lines.len(),
        img.width,
        img.height,
        out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real fontconfig (the host's `fc-match`; GTK and Qt link the same
    /// library on the toolkits' disk, ADR 0120) reads Poly's `fonts.conf`
    /// without a warning. Skipped where the host has no fontconfig.
    #[test]
    fn real_fontconfig_reads_polys_fonts_conf_quietly() {
        let dir = std::env::temp_dir().join(format!("poly-fc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("fonts.conf");
        std::fs::write(&conf, POLY_FONTS_CONF).unwrap();
        let out = Command::new("fc-match")
            .env("FONTCONFIG_FILE", &conf)
            .env("XDG_CACHE_HOME", &dir)
            .arg("Arial")
            .stdout(Stdio::null())
            .output();
        let _ = std::fs::remove_dir_all(&dir);
        let Ok(out) = out else { return };
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("Fontconfig"), "fontconfig complained about fonts.conf: {stderr}");
    }
}
