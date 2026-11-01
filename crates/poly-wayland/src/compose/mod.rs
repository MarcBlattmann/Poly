//! The software compositor: a [`Scene`] of windows (positions, z-order,
//! the output's scale, Poly's neutral frame around toplevels) composed into
//! any [`Framebuffer`], redrawing only what changed.
//!
//! Pixels inside are `0xAARRGGBB` words with premultiplied alpha, the
//! layout of `wl_shm`'s `argb8888` (wayland.xml: "For all wl_shm formats
//! and unless specified in another protocol extension, pre-multiplied alpha
//! is used for pixel values"); a framebuffer holds `xrgb8888`, the format
//! virtio-gpu's `B8G8R8X8` resources and Linux's `XRGB8888` name too.
//!
//! Coordinates are logical (what clients and the frame are laid out in)
//! until rendering, which multiplies them by the output's integer scale
//! (HiDPI): a window whose buffer scale equals the output's is copied pixel
//! for pixel, others are scaled by nearest neighbour.

mod frame;
mod render;
mod scene;
mod text;

pub use frame::{BORDER, CLOSE_SIZE, TITLE_HEIGHT, Theme};
pub use scene::{Content, Hit, Scene, Window, WindowId};
pub use text::{BitmapFont, TextRenderer};

use alloc::vec;
use alloc::vec::Vec;

/// A rectangle: top-left corner and size (never negative).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect { x, y, width: if width < 0 { 0 } else { width }, height: if height < 0 { 0 } else { height } }
    }

    pub const fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// One past the rightmost column.
    pub const fn right(&self) -> i32 {
        self.x.saturating_add(self.width)
    }

    /// One past the bottom row.
    pub const fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height)
    }

    fn from_edges(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect::new(left, top, right.saturating_sub(left), bottom.saturating_sub(top))
    }

    /// The part both cover (empty if none).
    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect::from_edges(self.x.max(o.x), self.y.max(o.y), self.right().min(o.right()), self.bottom().min(o.bottom()))
    }

    /// The smallest rectangle covering both (an empty one counts for nothing).
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        Rect::from_edges(self.x.min(o.x), self.y.min(o.y), self.right().max(o.right()), self.bottom().max(o.bottom()))
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x.saturating_add(dx), self.y.saturating_add(dy), self.width, self.height)
    }

    /// Every coordinate multiplied by `s`.
    pub fn scale(&self, s: i32) -> Rect {
        Rect::new(
            self.x.saturating_mul(s),
            self.y.saturating_mul(s),
            self.width.saturating_mul(s),
            self.height.saturating_mul(s),
        )
    }

    /// Divided by `s`, rounded outwards (buffer pixels to logical ones).
    pub fn scale_down_out(&self, s: i32) -> Rect {
        if s <= 1 {
            return *self;
        }
        Rect::from_edges(
            self.x.div_euclid(s),
            self.y.div_euclid(s),
            self.right().saturating_add(s - 1).div_euclid(s),
            self.bottom().saturating_add(s - 1).div_euclid(s),
        )
    }
}

/// Where composed frames go: rows of `xrgb8888` pixels. The kernel's
/// display (virtio-gpu's scanout resource, a boot framebuffer) and the
/// off-screen [`MemoryFramebuffer`] implement it.
pub trait Framebuffer {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    /// Row `y` (below [`Framebuffer::height`]): at least
    /// [`Framebuffer::width`] pixels.
    fn row_mut(&mut self, y: u32) -> &mut [u32];
    /// The rectangles drawn since the last call are complete: show them
    /// (virtio-gpu: transfer to the host and flush). Nothing by default.
    fn present(&mut self, _damage: &[Rect]) {}
}

/// A framebuffer in memory: the compositor's off-screen target until the
/// display driver's interface is there, and what the tests render into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryFramebuffer {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
    /// Every rectangle presented, for tests and statistics.
    presented: Vec<Rect>,
}

impl MemoryFramebuffer {
    pub fn new(width: u32, height: u32) -> MemoryFramebuffer {
        MemoryFramebuffer { width, height, pixels: vec![0; width as usize * height as usize], presented: Vec::new() }
    }

    /// The pixel at (`x`, `y`) as `0xRRGGBB`.
    pub fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.pixels[(y * self.width + x) as usize] & 0x00ff_ffff)
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// The rectangles presented since the last call.
    pub fn take_presented(&mut self) -> Vec<Rect> {
        core::mem::take(&mut self.presented)
    }

    /// The picture as a binary PPM (`P6`, netpbm's portable pixmap).
    pub fn to_ppm(&self) -> Vec<u8> {
        let mut out = alloc::format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        out.reserve(self.pixels.len() * 3);
        for p in &self.pixels {
            out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
        out
    }
}

impl Framebuffer for MemoryFramebuffer {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn row_mut(&mut self, y: u32) -> &mut [u32] {
        let w = self.width as usize;
        let start = y as usize * w;
        &mut self.pixels[start..start + w]
    }

    fn present(&mut self, damage: &[Rect]) {
        self.presented.extend_from_slice(damage);
    }
}

/// `src` (premultiplied) over `dst`: `src + dst * (1 - alpha)`, per channel,
/// rounded to the nearest value. The result is opaque if `dst` was.
pub fn over(src: u32, dst: u32) -> u32 {
    let a = src >> 24;
    if a == 0xff {
        return src;
    }
    if a == 0 && src == 0 {
        return dst;
    }
    let inv = 255 - a;
    let channel = |shift: u32| {
        let s = (src >> shift) & 0xff;
        let d = (dst >> shift) & 0xff;
        (s + (d * inv + 127) / 255).min(255) << shift
    };
    channel(24) | channel(16) | channel(8) | channel(0)
}

/// The opaque colour `rgb` with coverage `alpha`, premultiplied.
pub fn with_alpha(rgb: u32, alpha: u8) -> u32 {
    let a = u32::from(alpha);
    let channel = |shift: u32| ((((rgb >> shift) & 0xff) * a + 127) / 255) << shift;
    a << 24 | channel(16) | channel(8) | channel(0)
}

#[cfg(test)]
mod tests;
