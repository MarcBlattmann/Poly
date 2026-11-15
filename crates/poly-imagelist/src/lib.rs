//! The rules of comctl32's image lists (Learn, "Image Lists", `commctrl.h`).
//!
//! An image list holds images of one size, `cx` by `cy`. Here an image is
//! a block of `0xAARRGGBB` pixels: alpha 0 is transparent (what a mask
//! says), 255 opaque, and anything between is a source's own alpha. The
//! DLL (`personalities/nt/win32/comctl32`) reads GDI bitmaps into
//! [`Bitmap`]s and writes pixels out again; everything that is a rule of
//! the list lives here, so it is tested on the host.
//!
//! Observed behaviour (`specs/windows/comctl32.dll/ImageList_*.yaml`, comctl32
//! version 5.82): sizes must be positive; a new list has background colour
//! `CLR_NONE`; `ImageList_Add` takes `width / cx` images from a bitmap
//! (none: failure) and ignores a mask bitmap when the list has no
//! `ILC_MASK`; `ImageList_ReplaceIcon` with -1 appends and fails for an
//! index past the end; `ImageList_Remove(-1)` empties the list.

#![no_std]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

/// `CLR_NONE`, `CLR_DEFAULT` (`commctrl.h`).
pub const CLR_NONE: u32 = 0xFFFF_FFFF;
pub const CLR_DEFAULT: u32 = 0xFF00_0000;

/// `ILC_*` (`commctrl.h`).
pub const ILC_MASK: u32 = 0x1;
pub const ILC_COLORDDB: u32 = 0xFE;
pub const ILC_COLOR4: u32 = 0x4;
pub const ILC_COLOR8: u32 = 0x8;
pub const ILC_COLOR16: u32 = 0x10;
pub const ILC_COLOR24: u32 = 0x18;
pub const ILC_COLOR32: u32 = 0x20;

/// `ILD_*` (`commctrl.h`).
pub const ILD_NORMAL: u32 = 0;
pub const ILD_TRANSPARENT: u32 = 1;
pub const ILD_BLEND25: u32 = 2;
pub const ILD_BLEND50: u32 = 4;
pub const ILD_MASK: u32 = 0x10;
pub const ILD_IMAGE: u32 = 0x20;
pub const ILD_OVERLAYMASK: u32 = 0xF00;

/// `ILCF_*`: `ImageList_Copy` flags.
pub const ILCF_SWAP: u32 = 1;

/// The images lie in rows of this many in the image bitmap.
pub const COLUMNS: i32 = 4;

/// The largest edge of an image the list accepts.
pub const MAX_EDGE: i32 = 16384;

/// A bitmap read out of GDI: `w * h` pixels of `B, G, R, A` bytes, rows
/// top-down.
#[derive(Clone, Copy, Debug)]
pub struct Bitmap<'a> {
    pub w: i32,
    pub h: i32,
    pub bgra: &'a [u8],
}

impl Bitmap<'_> {
    fn valid(&self) -> bool {
        self.w > 0 && self.h > 0 && self.bgra.len() >= (self.w as usize) * (self.h as usize) * 4
    }
    fn px(&self, x: i32, y: i32) -> [u8; 4] {
        let at = ((y * self.w + x) as usize) * 4;
        [self.bgra[at], self.bgra[at + 1], self.bgra[at + 2], self.bgra[at + 3]]
    }
    /// The bitmap carries per-pixel alpha: its alpha bytes are not all the
    /// same (a plain DIB has 0 or 255 everywhere and is opaque).
    fn has_alpha(&self) -> bool {
        let n = (self.w as usize) * (self.h as usize);
        let first = self.bgra[3];
        self.bgra[..n * 4].chunks_exact(4).any(|p| p[3] != first)
    }
}

/// `COLORREF` `0x00BBGGRR` as `0xAARRGGBB` with alpha 255.
pub fn colorref_to_argb(c: u32) -> u32 {
    0xFF00_0000 | (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16 & 0xFF)
}

/// `0xAARRGGBB` as a `COLORREF`.
pub fn argb_to_colorref(p: u32) -> u32 {
    (p >> 16 & 0xFF) | (p & 0xFF00) | (p & 0xFF) << 16
}

