//! `IDWriteFontFace` (and `IDWriteFontFace1`'s metrics).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;

use poly_dwrite::face::{Cmap, FaceInfo, GlyphMetrics, Metrics, design_glyph_metrics};
use poly_dwrite::model::FaceEntry;
use poly_dwrite::sfnt::{self, Sfnt};
use poly_dwrite::system::in_charset;

use crate::abi::*;
use crate::com::*;
use crate::file::{FileSource, new_font_file};

#[repr(C)]
pub struct Face {
    pub head: Head,
    pub factory: *mut c_void,
    pub src: Arc<FileSource>,
    pub index: u32,
    pub sims: u32,
    pub variant: u32,
    pub info: Arc<FaceInfo>,
    pub bytes: Arc<Vec<u8>>,
    pub symbol: bool,
    pub charset: Option<&'static [(u32, u32)]>,
    pub cff: bool,
}

impl Face {
    pub fn sfnt(&self) -> Option<Sfnt<'_>> {
        Sfnt::parse(&self.bytes, self.index)
    }
}

static IIDS: [GUID; 2] = [IID_IDWriteFontFace, IID_IDWriteFontFace1];

static VTBL: IDWriteFontFace1Vtbl = IDWriteFontFace1Vtbl {
    base: IDWriteFontFaceVtbl {
        QueryInterface: qi,
        AddRef: add_ref,
        Release: release,
        GetType: face_type,
        GetFiles: get_files,
        GetIndex: get_index,
        GetSimulations: get_simulations,
        IsSymbolFont: is_symbol,
        GetMetrics: get_metrics,
        GetGlyphCount: glyph_count,
        GetDesignGlyphMetrics: design_metrics,
        GetGlyphIndices: glyph_indices,
        TryGetFontTable: try_get_table,
        ReleaseFontTable: release_table,
        GetGlyphRunOutline: crate::render::glyph_run_outline,
        GetRecommendedRenderingMode: crate::render::recommended_mode,
        GetGdiCompatibleMetrics: crate::render::gdi_metrics,
        GetGdiCompatibleGlyphMetrics: crate::render::gdi_glyph_metrics,
    },
    GetMetrics_1: get_metrics_1,
    GetGdiCompatibleMetrics_1: crate::render::gdi_metrics_1,
    GetCaretMetrics: caret_metrics,
    GetUnicodeRanges: unicode_ranges,
    IsMonospacedFont: is_monospaced,
    GetDesignGlyphAdvances: design_advances,
    GetGdiCompatibleGlyphAdvances: crate::render::gdi_glyph_advances,
    GetKerningPairAdjustments: crate::stub!(),
    HasKerningPairs: zero_i32,
    GetRecommendedRenderingMode_1: crate::render::recommended_mode_1,
    GetVerticalGlyphVariants: crate::stub!(),
    HasVerticalGlyphVariants: zero_i32,
};

unsafe extern "system" fn zero_i32(_: *mut c_void) -> i32 {
    0
}

unsafe extern "system" fn qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded unchanged.
    unsafe { query_interface(this, riid, out) }
}

unsafe fn face_dtor(h: *mut Head) {
    // SAFETY: the last reference to a `Face`.
    unsafe {
        let f = &*(h as *const Face);
        crate::factory::forget_face(f.factory, h as *mut Face);
        com_release(f.factory);
        drop_box::<Face>(h);
    }
}

/// The face of `entry` with `sims`: the cached one while someone holds it.
pub fn get_face(
    factory: *mut c_void,
    src: &Arc<FileSource>,
    entry: &FaceEntry,
    sims: u32,
) -> Result<*mut c_void, HRESULT> {
    let key = crate::factory::FaceKey {
        loader: src.loader,
        key: src.key.clone(),
        index: entry.index,
        sims,
        variant: entry.variant,
    };
    if let Some(p) = crate::factory::find_face(factory, &key) {
        return Ok(p as *mut c_void);
    }
    let bytes = src.bytes()?;
    let cff = Sfnt::parse(&bytes, entry.index).is_some_and(|s| s.cff);
    let face = into_raw(Face {
        head: Head::new(&VTBL as *const _ as *const c_void, face_dtor, &IIDS),
        factory,
        src: src.clone(),
        index: entry.index,
        sims,
        variant: entry.variant,
        info: entry.info.clone(),
        bytes,
        symbol: entry.symbol.unwrap_or(entry.info.is_symbol),
        charset: entry.charset,
        cff,
    });
    // SAFETY: a live factory.
    unsafe { com_add_ref(factory) };
    crate::factory::remember_face(factory, key, face);
    Ok(face as *mut c_void)
}

