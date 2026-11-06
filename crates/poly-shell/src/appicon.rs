//! The icons of installed apps (ADR 0146): a desktop entry's `Icon=` key
//! found the way the freedesktop.org Icon Theme Specification (version
//! 0.13) says, decoded with `poly-png`, scaled to the size the shell draws
//! them at, and kept in a small cache.
//!
//! - **Lookup** ([`candidates`]): an absolute path is the file itself; a
//!   name is looked for in the `hicolor` theme, in the size directory
//!   nearest to the wanted size (the smallest that is at least as big, so
//!   scaling down, else the biggest that is smaller), then in
//!   `/usr/share/pixmaps`. Only PNG files are read: SVG and XPM entries
//!   are skipped and the shell keeps its built-in glyph.
//! - **Loading** ([`AppIcons::get`]): lazily, the first time an icon is
//!   drawn at a size, through an [`IconFiles`] the compositor provides (the
//!   shell itself does no I/O). A name that is not found is remembered
//!   too, so the disk is not searched again at every frame.
//! - **Scaling** ([`Bitmap::fit`]): premultiplied alpha, averaging when
//!   making smaller, bilinear when making bigger.
//! - **Memory**: the cache holds at most [`MAX_ENTRIES`] icons and
//!   [`MAX_BYTES`] of pixels, the oldest dropped first; a file larger than
//!   [`MAX_FILE`] or [`MAX_PIXELS`] pixels is not read.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use poly_wayland::compose::{Framebuffer, Rect};

/// What the shell reads icons from: the compositor's view of the disk.
pub trait IconFiles: fmt::Debug {
    /// A file's contents, `None` if it is missing or unreadable.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// The names in a directory (empty if it is missing).
    fn list(&self, dir: &str) -> Vec<String>;
}

/// The icon directories (`$XDG_DATA_DIRS/icons`, the defaults).
pub const ICON_DIRS: [&str; 2] = ["/usr/share/icons", "/usr/local/share/icons"];
/// The theme every theme falls back to.
pub const THEME: &str = "hicolor";
/// Where flat, unthemed icons are (`$XDG_DATA_DIRS/pixmaps`).
pub const PIXMAPS: &str = "/usr/share/pixmaps";

/// The cache's bounds.
pub const MAX_ENTRIES: usize = 256;
pub const MAX_BYTES: usize = 4 << 20;
/// A file bigger than this is not read; nor is an image of more pixels.
pub const MAX_FILE: usize = 2 << 20;
pub const MAX_PIXELS: u64 = 1 << 22;

/// A directory name of a theme as a pixel size: `48x48` is 48, `48x48@2`
/// (twice the pixels for a high-DPI screen) 96; `scalable` and others are
/// not sizes.
pub fn parse_size_dir(name: &str) -> Option<u32> {
    let (dims, scale) = match name.split_once('@') {
        Some((d, s)) => (d, s.parse::<u32>().ok().filter(|&s| (1..=8).contains(&s))?),
        None => (name, 1),
    };
    let (w, h) = dims.split_once('x')?;
    let (w, h) = (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?);
    (w == h && (1..=4096).contains(&w)).then_some(w * scale)
}

/// The size directories of `hicolor` among `names` (a directory listing),
/// as (pixels, name), smallest first.
pub fn size_dirs(names: &[String]) -> Vec<(u32, String)> {
    let mut v: Vec<(u32, String)> = names.iter().filter_map(|n| Some((parse_size_dir(n)?, n.clone()))).collect();
    v.sort();
    v.dedup();
    v
}

/// An icon name without a file extension the lookup would not want, `None`
/// if it cannot be a name (empty, a path, or `..`).
fn bare_name(name: &str) -> Option<&str> {
    let name = [".png", ".svg", ".svgz", ".xpm"].iter().find_map(|e| name.strip_suffix(e)).unwrap_or(name);
    (!name.is_empty() && !name.contains('/') && name != "..").then_some(name)
}