/// The bits per pixel an image list of `flags` has (`ILC_COLOR`'s 0 gives
/// 4 bits as the oracle shows; `ILC_COLORDDB` is the display's, `display_bpp`).
pub fn color_depth(flags: u32, display_bpp: u32) -> u32 {
    match flags & 0xFE {
        ILC_COLORDDB => display_bpp,
        0 => 4,
        4 => 4,
        8 => 8,
        0x10 => 16,
        0x18 => 24,
        0x20 => 32,
        _ => 4,
    }
}

/// One image: `cx * cy` pixels, `0xAARRGGBB`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub argb: Vec<u32>,
}

/// An image list.
#[derive(Clone, Debug)]
pub struct ImageList {
    pub cx: i32,
    pub cy: i32,
    pub flags: u32,
    pub grow: i32,
    /// The images the list was made for (`ImageList_Create`'s `cInitial`).
    pub initial: i32,
    /// The rows of the image bitmap (`GetImageInfo`): four images wide, grown a `grow` ahead.
    pub rows: i32,
    pub bk: u32,
    pub images: Vec<Image>,
    /// `ImageList_SetOverlayImage`: overlay 1..=15 shows image `overlay[n - 1]` (-1: none).
    pub overlay: [i32; 15],
}

impl ImageList {
    /// `ImageList_Create`: `None` for a size that is not positive (or too big).
    pub fn new(cx: i32, cy: i32, flags: u32, initial: i32, grow: i32) -> Option<ImageList> {
        if cx <= 0 || cy <= 0 || cx > MAX_EDGE || cy > MAX_EDGE {
            return None;
        }
        let initial = initial.clamp(0, 1 << 16);
        Some(ImageList {
            cx,
            cy,
            flags,
            grow,
            initial,
            rows: ((initial + COLUMNS - 1) / COLUMNS).max(1),
            bk: CLR_NONE,
            images: Vec::new(),
            overlay: [-1; 15],
        })
    }

    /// The image bitmap has room for `n` images and `grow` more (rows are added when it is full:
    /// `ImageList_GetImageInfo.yaml` `ili_changes`: 5 images of a list that grows by 1 make two rows).
    fn room_for(&mut self, n: usize) {
        if n as i64 > i64::from(self.rows) * i64::from(COLUMNS) {
            self.rows = ((n as i32 + self.grow.max(1) + COLUMNS - 1) / COLUMNS).max(1);
        }
    }

    fn masked(&self) -> bool {
        self.flags & ILC_MASK != 0
    }

    fn blank(&self) -> Image {
        Image { argb: vec![0; (self.cx as usize) * (self.cy as usize)] }
    }

    pub fn count(&self) -> i32 {
        self.images.len() as i32
    }

    /// The image `index` of `color` (the strip), as pixels. `mask` makes
    /// its non-black pixels transparent when the list has a mask.
    fn cut(&self, color: &Bitmap, mask: Option<&Bitmap>, index: i32, key: Option<u32>) -> Image {
        let alpha = color.has_alpha();
        let mut out = Vec::with_capacity((self.cx as usize) * (self.cy as usize));
        for y in 0..self.cy {
            for x in 0..self.cx {
                let (sx, sy) = (index * self.cx + x, y);
                if sy >= color.h || sx >= color.w {
                    out.push(0);
                    continue;
                }
                let p = color.px(sx, sy);
                let mut a = if alpha { p[3] } else { 255 };
                if let Some(m) = mask
                    && self.masked()
                    && sx < m.w
                    && sy < m.h
                {
                    let q = m.px(sx, sy);
                    if q[0] != 0 || q[1] != 0 || q[2] != 0 {
                        a = 0;
                    }
                }
                if let Some(k) = key
                    && self.masked()
                    && u32::from(p[2]) | u32::from(p[1]) << 8 | u32::from(p[0]) << 16 == k & 0xFF_FFFF
                {
                    a = 0;
                }
                let rgb = if a == 0 { 0 } else { u32::from(p[2]) << 16 | u32::from(p[1]) << 8 | u32::from(p[0]) };
                out.push(u32::from(a) << 24 | rgb);
            }
        }
        Image { argb: out }
    }

    fn add_strip(&mut self, color: &Bitmap, mask: Option<&Bitmap>, key: Option<u32>) -> i32 {
        if !color.valid() {
            return -1;
        }
        let n = color.w / self.cx;
        if n <= 0 {
            return -1;
        }
        let first = self.count();
        for i in 0..n {
            let img = self.cut(color, mask, i, key);
            self.images.push(img);
            let len = self.images.len();
            self.room_for(len);
        }
        first
    }