/// What names a face: its file, index, simulations and variant.
pub unsafe fn identity(face: *mut c_void) -> Option<(Arc<FileSource>, u32, u32, u32)> {
    if face.is_null() {
        return None;
    }
    // SAFETY: the first word of a COM object is its vtable pointer.
    let v = unsafe { *(face as *const *const c_void) };
    if v != &VTBL as *const _ as *const c_void {
        return None;
    }
    // SAFETY: one of our faces.
    let f = unsafe { &*(face as *const Face) };
    Some((f.src.clone(), f.index, f.sims, f.variant))
}

pub unsafe fn face_ref<'a>(face: *mut c_void) -> &'a Face {
    // SAFETY: the caller passes one of our faces.
    unsafe { &*(face as *const Face) }
}

/// Is this pointer one of our faces?
pub unsafe fn is_face(face: *mut c_void) -> bool {
    // SAFETY: reading the first word of a COM object.
    !face.is_null() && unsafe { *(face as *const *const c_void) } == &VTBL as *const _ as *const c_void
}

/// Write a `DWRITE_FONT_METRICS`.
pub unsafe fn write_metrics(out: *mut c_void, m: &Metrics) {
    if out.is_null() {
        return;
    }
    // SAFETY: the caller passes a `DWRITE_FONT_METRICS` (20 bytes).
    unsafe {
        let p = out as *mut u16;
        *p = m.design_units_per_em;
        *p.add(1) = m.ascent;
        *p.add(2) = m.descent;
        *(p.add(3) as *mut i16) = m.line_gap;
        *p.add(4) = m.cap_height;
        *p.add(5) = m.x_height;
        *(p.add(6) as *mut i16) = m.underline_position;
        *p.add(7) = m.underline_thickness;
        *(p.add(8) as *mut i16) = m.strikethrough_position;
        *p.add(9) = m.strikethrough_thickness;
    }
}

/// Write a `DWRITE_GLYPH_METRICS`.
pub unsafe fn write_glyph_metrics(out: *mut c_void, g: &GlyphMetrics) {
    // SAFETY: the caller passes a `DWRITE_GLYPH_METRICS` (28 bytes).
    unsafe {
        let p = out as *mut i32;
        *p = g.left_side_bearing;
        *p.add(1) = g.advance_width as i32;
        *p.add(2) = g.right_side_bearing;
        *p.add(3) = g.top_side_bearing;
        *p.add(4) = g.advance_height as i32;
        *p.add(5) = g.bottom_side_bearing;
        *p.add(6) = g.vertical_origin_y;
    }
}

unsafe fn me<'a>(this: *mut c_void) -> &'a Face {
    // SAFETY: our face.
    unsafe { &*(this as *const Face) }
}

unsafe extern "system" fn face_type(this: *mut c_void) -> u32 {
    // SAFETY: our face.
    if unsafe { me(this).cff } { sfnt::face_type::CFF } else { sfnt::face_type::TRUETYPE }
}

unsafe extern "system" fn get_files(this: *mut c_void, n: *mut c_void, files: *mut c_void) -> HRESULT {
    // SAFETY: our face.
    unsafe {
        let f = me(this);
        if n.is_null() {
            return E_INVALIDARG;
        }
        if files.is_null() {
            *(n as *mut u32) = 1;
            return S_OK;
        }
        if *(n as *const u32) != 1 {
            return E_INVALIDARG;
        }
        put(files, new_font_file(f.src.clone()));
    }
    S_OK
}

unsafe extern "system" fn get_index(this: *mut c_void) -> u32 {
    // SAFETY: our face.
    unsafe { me(this).index }
}

unsafe extern "system" fn get_simulations(this: *mut c_void) -> u32 {
    // SAFETY: our face.
    unsafe { me(this).sims }
}

unsafe extern "system" fn is_symbol(this: *mut c_void) -> i32 {
    // SAFETY: our face.
    i32::from(unsafe { me(this).symbol })
}

unsafe extern "system" fn get_metrics(this: *mut c_void, out: *mut c_void) {
    // SAFETY: our face.
    unsafe { write_metrics(out, &me(this).info.metrics) };
}

unsafe extern "system" fn glyph_count(this: *mut c_void) -> u16 {
    // SAFETY: our face.
    unsafe { me(this).info.num_glyphs }
}

unsafe extern "system" fn design_metrics(
    this: *mut c_void,
    glyphs: *mut c_void,
    n: u32,
    out: *mut c_void,
    _sideways: i32,
) -> HRESULT {
    // SAFETY: our face; arrays of `n` elements.
    unsafe {
        let f = me(this);
        let Some(sf) = f.sfnt() else { return DWRITE_E_FILEFORMAT };
        let gs: &[u16] = slice(glyphs, n);
        for (i, &g) in gs.iter().enumerate() {
            let m = design_glyph_metrics(&sf, &f.info, g);
            write_glyph_metrics((out as *mut u8).add(28 * i) as *mut c_void, &m);
        }
    }
    S_OK
}