/// The files to try for icon `icon` wanted at `want` pixels, best first:
/// no I/O. `sizes` are `hicolor`'s size directories ([`size_dirs`]).
pub fn candidates(icon: &str, want: u32, sizes: &[(u32, String)]) -> Vec<String> {
    let mut out = Vec::new();
    if icon.starts_with('/') {
        // An absolute path: only PNG files are read.
        if !icon.ends_with(".svg") && !icon.ends_with(".svgz") && !icon.ends_with(".xpm") && !icon.ends_with('/') {
            out.push(icon.to_string());
        }
        return out;
    }
    let Some(name) = bare_name(icon) else { return out };
    // The nearest sizes at least as big first (smallest first), then the
    // smaller ones, biggest first.
    let bigger = sizes.iter().filter(|(px, _)| *px >= want);
    let smaller = sizes.iter().rev().filter(|(px, _)| *px < want);
    for (_, dir) in bigger.chain(smaller) {
        for base in ICON_DIRS {
            out.push(alloc::format!("{base}/{THEME}/{dir}/apps/{name}.png"));
        }
    }
    out.push(alloc::format!("{PIXMAPS}/{name}.png"));
    out
}

/// A picture of at most `size` pixels each way, premultiplied `0xAARRGGBB`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// For output pixel `i` of `n` made from `src` input pixels: the first
/// input pixel and the weights of those from it on (they add up to 1).
fn taps(i: usize, n: usize, src: usize) -> (usize, Vec<f32>) {
    let ratio = src as f32 / n as f32;
    if ratio > 1.0 {
        // Making smaller: the average of the input covered, the edge
        // pixels by how much of them is.
        let (lo, hi) = (i as f32 * ratio, (i + 1) as f32 * ratio);
        let first = lo as usize;
        // (`f32::ceil` is not in core.)
        let whole = hi as usize;
        let last = (if whole as f32 >= hi { whole } else { whole + 1 }).min(src).max(first + 1);
        let mut w: Vec<f32> = (first..last).map(|p| ((p + 1) as f32).min(hi) - (p as f32).max(lo)).collect();
        let sum: f32 = w.iter().sum();
        for x in &mut w {
            *x /= sum;
        }
        (first, w)
    } else {
        // Making bigger (or the same): bilinear between the two nearest
        // pixel centres.
        let c = (i as f32 + 0.5) * ratio - 0.5;
        if c <= 0.0 {
            return (0, alloc::vec![1.0]);
        }
        let first = c as usize;
        if first + 1 >= src {
            return (src - 1, alloc::vec![1.0]);
        }
        let f = c - first as f32;
        (first, alloc::vec![1.0 - f, f])
    }
}

impl Bitmap {
    /// `img` scaled to fit `size` x `size` pixels, keeping its shape.
    pub fn fit(img: &poly_png::Image, size: u32) -> Bitmap {
        let (w, h) = (img.width as usize, img.height as usize);
        let longest = w.max(h);
        let scaled = |n: usize| ((n * size as usize + longest / 2) / longest).clamp(1, size as usize);
        let (nw, nh) = (scaled(w), scaled(h));
        // Premultiplied channels as floats: a transparent pixel's colour
        // must not bleed into its neighbours.
        let src: Vec<[f32; 4]> = img
            .pixels
            .iter()
            .map(|&p| {
                let a = (p >> 24) as f32 / 255.0;
                [((p >> 16) & 0xff) as f32 * a, ((p >> 8) & 0xff) as f32 * a, (p & 0xff) as f32 * a, a * 255.0]
            })
            .collect();
        // Across, then down.
        let mut across = alloc::vec![[0f32; 4]; nw * h];
        for x in 0..nw {
            let (first, w8) = taps(x, nw, w);
            for y in 0..h {
                let mut acc = [0f32; 4];
                for (k, wt) in w8.iter().enumerate() {
                    let p = src[y * w + first + k];
                    for c in 0..4 {
                        acc[c] += p[c] * wt;
                    }
                }
                across[y * nw + x] = acc;
            }
        }
        let mut pixels = alloc::vec![0u32; nw * nh];
        for y in 0..nh {
            let (first, wts) = taps(y, nh, h);
            for x in 0..nw {
                let mut acc = [0f32; 4];
                for (k, wt) in wts.iter().enumerate() {
                    let p = across[(first + k) * nw + x];
                    for c in 0..4 {
                        acc[c] += p[c] * wt;
                    }
                }
                let a = (acc[3] + 0.5) as u32;
                // Rounding must not push a colour past its alpha.
                let ch = |v: f32| ((v + 0.5) as u32).min(a);
                pixels[y * nw + x] = (a << 24) | (ch(acc[0]) << 16) | (ch(acc[1]) << 8) | ch(acc[2]);
            }
        }
        Bitmap { width: nw as u32, height: nh as u32, pixels }
    }