    /// `ImageList_Add`: the index of the first image added, -1 on failure.
    pub fn add(&mut self, color: &Bitmap, mask: Option<&Bitmap>) -> i32 {
        self.add_strip(color, mask, None)
    }

    /// `ImageList_AddMasked`: pixels of the colour `key` (a `COLORREF`) are transparent.
    pub fn add_masked(&mut self, color: &Bitmap, key: u32) -> i32 {
        // A key that is CLR_NONE or CLR_DEFAULT is the colour of the bitmap's top-left pixel.
        let key = if (key == CLR_NONE || key == CLR_DEFAULT) && color.valid() {
            let p = color.px(0, 0);
            u32::from(p[2]) | u32::from(p[1]) << 8 | u32::from(p[0]) << 16
        } else {
            key
        };
        self.add_strip(color, None, Some(key))
    }

    /// `ImageList_Replace`: image `index` becomes the first image of the strip.
    pub fn replace(&mut self, index: i32, color: &Bitmap, mask: Option<&Bitmap>) -> bool {
        if index < 0 || index >= self.count() || !color.valid() || color.w < self.cx {
            return false;
        }
        self.images[index as usize] = self.cut(color, mask, 0, None);
        true
    }

    /// An icon's pixels stretched to the list's size: `color` is its colour
    /// bitmap, `mask` the AND mask (white: transparent); an icon whose
    /// colour bitmap has alpha keeps it.
    fn icon_image(&self, color: &Bitmap, mask: &Bitmap) -> Image {
        let alpha = color.has_alpha();
        let mut out = Vec::with_capacity((self.cx as usize) * (self.cy as usize));
        for y in 0..self.cy {
            for x in 0..self.cx {
                let sx = (i64::from(x) * i64::from(color.w) / i64::from(self.cx)) as i32;
                let sy = (i64::from(y) * i64::from(color.h) / i64::from(self.cy)) as i32;
                let p = color.px(sx, sy);
                let mut a = if alpha { p[3] } else { 255 };
                if mask.valid() && !alpha {
                    let mx = (i64::from(x) * i64::from(mask.w) / i64::from(self.cx)) as i32;
                    let my = (i64::from(y) * i64::from(mask.h) / i64::from(self.cy)) as i32;
                    let q = mask.px(mx.min(mask.w - 1), my.min(mask.h - 1));
                    if q[0] != 0 || q[1] != 0 || q[2] != 0 {
                        a = 0;
                    }
                }
                let rgb = if a == 0 { 0 } else { u32::from(p[2]) << 16 | u32::from(p[1]) << 8 | u32::from(p[0]) };
                out.push(u32::from(a) << 24 | rgb);
            }
        }
        Image { argb: out }
    }

    /// `ImageList_ReplaceIcon`: -1 appends; an index past the end fails (-1).
    pub fn replace_icon(&mut self, index: i32, color: &Bitmap, mask: &Bitmap) -> i32 {
        if !color.valid() || index < -1 || index >= self.count() {
            return -1;
        }
        let img = self.icon_image(color, mask);
        if index == -1 {
            self.images.push(img);
            let len = self.images.len();
            self.room_for(len);
            self.count() - 1
        } else {
            self.images[index as usize] = img;
            index
        }
    }

    /// `ImageList_Remove`: -1 empties the list.
    pub fn remove(&mut self, index: i32) -> bool {
        if index == -1 {
            self.images.clear();
            return true;
        }
        if index < 0 || index >= self.count() {
            return false;
        }
        self.images.remove(index as usize);
        true
    }

    /// `ImageList_SetImageCount`: new images are empty (transparent).
    pub fn set_count(&mut self, n: u32) -> bool {
        if n > 1 << 20 {
            return false;
        }
        let n = n as usize;
        if n < self.images.len() {
            self.images.truncate(n);
        } else {
            while self.images.len() < n {
                let b = self.blank();
                self.images.push(b);
            }
        }
        // The bitmap is made for the count and a `grow` more (and shrinks to it).
        self.rows = ((n as i32 + self.grow.max(1) + COLUMNS - 1) / COLUMNS).max(1);
        true
    }

    /// `ImageList_SetIconSize`: a new size removes every image.
    pub fn set_icon_size(&mut self, cx: i32, cy: i32) -> bool {
        if cx <= 0 || cy <= 0 || cx > MAX_EDGE || cy > MAX_EDGE {
            return false;
        }
        self.cx = cx;
        self.cy = cy;
        self.images.clear();
        self.rows = ((self.initial + COLUMNS - 1) / COLUMNS).max(1);
        true
    }

