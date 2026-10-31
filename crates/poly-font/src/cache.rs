//! A glyph cache: rendered bitmaps keyed by font, size, glyph, subpixel
//! offset and mode, with a byte budget and least-recently-used eviction.

use crate::font::{Font, GlyphId};
use crate::math::floor;
use crate::render::{Bitmap, RenderMode, render_glyph};
use alloc::collections::BTreeMap;

/// How many subpixel positions a glyph is rendered at: pen positions are
/// rounded to quarters of a pixel.
pub const SUBPIXEL_STEPS: u8 = 4;

/// Split a pen x position into a whole pixel and a subpixel step
/// (`0..SUBPIXEL_STEPS`), rounding to the nearest step.
pub fn quantize(x: f32) -> (i32, u8) {
    let steps = f32::from(SUBPIXEL_STEPS);
    let q = floor(x * steps + 0.5);
    let whole = floor(q / steps);
    (whole as i32, (q - whole * steps) as u8)
}

/// What a cached bitmap is of. `font` is the caller's number for a font
/// (the cache does not look inside fonts to tell them apart); `size` is
/// in 1/64 pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphKey {
    pub font: u32,
    pub size: u32,
    pub glyph: GlyphId,
    pub subpixel: u8,
    pub mode: RenderMode,
}

impl GlyphKey {
    pub fn new(font: u32, size_px: f32, glyph: GlyphId, subpixel: u8, mode: RenderMode) -> GlyphKey {
        let size = (size_px.clamp(0.0, crate::render::MAX_SIZE_PX) * 64.0 + 0.5) as u32;
        GlyphKey { font, size, glyph, subpixel: subpixel % SUBPIXEL_STEPS, mode }
    }

    /// The pixel size the key stands for.
    pub fn size_px(&self) -> f32 {
        self.size as f32 / 64.0
    }
}

#[derive(Debug)]
struct Entry {
    bitmap: Bitmap,
    last_used: u64,
}

/// Hit and miss counts, for tuning the budget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

/// Rendered glyphs, up to a budget of bitmap bytes.
#[derive(Debug)]
pub struct GlyphCache {
    entries: BTreeMap<GlyphKey, Entry>,
    bytes: usize,
    budget: usize,
    clock: u64,
    stats: CacheStats,
}

/// What an entry costs beyond its pixels (key, bookkeeping).
const ENTRY_OVERHEAD: usize = 64;

impl GlyphCache {
    /// A cache holding up to about `budget` bytes of bitmaps.
    pub fn new(budget: usize) -> GlyphCache {
        GlyphCache { entries: BTreeMap::new(), bytes: 0, budget, clock: 0, stats: CacheStats::default() }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bytes held now.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Drop everything (for example after a font is unloaded).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Drop the glyphs of one font.
    pub fn remove_font(&mut self, font: u32) {
        let bytes = &mut self.bytes;
        self.entries.retain(|k, e| {
            let keep = k.font != font;
            if !keep {
                *bytes -= cost(&e.bitmap);
            }
            keep
        });
    }

    /// The cached bitmap for `key`, rendering it from `font` on a miss.
    pub fn get(&mut self, font: &Font<'_>, key: GlyphKey) -> &Bitmap {
        self.clock += 1;
        let clock = self.clock;
        if self.entries.contains_key(&key) {
            self.stats.hits += 1;
        } else {
            self.stats.misses += 1;
            let dx = f32::from(key.subpixel) / f32::from(SUBPIXEL_STEPS);
            let bitmap = render_glyph(font, key.glyph, key.size_px(), dx, key.mode);
            let c = cost(&bitmap);
            // Make room first (the new entry is never the one evicted; one
            // bigger than the whole budget is still kept until the next
            // insertion).
            while self.bytes + c > self.budget && !self.entries.is_empty() {
                self.evict_oldest();
            }
            self.bytes += c;
            self.entries.insert(key, Entry { bitmap, last_used: clock });
        }
        // The key was present or has just been inserted.
        let e = self.entries.entry(key).or_insert_with(|| Entry { bitmap: Bitmap::default(), last_used: clock });
        e.last_used = clock;
        &e.bitmap
    }

    fn evict_oldest(&mut self) {
        let oldest = self.entries.iter().min_by_key(|(_, e)| e.last_used).map(|(k, _)| *k);
        if let Some(k) = oldest
            && let Some(e) = self.entries.remove(&k)
        {
            self.bytes -= cost(&e.bitmap);
            self.stats.evictions += 1;
        }
    }
}

fn cost(b: &Bitmap) -> usize {
    b.data.len() + ENTRY_OVERHEAD
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfont;

    #[test]
    fn quantizes_to_quarter_pixels() {
        assert_eq!(quantize(0.0), (0, 0));
        assert_eq!(quantize(0.3), (0, 1));
        assert_eq!(quantize(0.9), (1, 0));
        assert_eq!(quantize(2.6), (2, 2));
        assert_eq!(quantize(-0.25), (-1, 3));
        assert_eq!(quantize(-0.1), (0, 0));
    }

    #[test]
    fn caches_and_evicts_least_recently_used() {
        let data = testfont::build();
        let font = Font::parse(&data).unwrap();
        let key = |g: u16, sub: u8| GlyphKey::new(0, 16.0, GlyphId(g), sub, RenderMode::Gray);
        let first = font_bitmap_cost(&font, key(1, 0));
        let mut cache = GlyphCache::new(first * 2 + 10);
        let a = cache.get(&font, key(1, 0)).clone();
        assert_eq!(a, render_glyph(&font, GlyphId(1), 16.0, 0.0, RenderMode::Gray));
        assert_eq!(cache.get(&font, key(1, 0)), &a);
        assert_eq!(cache.stats(), CacheStats { hits: 1, misses: 1, evictions: 0 });
        // A different subpixel offset is a different bitmap.
        let b = cache.get(&font, key(1, 2)).clone();
        assert_ne!(a, b);
        assert_eq!(cache.len(), 2);
        // Touch (1, 0), then a third entry evicts (1, 2), the older one.
        cache.get(&font, key(1, 0));
        cache.get(&font, key(1, 1));
        assert_eq!(cache.stats().evictions, 1);
        assert!(cache.entries.contains_key(&key(1, 0)));
        assert!(!cache.entries.contains_key(&key(1, 2)));
        assert!(cache.bytes() <= first * 2 + 10 + first);
        cache.remove_font(0);
        assert!(cache.is_empty());
        assert_eq!(cache.bytes(), 0);
    }

    fn font_bitmap_cost(font: &Font<'_>, key: GlyphKey) -> usize {
        cost(&render_glyph(font, key.glyph, key.size_px(), 0.0, key.mode))
    }
}