    /// Draw it centred in the `size` x `size` box at (`x`, `y`), the part
    /// inside `clip`, over what is there.
    pub fn draw(&self, fb: &mut dyn Framebuffer, clip: Rect, x: i32, y: i32, size: i32) {
        let (ox, oy) = (x + (size - self.width as i32) / 2, y + (size - self.height as i32) / 2);
        let r = Rect::new(ox, oy, self.width as i32, self.height as i32).intersect(&clip).intersect(&Rect::new(
            0,
            0,
            fb.width() as i32,
            fb.height() as i32,
        ));
        for py in r.y..r.bottom() {
            let row = &mut fb.row_mut(py as u32)[r.x as usize..r.right() as usize];
            let src = &self.pixels[(py - oy) as usize * self.width as usize..][(r.x - ox) as usize..];
            for (d, &s) in row.iter_mut().zip(src) {
                let a = s >> 24;
                if a == 0 {
                    continue;
                }
                if a == 255 {
                    *d = s;
                    continue;
                }
                // Premultiplied "over": the source plus what shows through.
                let keep = 255 - a;
                let over = |shift: u32| {
                    let (sc, dc) = ((s >> shift) & 0xff, (*d >> shift) & 0xff);
                    (sc + (dc * keep + 127) / 255).min(255) << shift
                };
                *d = 0xff00_0000 | over(16) | over(8) | over(0);
            }
        }
    }

    fn bytes(&self) -> usize {
        self.pixels.len() * 4
    }
}

/// The icons the shell has loaded.
#[derive(Default)]
pub struct AppIcons {
    files: Option<Box<dyn IconFiles>>,
    /// `hicolor`'s size directories, listed at the first lookup.
    sizes: Option<Vec<(u32, String)>>,
    /// By icon and size; `None`: looked for, not found.
    cache: BTreeMap<(String, u32), Option<Rc<Bitmap>>>,
    /// The keys, oldest first.
    order: VecDeque<(String, u32)>,
    bytes: usize,
}

impl fmt::Debug for AppIcons {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppIcons")
            .field("source", &self.files.is_some())
            .field("cached", &self.cache.len())
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl AppIcons {
    /// Where icons are read from; without it there are none (the shell
    /// draws its glyphs).
    pub fn set_files(&mut self, files: Box<dyn IconFiles>) {
        self.files = Some(files);
        self.sizes = None;
        self.cache.clear();
        self.order.clear();
        self.bytes = 0;
    }

    /// Whether icons can be loaded at all.
    pub fn is_enabled(&self) -> bool {
        self.files.is_some()
    }

    /// How many icons (and misses) are cached, and the bytes of pixels.
    pub fn cached(&self) -> (usize, usize) {
        (self.cache.len(), self.bytes)
    }

    /// Icon `icon` (a `Icon=` value) at `size` pixels: loaded the first
    /// time, `None` if there is none (or it is not a PNG).
    pub fn get(&mut self, icon: &str, size: u32) -> Option<Rc<Bitmap>> {
        let files = self.files.as_ref()?;
        let key = (icon.to_string(), size);
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let sizes = self.sizes.get_or_insert_with(|| {
            let mut names = Vec::new();
            for base in ICON_DIRS {
                names.extend(files.list(&alloc::format!("{base}/{THEME}")));
            }
            size_dirs(&names)
        });
        let mut found = None;
        let mut broken = 0;
        for path in candidates(icon, size, sizes) {
            let Some(data) = files.read(&path).filter(|d| d.len() <= MAX_FILE) else { continue };
            if let Ok(img) = poly_png::decode_with(&data, &poly_png::Limits { max_pixels: MAX_PIXELS }) {
                found = Some(Rc::new(Bitmap::fit(&img, size)));
                break;
            }
            // Files that are there but cannot be used: not the whole theme.
            broken += 1;
            if broken == MAX_BROKEN {
                break;
            }
        }
        self.insert(key, found.clone());
        found
    }

    fn insert(&mut self, key: (String, u32), value: Option<Rc<Bitmap>>) {
        self.bytes += value.as_ref().map_or(0, |b| b.bytes());
        self.cache.insert(key.clone(), value);
        self.order.push_back(key);
        // Drop the oldest until it fits (but keep what was just added).
        while self.order.len() > 1 && (self.order.len() > MAX_ENTRIES || self.bytes > MAX_BYTES) {
            if let Some(old) = self.order.pop_front()
                && let Some(Some(b)) = self.cache.remove(&old)
            {
                self.bytes -= b.bytes();
            }
        }
    }
}

/// How many files that are there but do not decode one lookup tries.
const MAX_BROKEN: usize = 3;

/// A fake disk and PNG files for the shell's tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use alloc::format;
    use alloc::vec;
    use core::cell::RefCell;