    /// `ImageList_SetBkColor`: the previous colour.
    pub fn set_bk(&mut self, c: u32) -> u32 {
        core::mem::replace(&mut self.bk, c)
    }

    /// `ImageList_GetImageRect`: where image `index` is in the image bitmap
    /// (a single row of `cx` wide cells).
    pub fn image_rect(&self, index: i32) -> Option<[i32; 4]> {
        if index < 0 || index >= self.count() {
            return None;
        }
        let (col, row) = (index % COLUMNS, index / COLUMNS);
        Some([col * self.cx, row * self.cy, (col + 1) * self.cx, (row + 1) * self.cy])
    }

    /// The image bitmap as `B, G, R, A` bytes: four images wide, `rows` high.
    pub fn strip_bgra(&self) -> (i32, i32, Vec<u8>) {
        let w = COLUMNS * self.cx;
        let h = self.rows.max((self.count() + COLUMNS - 1) / COLUMNS).max(1) * self.cy;
        let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
        for (i, img) in self.images.iter().enumerate() {
            let (ox, oy) = ((i as i32 % COLUMNS) * self.cx, (i as i32 / COLUMNS) * self.cy);
            for y in 0..self.cy as usize {
                for x in 0..self.cx as usize {
                    let p = img.argb[y * self.cx as usize + x];
                    let at = ((oy as usize + y) * w as usize + ox as usize + x) * 4;
                    out[at] = p as u8;
                    out[at + 1] = (p >> 8) as u8;
                    out[at + 2] = (p >> 16) as u8;
                    out[at + 3] = (p >> 24) as u8;
                }
            }
        }
        (w, h, out)
    }

    /// The mask strip, one `B, G, R, A` pixel per image pixel: white where transparent.
    pub fn mask_bgra(&self) -> (i32, i32, Vec<u8>) {
        let (w, h, mut px) = self.strip_bgra();
        for p in px.chunks_exact_mut(4) {
            let v = if p[3] < 128 { 255 } else { 0 };
            p.copy_from_slice(&[v, v, v, 0]);
        }
        (w, h, px)
    }

    /// An image as an icon needs it: colour `B, G, R, A` and the mask as
    /// `B, G, R, A` (white: transparent). `ILD_MASK` gives the mask as the picture.
    pub fn icon_parts(&self, index: i32, flags: u32) -> Option<(Vec<u8>, Vec<u8>)> {
        let img = self.images.get(usize::try_from(index).ok()?)?;
        let mut color = Vec::with_capacity(img.argb.len() * 4);
        let mut mask = Vec::with_capacity(img.argb.len() * 4);
        for &p in &img.argb {
            let transparent = p >> 24 < 128;
            let m = if transparent { 255 } else { 0 };
            mask.extend_from_slice(&[m, m, m, 0]);
            if flags & ILD_MASK != 0 {
                color.extend_from_slice(&[m, m, m, 255]);
            } else {
                color.extend_from_slice(&[p as u8, (p >> 8) as u8, (p >> 16) as u8, (p >> 24) as u8]);
            }
        }
        Some((color, mask))
    }

    /// `ImageList_Copy` inside one list: image `src` to image `dst` (or swapped).
    pub fn copy_within(&mut self, dst: i32, src: i32, flags: u32) -> bool {
        // Only ILCF_MOVE (0) and ILCF_SWAP (1) are flags (ImageList_Copy.yaml: others fail).
        if flags & !ILCF_SWAP != 0 {
            return false;
        }
        let n = self.count();
        if dst < 0 || src < 0 || dst >= n || src >= n {
            return false;
        }
        if flags & ILCF_SWAP != 0 {
            self.images.swap(dst as usize, src as usize);
        } else {
            let img = self.images[src as usize].clone();
            self.images[dst as usize] = img;
        }
        true
    }

    /// `ImageList_Copy` between two lists of the same image size.
    pub fn copy_from(&mut self, dst: i32, from: &mut ImageList, src: i32, flags: u32) -> bool {
        if (self.cx, self.cy) != (from.cx, from.cy) || dst < 0 || src < 0 || dst >= self.count() || src >= from.count()
        {
            return false;
        }
        if flags & ILCF_SWAP != 0 {
            core::mem::swap(&mut self.images[dst as usize], &mut from.images[src as usize]);
        } else {
            self.images[dst as usize] = from.images[src as usize].clone();
        }
        true
    }