unsafe extern "system" fn design_advances(
    this: *mut c_void,
    n: u32,
    glyphs: *mut c_void,
    out: *mut c_void,
    _sideways: i32,
) -> HRESULT {
    // SAFETY: our face; arrays of `n` elements.
    unsafe {
        let f = me(this);
        let Some(sf) = f.sfnt() else { return DWRITE_E_FILEFORMAT };
        let gs: &[u16] = slice(glyphs, n);
        for (i, &g) in gs.iter().enumerate() {
            let g = if g >= f.info.num_glyphs { 0 } else { g };
            *(out as *mut i32).add(i) = i32::from(poly_dwrite::face::advance(&sf, g));
        }
    }
    S_OK
}

unsafe extern "system" fn glyph_indices(this: *mut c_void, cps: *mut c_void, n: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: our face; arrays of `n` elements.
    unsafe {
        let f = me(this);
        let cps: &[u32] = slice(cps, n);
        let Some(sf) = f.sfnt() else { return DWRITE_E_FILEFORMAT };
        let cmap = Cmap::of(&sf);
        for (i, &cp) in cps.iter().enumerate() {
            let g = if in_charset(f.charset, cp) { cmap.glyph(cp) } else { 0 };
            *(out as *mut u16).add(i) = g;
        }
    }
    S_OK
}

unsafe extern "system" fn try_get_table(
    this: *mut c_void,
    tag: u32,
    data: *mut c_void,
    size: *mut c_void,
    ctx: *mut c_void,
    exists: *mut c_void,
) -> HRESULT {
    // SAFETY: our face; outputs writable.
    unsafe {
        let f = me(this);
        put::<u8>(data, core::ptr::null_mut());
        put_val(size, 0u32);
        put::<c_void>(ctx, core::ptr::null_mut());
        put_val(exists, 0i32);
        let t = tag.to_le_bytes();
        if let Some(tb) = f.sfnt().and_then(|s| s.table(&t)) {
            put(data, tb.as_ptr() as *mut u8);
            put_val(size, tb.len() as u32);
            put_val(exists, 1i32);
        }
    }
    S_OK
}

unsafe extern "system" fn release_table(_: *mut c_void, _: *mut c_void) {}

unsafe extern "system" fn is_monospaced(this: *mut c_void) -> i32 {
    // SAFETY: our face.
    unsafe {
        let f = me(this);
        let Some(sf) = f.sfnt() else { return 0 };
        let post = sf.table(b"post");
        i32::from(post.and_then(|p| sfnt::be32(p, 12)).unwrap_or(0) != 0)
    }
}

#[allow(dead_code)]
fn _unused(_: &[u16]) {}

unsafe extern "system" fn unicode_ranges(
    this: *mut c_void,
    max: u32,
    ranges: *mut c_void,
    actual: *mut c_void,
) -> HRESULT {
    // SAFETY: our face; the output array holds `max` ranges.
    unsafe {
        let f = me(this);
        let Some(sf) = f.sfnt() else { return DWRITE_E_FILEFORMAT };
        let covered = Cmap::of(&sf).ranges();
        let all = match f.charset {
            None => covered,
            Some(set) => poly_dwrite::system::intersect(&covered, set),
        };
        put_val(actual, all.len() as u32);
        if ranges.is_null() || (max as usize) < all.len() {
            return E_INSUFFICIENT_BUFFER;
        }
        for (i, (a, b)) in all.iter().enumerate() {
            let p = (ranges as *mut u32).add(2 * i);
            *p = *a;
            *p.add(1) = *b;
        }
    }
    S_OK
}

/// Write a `DWRITE_FONT_METRICS1`: the metrics, then the font's box, the
/// subscript and superscript placement and whether the typographic metrics
/// are in use.
pub unsafe fn write_metrics_1(out: *mut c_void, info: &FaceInfo, m: &poly_dwrite::face::Metrics) {
    if out.is_null() {
        return;
    }
    // SAFETY: the caller passes a `DWRITE_FONT_METRICS1` (48 bytes).
    unsafe {
        write_metrics(out, m);
        let p = (out as *mut i16).add(10);
        for (i, v) in
            info.extra.glyph_box.iter().chain(&info.extra.subscript).chain(&info.extra.superscript).enumerate()
        {
            *p.add(i) = *v;
        }
        *(out as *mut i32).add(11) = i32::from(info.extra.has_typographic_metrics);
    }
}

unsafe extern "system" fn get_metrics_1(this: *mut c_void, out: *mut c_void) {
    // SAFETY: our face.
    unsafe {
        let f = me(this);
        write_metrics_1(out, &f.info, &f.info.metrics);
    }
}

unsafe extern "system" fn caret_metrics(this: *mut c_void, out: *mut c_void) {
    // SAFETY: our face; `out` is a `DWRITE_CARET_METRICS` (three `INT16`s).
    unsafe {
        let c = me(this).info.caret;
        let p = out as *mut i16;
        *p = c[0];
        *p.add(1) = c[1];
        *p.add(2) = c[2];
    }
}