    /// A disk of files, counting reads.
    #[derive(Debug, Default)]
    pub struct Disk {
        pub files: BTreeMap<String, Vec<u8>>,
        pub reads: RefCell<Vec<String>>,
    }

    impl IconFiles for Rc<Disk> {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.reads.borrow_mut().push(path.to_string());
            self.files.get(path).cloned()
        }
        fn list(&self, dir: &str) -> Vec<String> {
            let prefix = format!("{dir}/");
            let mut names: Vec<String> = self
                .files
                .keys()
                .filter_map(|p| p.strip_prefix(&prefix))
                .filter_map(|r| r.split('/').next())
                .map(ToString::to_string)
                .collect();
            names.dedup();
            names
        }
    }

    /// A PNG of a `w` x `h` image of one colour (RGBA, stored deflate).
    pub fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        // Each row: filter type 0, then the pixels.
        let mut row = vec![0u8];
        for _ in 0..w {
            row.extend_from_slice(&rgba);
        }
        let raw = row.repeat(h as usize);
        let mut z = vec![0x78, 0x01];
        let mut chunks = raw.chunks(60_000).peekable();
        while let Some(c) = chunks.next() {
            z.push(u8::from(chunks.peek().is_none()));
            z.extend_from_slice(&(c.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
            z.extend_from_slice(c);
        }
        z.extend_from_slice(&poly_png::inflate::adler32(&raw).to_be_bytes());
        let chunk = |kind: &[u8; 4], data: &[u8]| {
            let mut c = (data.len() as u32).to_be_bytes().to_vec();
            c.extend_from_slice(kind);
            c.extend_from_slice(data);
            c.extend_from_slice(&poly_png::crc32(&[kind, data]).to_be_bytes());
            c
        };
        let mut head = w.to_be_bytes().to_vec();
        head.extend_from_slice(&h.to_be_bytes());
        head.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut f = poly_png::SIGNATURE.to_vec();
        f.extend(chunk(b"IHDR", &head));
        f.extend(chunk(b"IDAT", &z));
        f.extend(chunk(b"IEND", &[]));
        f
    }

    /// A disk with these files.
    pub fn disk(files: &[(&str, Vec<u8>)]) -> Rc<Disk> {
        Rc::new(Disk { files: files.iter().map(|(p, d)| ((*p).to_string(), d.clone())).collect(), ..Disk::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use alloc::format;
    use alloc::vec;
    use poly_wayland::compose::MemoryFramebuffer;

    fn icons(files: &[(&str, Vec<u8>)]) -> (AppIcons, Rc<Disk>) {
        let disk = disk(files);
        let mut a = AppIcons::default();
        a.set_files(Box::new(disk.clone()));
        (a, disk)
    }

    const RED: [u8; 4] = [255, 0, 0, 255];

    #[test]
    fn size_directories() {
        assert_eq!(parse_size_dir("48x48"), Some(48));
        assert_eq!(parse_size_dir("48x48@2"), Some(96));
        assert_eq!(parse_size_dir("512x512"), Some(512));
        assert_eq!(parse_size_dir("scalable"), None);
        assert_eq!(parse_size_dir("symbolic"), None);
        assert_eq!(parse_size_dir("16x32"), None);
        assert_eq!(parse_size_dir("0x0"), None);
        assert_eq!(parse_size_dir("48x48@x"), None);
        assert_eq!(parse_size_dir("48x48@0"), None);
        let names: Vec<String> =
            ["scalable", "48x48", "16x16", "256x256", "24x24@2", "48x48"].map(String::from).to_vec();
        let dirs: Vec<u32> = size_dirs(&names).iter().map(|d| d.0).collect();
        assert_eq!(dirs, [16, 48, 48, 256], "sorted, each directory once");
    }

    #[test]
    fn candidates_prefer_the_nearest_size_at_least_as_big() {
        let sizes = size_dirs(&["16x16", "32x32", "48x48", "128x128", "scalable"].map(String::from));
        let paths = |name: &str, want| -> Vec<String> {
            candidates(name, want, &sizes)
                .iter()
                .map(|p| p.replace("/usr/share/icons/hicolor/", "").replace("/apps", ""))
                .collect()
        };
        // 28 wanted: 32, 48, 128 (bigger, nearest first), then 16; the
        // second icon directory after each; pixmaps last.
        let got = paths("chromium", 28);
        assert_eq!(got[0], "32x32/chromium.png");
        assert!(got[1].starts_with("/usr/local/share/icons/hicolor/32x32"));
        let firsts: Vec<&str> = got.iter().step_by(2).take(4).map(String::as_str).collect();
        assert_eq!(firsts, ["32x32/chromium.png", "48x48/chromium.png", "128x128/chromium.png", "16x16/chromium.png"]);
        assert_eq!(got.last().unwrap(), "/usr/share/pixmaps/chromium.png");
        // An exact size comes first; a size nothing reaches falls to the biggest.
        assert_eq!(paths("x", 48)[0], "48x48/x.png");
        assert_eq!(paths("x", 500)[0], "128x128/x.png");
        // An extension in the name is dropped.
        assert_eq!(paths("foo.png", 16)[0], "16x16/foo.png");
        assert_eq!(paths("foo.svg", 16)[0], "16x16/foo.png");
        // Without sizes: only pixmaps.
        assert_eq!(candidates("a", 16, &[]), ["/usr/share/pixmaps/a.png"]);
    }

    #[test]
    fn paths_and_odd_names() {
        assert_eq!(candidates("/opt/app/icon.png", 28, &[]), ["/opt/app/icon.png"]);
        assert_eq!(candidates("/opt/app/icon", 28, &[]), ["/opt/app/icon"]);
        assert!(candidates("/opt/app/icon.svg", 28, &[]).is_empty(), "SVG is skipped");
        assert!(candidates("/opt/app/icon.xpm", 28, &[]).is_empty());
        assert!(candidates("/opt/app/", 28, &[]).is_empty());
        for bad in ["", "..", "../../etc/passwd", "a/b", ".png"] {
            assert!(candidates(bad, 28, &[]).is_empty(), "{bad:?}");
        }
    }

    #[test]
    fn an_icon_is_found_in_hicolor_scaled_and_cached() {
        let (mut a, disk) = icons(&[
            ("/usr/share/icons/hicolor/16x16/apps/foo.png", png(16, 16, [0, 255, 0, 255])),
            ("/usr/share/icons/hicolor/64x64/apps/foo.png", png(64, 64, RED)),
            ("/usr/share/icons/hicolor/scalable/apps/foo.svg", b"<svg/>".to_vec()),
        ]);
        let b = a.get("foo", 28).unwrap();
        assert_eq!((b.width, b.height), (28, 28));
        assert_eq!(b.pixels[14 * 28 + 14], 0xffff_0000, "the 64-pixel file: the smallest that is big enough");
        let reads = disk.reads.borrow().len();
        // The second time nothing is read, and the same bitmap comes back.
        let again = a.get("foo", 28).unwrap();
        assert!(Rc::ptr_eq(&b, &again));
        assert_eq!(disk.reads.borrow().len(), reads);
        // Another size is another entry; at 12 pixels the 16-pixel file.
        let small = a.get("foo", 12).unwrap();
        assert_eq!(small.pixels[6 * 12 + 6], 0xff00_ff00);
        assert_eq!(a.cached().0, 2);
    }

    #[test]
    fn debians_chromium_icons() {
        // The files of Debian's package, as the Chromium disk has them.
        let dir = "/usr/share/icons/hicolor";
        let (mut a, disk) = icons(&[
            (
                &format!("{dir}/16x16/apps/chromium.png"),
                include_bytes!("../../poly-png/tests/data/chromium-16.png").to_vec(),
            ),
            (
                &format!("{dir}/48x48/apps/chromium.png"),
                include_bytes!("../../poly-png/tests/data/chromium-48.png").to_vec(),
            ),
            (
                &format!("{dir}/256x256/apps/chromium.png"),
                include_bytes!("../../poly-png/tests/data/chromium-256.png").to_vec(),
            ),
        ]);
        let b = a.get("chromium", 28).unwrap();
        assert_eq!((b.width, b.height), (28, 28));
        assert_eq!(b.pixels[0] >> 24, 0, "the corner is clear");
        let mid = b.pixels[14 * 28 + 14];
        assert_eq!(mid >> 24, 255);
        let (r, g, bl) = ((mid >> 16) & 0xff, (mid >> 8) & 0xff, mid & 0xff);
        assert!(r.abs_diff(0x1a) < 24 && g.abs_diff(0x73) < 24 && bl.abs_diff(0xe8) < 24, "Chromium's blue: {mid:08x}");
        assert!(disk.reads.borrow()[0].contains("48x48"), "48 is the nearest size above 28");
        // At twice the size the 256-pixel file is the nearest above 56.
        let big = a.get("chromium", 56).unwrap();
        assert_eq!((big.width, big.height), (56, 56));
        assert!(disk.reads.borrow().last().unwrap().contains("256x256"));
        // And a size above all of them takes the biggest.
        assert_eq!(a.get("chromium", 300).unwrap().width, 300);
    }

    #[test]
    fn misses_are_remembered_and_pixmaps_and_paths_work() {
        let (mut a, disk) = icons(&[
            ("/usr/share/pixmaps/old.png", png(32, 32, RED)),
            ("/home/me/mine.png", png(8, 8, RED)),
            ("/usr/share/icons/hicolor/48x48/apps/broken.png", b"not a png".to_vec()),
        ]);
        assert!(a.get("old", 28).is_some());
        assert!(a.get("/home/me/mine.png", 28).is_some());
        assert!(a.get("broken", 28).is_none(), "a file that is no PNG");
        assert!(a.get("nothing", 28).is_none());
        let reads = disk.reads.borrow().len();
        assert!(a.get("nothing", 28).is_none());
        assert!(a.get("broken", 28).is_none());
        assert_eq!(disk.reads.borrow().len(), reads, "misses are not searched for again");
        // Without a source there are no icons and nothing to cache.
        let mut none = AppIcons::default();
        assert!(!none.is_enabled());
        assert!(none.get("old", 28).is_none());
        assert_eq!(none.cached(), (0, 0));
    }

    #[test]
    fn big_files_and_images_are_not_read() {
        let (mut a, _) = icons(&[
            ("/big.png", vec![0; MAX_FILE + 1]),
            ("/wide.png", png(2500, 2000, RED)),
            ("/fine.png", png(100, 100, RED)),
        ]);
        assert!(a.get("/big.png", 28).is_none());
        assert!(a.get("/wide.png", 28).is_none(), "more than {MAX_PIXELS} pixels");
        assert!(a.get("/fine.png", 28).is_some());
    }

    #[test]
    fn the_cache_stays_bounded() {
        let (mut a, _) = icons(&[("/usr/share/pixmaps/i.png", png(4, 4, RED))]);
        for i in 0..(MAX_ENTRIES + 50) {
            a.get(&format!("missing-{i}"), 28);
        }
        assert_eq!(a.cached().0, MAX_ENTRIES);
        // Bytes too: big bitmaps push out old ones.
        let mut a = AppIcons::default();
        a.set_files(Box::new(Rc::new(Disk {
            files: [("/p.png".to_string(), png(4, 4, RED))].into_iter().collect(),
            ..Disk::default()
        })));
        for size in 100..400u32 {
            a.get("/p.png", size);
            assert!(a.cached().1 <= MAX_BYTES + 4 * 400 * 400);
        }
        assert!(a.cached().1 <= MAX_BYTES);
    }

    #[test]
    fn scaling_keeps_shape_colour_and_transparency() {
        // A 4 x 2 opaque image fits into 8 x 8 as 8 x 4.
        let img = poly_png::Image { width: 4, height: 2, pixels: vec![0xff80_4020; 8] };
        let b = Bitmap::fit(&img, 8);
        assert_eq!((b.width, b.height), (8, 4));
        assert!(b.pixels.iter().all(|&p| p == 0xff80_4020), "{:x?}", b.pixels);
        // Down: a checkerboard of black and white averages to grey.
        let px: Vec<u32> = (0..64).map(|i| if (i % 8 + i / 8) % 2 == 0 { 0xff00_0000 } else { 0xffff_ffff }).collect();
        let b = Bitmap::fit(&poly_png::Image { width: 8, height: 8, pixels: px }, 2);
        assert!(b.pixels.iter().all(|&p| (0x7c..=0x83).contains(&(p & 0xff)) && p >> 24 == 255), "{:x?}", b.pixels);
        // A transparent border does not bleed its colour into the edge.
        let mut px = vec![0x00ff_0000; 16];
        for y in 1..3 {
            for x in 1..3 {
                px[y * 4 + x] = 0xff00_00ff;
            }
        }
        let b = Bitmap::fit(&poly_png::Image { width: 4, height: 4, pixels: px }, 2);
        for p in &b.pixels {
            let a = p >> 24;
            assert_eq!(p & 0x00ff_0000, 0, "no red from the transparent pixels: {p:08x}");
            assert!((p & 0xff) <= a, "premultiplied");
            assert!(a > 50 && a < 255, "the edge is partly covered: {a}");
        }
        // Up: 1 x 1 stays one colour; 2 x 1 blends between.
        let b = Bitmap::fit(&poly_png::Image { width: 1, height: 1, pixels: vec![0xff11_2233] }, 5);
        assert_eq!((b.width, b.height), (5, 5));
        assert!(b.pixels.iter().all(|&p| p == 0xff11_2233));
        let b = Bitmap::fit(&poly_png::Image { width: 2, height: 1, pixels: vec![0xff00_0000, 0xffff_ffff] }, 8);
        let row: Vec<u32> = b.pixels[..8].iter().map(|p| p & 0xff).collect();
        assert!(row.windows(2).all(|w| w[0] <= w[1]), "{row:?}");
        assert!(row[0] < 8 && row[7] > 247);
    }

    #[test]
    fn drawing_blends_over_what_is_there() {
        let at = |fb: &MemoryFramebuffer, x: usize, y: usize| fb.pixels()[y * 10 + x];
        let mut fb = MemoryFramebuffer::new(10, 10);
        for y in 0..10 {
            fb.row_mut(y).fill(0xff00_00ff);
        }
        // Premultiplied: an opaque pixel, a half-transparent white (alpha
        // 128, colour 128), a clear pixel.
        let b = Bitmap { width: 3, height: 1, pixels: vec![0xff11_2233, 0x8080_8080, 0x0000_0000] };
        b.draw(&mut fb, Rect::new(0, 0, 10, 10), 2, 4, 3);
        // Centred in a 3-pixel box: the picture is 3 x 1, so it is on row 5.
        assert_eq!(at(&fb, 2, 5), 0xff11_2233);
        let half = at(&fb, 3, 5);
        assert_eq!(half >> 24, 255);
        assert_eq!((half >> 16) & 0xff, 0x80, "white at half over blue: red is half");
        assert_eq!((half >> 8) & 0xff, 0x80);
        assert_eq!(half & 0xff, 0xff, "blue: 128 of white plus 127 of what was below");
        assert_eq!(at(&fb, 4, 5), 0xff00_00ff, "clear: untouched");
        // Clipped: nothing outside the clip changes.
        let mut fb2 = MemoryFramebuffer::new(10, 10);
        b.draw(&mut fb2, Rect::new(3, 0, 1, 10), 2, 4, 3);
        assert_eq!(at(&fb2, 2, 5), 0);
        assert_eq!(at(&fb2, 3, 5) >> 24, 255);
        // Off the edge of the screen: no panic.
        b.draw(&mut fb2, Rect::new(0, 0, 10, 10), 9, 9, 3);
        b.draw(&mut fb2, Rect::new(0, 0, 10, 10), -5, -5, 3);
    }
}