    /// `ImageList_Merge`: a list of one image, `a`'s image `ia` with `b`'s
    /// image `ib` drawn over it at (`dx`, `dy`); the size is what both cover.
    pub fn merge(a: &ImageList, ia: i32, b: &ImageList, ib: i32, dx: i32, dy: i32) -> Option<ImageList> {
        let ia = a.images.get(usize::try_from(ia).ok()?)?;
        let ib = b.images.get(usize::try_from(ib).ok()?)?;
        let left = 0.min(dx);
        let top = 0.min(dy);
        let right = a.cx.max(dx + b.cx);
        let bottom = a.cy.max(dy + b.cy);
        let mut out = ImageList::new(right - left, bottom - top, a.flags, 1, a.grow)?;
        out.bk = a.bk;
        let mut img = out.blank();
        let w = out.cx as usize;
        for y in 0..a.cy {
            for x in 0..a.cx {
                img.argb[((y - top) as usize) * w + (x - left) as usize] = ia.argb[(y * a.cx + x) as usize];
            }
        }
        for y in 0..b.cy {
            for x in 0..b.cx {
                let p = ib.argb[(y * b.cx + x) as usize];
                if p >> 24 == 0 {
                    continue;
                }
                let at = ((y + dy - top) as usize) * w + (x + dx - left) as usize;
                img.argb[at] = over(p, img.argb[at]);
            }
        }
        out.images.push(img);
        Some(out)
    }

    /// What drawing image `index` does to pixel (`x`, `y`) of the image.
    ///
    /// `bk` is the background (`CLR_NONE` or `ILD_TRANSPARENT`: the destination
    /// shows through; `CLR_DEFAULT`: the list's), `fg` the blend colour for
    /// `ILD_BLEND25/50` (`CLR_DEFAULT`: the highlight colour `highlight`).
    #[allow(clippy::too_many_arguments)]
    pub fn shade(&self, index: i32, x: i32, y: i32, flags: u32, bk: u32, fg: u32, highlight: u32) -> Shade {
        let Some(img) = usize::try_from(index).ok().and_then(|i| self.images.get(i)) else {
            return Shade::Skip;
        };
        if x < 0 || y < 0 || x >= self.cx || y >= self.cy {
            return Shade::Skip;
        }
        let p = img.argb[(y * self.cx + x) as usize];
        let a = p >> 24;
        let bk = if bk == CLR_DEFAULT { self.bk } else { bk };
        let fill_bk = flags & ILD_TRANSPARENT == 0 && bk != CLR_NONE;
        if flags & ILD_MASK != 0 && self.masked() {
            // The mask goes over the destination with AND when there is no background (the opaque pixels
            // black, the rest left alone); with a background it is copied (the rest white).
            return if a >= 128 {
                Shade::Color(0)
            } else if fill_bk {
                Shade::Color(0xFF_FFFF)
            } else {
                Shade::Skip
            };
        }
        if flags & ILD_IMAGE != 0 {
            // The colour bitmap as it is: its transparent pixels are black.
            return Shade::Color(argb_to_colorref(p));
        }
        if a == 0 {
            return if fill_bk { Shade::Color(bk & 0xFF_FFFF) } else { Shade::Skip };
        }
        let c = argb_to_colorref(p);
        if flags & (ILD_BLEND25 | ILD_BLEND50) != 0 {
            // Blending dithers: every second pixel takes the blend colour (the highlight colour for
            // `CLR_DEFAULT`) and the others keep the image; with `CLR_NONE` there is no blend colour,
            // the image shows at the first kind only and the destination at the others.
            let even = (x + y) % 2 == 0;
            if fg == CLR_NONE {
                if !even {
                    return Shade::Skip;
                }
            } else if even {
                let b = if fg == CLR_DEFAULT { highlight } else { fg };
                return Shade::Color(b & 0xFF_FFFF);
            }
        }
        if a == 255 { Shade::Color(c) } else { Shade::Alpha(c, a as u8) }
    }

    /// The overlay image `n` (1..=15) shows, if set.
    pub fn overlay_index(&self, flags: u32) -> Option<i32> {
        let n = (flags & ILD_OVERLAYMASK) >> 8;
        if n == 0 || n > 15 {
            return None;
        }
        let i = self.overlay[(n - 1) as usize];
        if i < 0 { None } else { Some(i) }
    }

    /// `ImageList_SetOverlayImage`.
    pub fn set_overlay(&mut self, image: i32, overlay: i32) -> bool {
        if !(1..=15).contains(&overlay) || image < -1 || image >= self.count() {
            return false;
        }
        self.overlay[(overlay - 1) as usize] = image;
        true
    }
}

/// What to do with one destination pixel when drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shade {
    /// Leave it.
    Skip,
    /// Set it to this `COLORREF`.
    Color(u32),
    /// Blend this `COLORREF` over it with this alpha.
    Alpha(u32, u8),
}

impl Shade {
    /// Whether the caller has to read the destination pixel to finish.
    pub fn needs_dst(&self) -> bool {
        matches!(self, Shade::Alpha(..))
    }
    /// The pixel to write over `dst` (`COLORREF`), `None` to leave it.
    pub fn apply(&self, dst: u32) -> Option<u32> {
        match *self {
            Shade::Skip => None,
            Shade::Color(c) => Some(c),
            Shade::Alpha(c, a) => Some(mix(dst & 0xFF_FFFF, c, u32::from(a) * 100 / 255)),
        }
    }
}

/// `a` over `b`, both `0xAARRGGBB`; the result is opaque where `a` is.
fn over(a: u32, b: u32) -> u32 {
    let aa = a >> 24;
    if aa == 255 || b >> 24 == 0 {
        return a;
    }
    let ch = |s: u32, d: u32| (s * aa + d * (255 - aa)) / 255;
    let (ar, ag, ab) = (a >> 16 & 255, a >> 8 & 255, a & 255);
    let (br, bg, bb) = (b >> 16 & 255, b >> 8 & 255, b & 255);
    let alpha = aa + (b >> 24) * (255 - aa) / 255;
    alpha << 24 | ch(ar, br) << 16 | ch(ag, bg) << 8 | ch(ab, bb)
}

/// `COLORREF` `base` moved `pct` percent towards `toward`.
fn mix(base: u32, toward: u32, pct: u32) -> u32 {
    let ch = |shift: u32| {
        let b = base >> shift & 255;
        let t = toward >> shift & 255;
        (b * (100 - pct) + t * pct) / 100
    };
    ch(0) | ch(8) << 8 | ch(16) << 16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A top-down 32-bit strip of cells of `w` by `h`, one colour each (`0x00RRGGBB`).
    fn strip(w: i32, h: i32, colors: &[u32]) -> (i32, i32, Vec<u8>) {
        let tw = w * colors.len() as i32;
        let mut v = Vec::new();
        for _ in 0..h {
            for x in 0..tw {
                let c = colors[(x / w) as usize];
                v.extend_from_slice(&[c as u8, (c >> 8) as u8, (c >> 16) as u8, 0]);
            }
        }
        (tw, h, v)
    }

    fn bm(s: &(i32, i32, Vec<u8>)) -> Bitmap<'_> {
        Bitmap { w: s.0, h: s.1, bgra: &s.2 }
    }

    #[test]
    fn creation_rules() {
        assert!(ImageList::new(0, 0, ILC_COLOR32, 1, 1).is_none());
        assert!(ImageList::new(16, 0, ILC_COLOR32, 1, 1).is_none());
        assert!(ImageList::new(-1, -1, ILC_COLOR32, 1, 1).is_none());
        assert!(ImageList::new(1, 1, 0, 0, 0).is_some());
        assert!(ImageList::new(1000, 1000, 0, 0, -1).is_some());
        let l = ImageList::new(16, 16, ILC_COLOR32 | ILC_MASK, 2, 4).unwrap();
        assert_eq!((l.count(), l.bk), (0, CLR_NONE));
        assert_eq!(color_depth(0, 32), 4);
        assert_eq!(color_depth(ILC_COLOR24, 32), 24);
        assert_eq!(color_depth(ILC_COLORDDB, 32), 32);
    }

    #[test]
    fn add_counts_whole_images() {
        let mut l = ImageList::new(4, 4, ILC_COLOR32 | ILC_MASK, 1, 1).unwrap();
        let s3 = strip(4, 4, &[0xFF0000, 0x00FF00, 0x0000FF]);
        assert_eq!(l.add(&bm(&s3), None), 0);
        assert_eq!(l.count(), 3);
        let s5 = strip(2, 4, &[1, 2, 3, 4, 5]);
        assert_eq!(l.add(&bm(&s5), None), 3);
        assert_eq!(l.count(), 5);
        let tiny = strip(1, 1, &[7]);
        assert_eq!(l.add(&bm(&tiny), None), -1);
        assert_eq!(l.count(), 5);
        assert_eq!(l.add(&bm(&s3), Some(&bm(&s3))), 5);
        assert_eq!(l.count(), 8);
        // Without ILC_MASK the mask bitmap is ignored.
        let mut p = ImageList::new(4, 4, ILC_COLOR32, 1, 1).unwrap();
        assert_eq!(p.add(&bm(&s3), Some(&bm(&s3))), 0);
        assert_eq!(p.images[1].argb[0] >> 24, 255);
    }

    #[test]
    fn masked_add_makes_the_key_transparent() {
        let mut l = ImageList::new(4, 4, ILC_COLOR32 | ILC_MASK, 0, 1).unwrap();
        let s = strip(4, 4, &[0xFF0000, 0x00FF00, 0x0000FF]);
        // The key as a COLORREF (0x00BBGGRR): the green.
        assert_eq!(l.add_masked(&bm(&s), 0x00FF00), 0);
        assert_eq!(l.images[1].argb[0] >> 24, 0);
        assert_eq!(l.images[0].argb[0], 0xFFFF_0000);
        assert_eq!(l.images[2].argb[0], 0xFF00_00FF);
    }

    #[test]
    fn replace_icon_rules() {
        let mut l = ImageList::new(2, 2, ILC_COLOR32 | ILC_MASK, 1, 1).unwrap();
        let c = strip(2, 2, &[0x112233]);
        let m = strip(2, 2, &[0]);
        assert_eq!(l.replace_icon(-1, &bm(&c), &bm(&m)), 0);
        assert_eq!(l.replace_icon(-1, &bm(&c), &bm(&m)), 1);
        assert_eq!(l.replace_icon(0, &bm(&c), &bm(&m)), 0);
        assert_eq!(l.replace_icon(5, &bm(&c), &bm(&m)), -1);
        assert_eq!(l.replace_icon(2, &bm(&c), &bm(&m)), -1);
        assert_eq!(l.count(), 2);
    }

    #[test]
    fn remove_and_set_count() {
        let mut l = ImageList::new(2, 2, ILC_COLOR32, 1, 1).unwrap();
        assert!(l.set_count(5));
        assert_eq!(l.count(), 5);
        assert!(l.set_count(1));
        assert!(l.remove(0));
        assert!(!l.remove(7));
        assert!(l.set_count(3));
        assert!(l.remove(-1));
        assert_eq!(l.count(), 0);
        assert_eq!(l.image_rect(0), None);
        l.set_count(2);
        assert_eq!(l.image_rect(1), Some([2, 0, 4, 2]));
        l.set_count(6);
        assert_eq!(l.image_rect(5), Some([2, 2, 4, 4]));
    }

    #[test]
    fn the_image_bitmap_is_four_wide_and_grows_ahead() {
        // `ImageList_GetImageInfo.yaml` `ili_changes`: a list of 8x8 images that grows by 1.
        let mut l = ImageList::new(8, 8, ILC_COLOR32 | ILC_MASK, 1, 1).unwrap();
        let one = strip(8, 8, &[0xFF0000]);
        for _ in 0..5 {
            l.add(&bm(&one), None);
        }
        assert_eq!(l.strip_bgra().0, 32);
        assert_eq!((l.strip_bgra().1, l.rows), (16, 2));
        l.remove(1);
        assert_eq!(l.strip_bgra().1, 16, "removing does not shrink it");
        l.set_count(8);
        assert_eq!(l.strip_bgra().1, 24);
        l.set_count(2);
        assert_eq!(l.strip_bgra().1, 8);
        l.set_icon_size(16, 16);
        l.add(&bm(&strip(16, 16, &[0x0000FF])), None);
        assert_eq!((l.strip_bgra().0, l.strip_bgra().1), (64, 16));
    }

    #[test]
    fn bk_color_and_merge() {
        let mut l = ImageList::new(4, 4, ILC_COLOR32 | ILC_MASK, 1, 1).unwrap();
        assert_eq!(l.set_bk(0x112233), CLR_NONE);
        assert_eq!(l.set_bk(CLR_NONE), 0x112233);
        let s = strip(4, 4, &[0xFF0000, 0x00FF00]);
        l.add(&bm(&s), None);
        let m = ImageList::merge(&l, 0, &l, 1, 2, 1).unwrap();
        assert_eq!((m.cx, m.cy, m.count()), (6, 5, 1));
        // The second image covers where it is drawn.
        assert_eq!(m.images[0].argb[6 + 2], 0xFF00_FF00);
        assert_eq!(m.images[0].argb[0], 0xFFFF_0000);
    }

    #[test]
    fn copy_and_swap() {
        let mut l = ImageList::new(1, 1, ILC_COLOR32, 1, 1).unwrap();
        let s = strip(1, 1, &[0xFF0000, 0x00FF00]);
        l.add(&bm(&s), None);
        assert!(l.copy_within(0, 1, ILCF_SWAP));
        assert_eq!(l.images[0].argb[0], 0xFF00_FF00);
        assert!(l.copy_within(1, 0, 0));
        assert_eq!(l.images[1].argb[0], 0xFF00_FF00);
        assert!(!l.copy_within(1, 5, 0));
    }

    #[test]
    fn shading_for_drawing() {
        let mut l = ImageList::new(2, 1, ILC_COLOR32 | ILC_MASK, 1, 1).unwrap();
        let mut s = strip(2, 1, &[0x0000FF]);
        // The second pixel is the transparent key colour.
        s.2[4..8].copy_from_slice(&[0, 255, 0, 0]);
        l.add_masked(&bm(&s), 0x00FF00);
        // Opaque pixel: its colour as a COLORREF.
        assert_eq!(l.shade(0, 0, 0, ILD_NORMAL, CLR_NONE, CLR_NONE, 0), Shade::Color(0xFF_0000));
        // Transparent pixel: nothing with no background, the background with one.
        assert_eq!(l.shade(0, 1, 0, ILD_NORMAL, CLR_NONE, CLR_NONE, 0), Shade::Skip);
        assert_eq!(l.shade(0, 1, 0, ILD_NORMAL, 0x123456, CLR_NONE, 0), Shade::Color(0x123456));
        assert_eq!(l.shade(0, 1, 0, ILD_TRANSPARENT, 0x123456, CLR_NONE, 0), Shade::Skip);
        // The list's own background with CLR_DEFAULT.
        l.set_bk(0x00AA00);
        assert_eq!(l.shade(0, 1, 0, ILD_NORMAL, CLR_DEFAULT, CLR_NONE, 0), Shade::Color(0xAA00));
        // Blending dithers: the even pixels take the blend colour, the odd ones keep the image.
        assert_eq!(l.shade(0, 0, 0, ILD_BLEND50, CLR_NONE, 0xFF_FFFF, 0), Shade::Color(0xFF_FFFF));
        // The mask: black where the image is, the destination left alone (no background) or white (one).
        assert_eq!(l.shade(0, 0, 0, ILD_MASK, CLR_NONE, CLR_NONE, 0), Shade::Color(0));
        assert_eq!(l.shade(0, 1, 0, ILD_MASK, CLR_NONE, CLR_NONE, 0), Shade::Skip);
        assert_eq!(l.shade(0, 1, 0, ILD_MASK, 0x123456, CLR_NONE, 0), Shade::Color(0xFF_FFFF));
        // The bitmap as it is.
        assert_eq!(l.shade(0, 1, 0, ILD_IMAGE, CLR_NONE, CLR_NONE, 0), Shade::Color(0));
        assert_eq!(l.shade(5, 0, 0, ILD_NORMAL, CLR_NONE, CLR_NONE, 0), Shade::Skip);
    }

    #[test]
    fn icon_parts_and_overlays() {
        let mut l = ImageList::new(1, 1, ILC_COLOR32 | ILC_MASK, 1, 1).unwrap();
        l.set_count(1);
        let (c, m) = l.icon_parts(0, ILD_NORMAL).unwrap();
        assert_eq!(c, [0, 0, 0, 0]);
        assert_eq!(m, [255, 255, 255, 0]);
        assert!(l.icon_parts(1, 0).is_none());
        assert!(l.set_overlay(0, 1));
        assert!(!l.set_overlay(0, 16));
        assert_eq!(l.overlay_index(1 << 8), Some(0));
        assert_eq!(l.overlay_index(2 << 8), None);
    }
}
