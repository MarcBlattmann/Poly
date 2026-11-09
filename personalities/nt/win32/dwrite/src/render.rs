//! Rendering parameters, glyph run analysis (`IDWriteGlyphRunAnalysis`),
//! glyph run outlines, the recommended rendering mode and the
//! GDI-compatible metrics: the COM glue over `poly_dwrite::render`.

use alloc::vec::Vec;
use core::ffi::c_void;

use poly_dwrite::face::GlyphMetrics;
use poly_dwrite::render::{self as r, Kind, Matrix, Polys, Raster};
use poly_font::{Font, Segment};

use crate::abi::*;
use crate::com::*;
use crate::face::{face_ref, is_face, write_glyph_metrics, write_metrics, write_metrics_1};

// ----- rendering parameters

#[repr(C)]
pub struct Params {
    head: Head,
    gamma: f32,
    contrast: f32,
    gray_contrast: f32,
    level: f32,
    geometry: u32,
    mode: u32,
    grid_fit: u32,
}

static PARAMS_IIDS: [GUID; 4] =
    [IID_IDWriteRenderingParams, IID_IDWriteRenderingParams1, IID_IDWriteRenderingParams2, IID_IDWriteRenderingParams3];

static PARAMS_VTBL: IDWriteRenderingParams3Vtbl = IDWriteRenderingParams3Vtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    GetGamma: p_gamma,
    GetEnhancedContrast: p_contrast,
    GetClearTypeLevel: p_level,
    GetPixelGeometry: p_geometry,
    GetRenderingMode: p_mode,
    GetGrayscaleEnhancedContrast: p_gray_contrast,
    GetGridFitMode: p_grid_fit,
    GetRenderingMode1: p_mode,
};

unsafe extern "system" fn qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded unchanged.
    unsafe { query_interface(this, riid, out) }
}

unsafe fn pm<'a>(this: *mut c_void) -> &'a Params {
    // SAFETY: our parameters.
    unsafe { &*(this as *const Params) }
}

unsafe extern "system" fn p_gamma(this: *mut c_void) -> f32 {
    // SAFETY: our parameters.
    unsafe { pm(this).gamma }
}
unsafe extern "system" fn p_contrast(this: *mut c_void) -> f32 {
    // SAFETY: our parameters.
    unsafe { pm(this).contrast }
}
unsafe extern "system" fn p_gray_contrast(this: *mut c_void) -> f32 {
    // SAFETY: our parameters.
    unsafe { pm(this).gray_contrast }
}
unsafe extern "system" fn p_level(this: *mut c_void) -> f32 {
    // SAFETY: our parameters.
    unsafe { pm(this).level }
}
unsafe extern "system" fn p_geometry(this: *mut c_void) -> u32 {
    // SAFETY: our parameters.
    unsafe { pm(this).geometry }
}
unsafe extern "system" fn p_mode(this: *mut c_void) -> u32 {
    // SAFETY: our parameters.
    unsafe { pm(this).mode }
}
unsafe extern "system" fn p_grid_fit(this: *mut c_void) -> u32 {
    // SAFETY: our parameters.
    unsafe { pm(this).grid_fit }
}

fn new_params(
    gamma: f32,
    contrast: f32,
    gray_contrast: f32,
    level: f32,
    geometry: u32,
    mode: u32,
    grid_fit: u32,
) -> *mut c_void {
    into_raw(Params {
        head: Head::new(&PARAMS_VTBL as *const _ as *const c_void, drop_box::<Params>, &PARAMS_IIDS),
        gamma,
        contrast,
        gray_contrast,
        level,
        geometry,
        mode,
        grid_fit,
    }) as *mut c_void
}

pub unsafe extern "system" fn create_rendering_params(_this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: output writable.
    unsafe { put(out, new_params(1.8, 0.5, 0.5, 1.0, 1, 0, 0)) };
    S_OK
}

pub unsafe extern "system" fn create_monitor_rendering_params(
    _this: *mut c_void,
    _monitor: *mut c_void,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { create_rendering_params(_this, out) }
}

#[allow(clippy::too_many_arguments)]
unsafe fn custom(
    out: *mut c_void,
    gamma: f32,
    contrast: f32,
    gray: f32,
    level: f32,
    geometry: u32,
    mode: u32,
    grid: u32,
    max_mode: u32,
) -> HRESULT {
    // SAFETY: output writable.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        // `!r::positive(x)` is also true for NaN.
        if !r::positive(gamma)
            || !r::non_negative(contrast)
            || !r::non_negative(gray)
            || !r::non_negative(level)
            || geometry > 2
            || mode > max_mode
            || grid > 2
        {
            return E_INVALIDARG;
        }
        put(out, new_params(gamma, contrast, gray, level, geometry, mode, grid));
    }
    S_OK
}

pub unsafe extern "system" fn create_custom_rendering_params(
    _this: *mut c_void,
    gamma: f32,
    contrast: f32,
    level: f32,
    geometry: u32,
    mode: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { custom(out, gamma, contrast, contrast, level, geometry, mode, 0, 6) }
}

pub unsafe extern "system" fn create_custom_rendering_params_1(
    _this: *mut c_void,
    gamma: f32,
    contrast: f32,
    gray: f32,
    level: f32,
    geometry: u32,
    mode: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { custom(out, gamma, contrast, gray, level, geometry, mode, 0, 6) }
}

pub unsafe extern "system" fn create_custom_rendering_params_2(
    _this: *mut c_void,
    gamma: f32,
    contrast: f32,
    gray: f32,
    level: f32,
    geometry: u32,
    mode: u32,
    grid: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { custom(out, gamma, contrast, gray, level, geometry, mode, grid, 7) }
}

// ----- glyph run analysis

#[repr(C)]
struct Analysis {
    head: Head,
    raster: Raster,
}

static ANALYSIS_IIDS: [GUID; 1] = [IID_IDWriteGlyphRunAnalysis];

static ANALYSIS_VTBL: IDWriteGlyphRunAnalysisVtbl = IDWriteGlyphRunAnalysisVtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    GetAlphaTextureBounds: a_bounds,
    CreateAlphaTexture: a_texture,
    GetAlphaBlendParams: a_blend,
};

/// `DWRITE_GLYPH_RUN` (`dwrite.h`).
#[repr(C)]
struct GlyphRun {
    face: *mut c_void,
    em_size: f32,
    count: u32,
    indices: *const u16,
    advances: *const f32,
    offsets: *const f32,
    sideways: i32,
    bidi: u32,
}

/// `DWRITE_MATRIX` read from a caller's pointer.
unsafe fn read_matrix(p: *const c_void) -> Matrix {
    if p.is_null() {
        return Matrix::IDENTITY;
    }
    // SAFETY: the caller passes a `DWRITE_MATRIX`.
    let m = unsafe { core::slice::from_raw_parts(p as *const f32, 6) };
    Matrix { m11: m[0], m12: m[1], m21: m[2], m22: m[3], dx: m[4], dy: m[5] }
}

/// Everything of a run as flattened polygons, in device pixels.
unsafe fn run_polys(run: &GlyphRun, ppd: f32, m: &Matrix, origin: (f32, f32)) -> Result<Polys, HRESULT> {
    // SAFETY: the run's pointers were given by the caller.
    unsafe {
        if !is_face(run.face) {
            return Err(E_INVALIDARG);
        }
        let face = face_ref(run.face);
        let Ok(font) = Font::parse_index(&face.bytes, face.index) else { return Err(DWRITE_E_FILEFORMAT) };
        let em_px = run.em_size * ppd;
        let glyphs: &[u16] = slice(run.indices as *const c_void, run.count);
        let adv: Option<Vec<f32>> = if run.advances.is_null() {
            None
        } else {
            Some(slice::<f32>(run.advances as *const c_void, run.count).iter().map(|a| a * ppd).collect())
        };
        let offs: Option<Vec<(f32, f32)>> = if run.offsets.is_null() {
            None
        } else {
            Some(
                slice::<f32>(run.offsets as *const c_void, run.count * 2)
                    .chunks_exact(2)
                    .map(|c| (c[0] * ppd, c[1] * ppd))
                    .collect(),
            )
        };
        let placed = r::place_run(&font, em_px, glyphs, adv.as_deref(), offs.as_deref(), run.bidi & 1 != 0);
        let mut polys = Polys::default();
        let total = Matrix { dx: m.dx + origin.0 * ppd, dy: m.dy + origin.1 * ppd, ..*m };
        for p in placed {
            let mut segs = r::glyph_segments(&font, &face.info, p.glyph, em_px, run.sideways != 0);
            if face.sims & 2 != 0 {
                r::shear(&mut segs, 0.25);
            }
            r::flatten(&segs, p.x, p.y, &total, &mut polys);
            if face.sims & 1 != 0 {
                // The bold simulation: the outline smeared by 1/32 em.
                r::flatten(&segs, p.x + em_px / 32.0, p.y, &total, &mut polys);
            }
        }
        Ok(polys)
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn create_analysis(
    run: *mut c_void,
    ppd: f32,
    matrix: *mut c_void,
    rendering: u32,
    measuring: u32,
    gray: bool,
    origin: (f32, f32),
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: the run and matrix are the caller's.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        if run.is_null() || rendering > 5 || measuring > 2 || !r::positive(ppd) {
            return E_INVALIDARG;
        }
        let run = &*(run as *const GlyphRun);
        if !r::non_negative(run.em_size) || !is_face(run.face) {
            return E_INVALIDARG;
        }
        let kind = if rendering == r::mode::ALIASED {
            Kind::Aliased
        } else if gray {
            Kind::Gray
        } else if rendering == r::mode::GDI_CLASSIC || rendering == r::mode::GDI_NATURAL {
            Kind::ClearType { lines: 1 }
        } else {
            Kind::ClearType { lines: 16 }
        };
        let raster = if run.em_size == 0.0 || run.count == 0 {
            Raster::empty(kind)
        } else {
            let polys = match run_polys(run, ppd, &read_matrix(matrix), origin) {
                Ok(p) => p,
                Err(hr) => return hr,
            };
            r::rasterize(&polys, kind)
        };
        put(
            out,
            into_raw(Analysis {
                head: Head::new(&ANALYSIS_VTBL as *const _ as *const c_void, drop_box::<Analysis>, &ANALYSIS_IIDS),
                raster,
            }),
        );
    }
    S_OK
}

pub unsafe extern "system" fn create_glyph_run_analysis(
    _this: *mut c_void,
    run: *mut c_void,
    ppd: f32,
    matrix: *mut c_void,
    rendering: u32,
    measuring: u32,
    bx: f32,
    by: f32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { create_analysis(run, ppd, matrix, rendering, measuring, false, (bx, by), out) }
}

pub unsafe extern "system" fn create_glyph_run_analysis_2(
    _this: *mut c_void,
    run: *mut c_void,
    matrix: *mut c_void,
    rendering: u32,
    measuring: u32,
    _grid_fit: u32,
    antialias: u32,
    bx: f32,
    by: f32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { create_analysis(run, 1.0, matrix, rendering, measuring, antialias == 1, (bx, by), out) }
}

unsafe fn an<'a>(this: *mut c_void) -> &'a Analysis {
    // SAFETY: our analysis.
    unsafe { &*(this as *const Analysis) }
}

/// Does a texture type match what the analysis rendered? `None` for an
/// invalid type.
fn texture_matches(kind: Kind, t: u32) -> Option<bool> {
    match t {
        0 => Some(matches!(kind, Kind::Aliased | Kind::Gray)),
        1 => Some(matches!(kind, Kind::ClearType { .. })),
        _ => None,
    }
}

unsafe fn write_rect(out: *mut c_void, rect: [i32; 4]) {
    if !out.is_null() {
        // SAFETY: the caller passes a RECT.
        unsafe { core::ptr::copy_nonoverlapping(rect.as_ptr(), out as *mut i32, 4) };
    }
}

unsafe extern "system" fn a_bounds(this: *mut c_void, t: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: our analysis.
    unsafe {
        let a = an(this);
        match texture_matches(a.raster.kind, t) {
            None => {
                write_rect(out, [0; 4]);
                E_INVALIDARG
            }
            Some(true) => {
                write_rect(out, a.raster.rect);
                S_OK
            }
            Some(false) => {
                write_rect(out, [0; 4]);
                S_OK
            }
        }
    }
}

unsafe extern "system" fn a_texture(
    this: *mut c_void,
    t: u32,
    rect: *mut c_void,
    buf: *mut c_void,
    size: u32,
) -> HRESULT {
    // SAFETY: our analysis; the buffer holds `size` bytes.
    unsafe {
        let a = an(this);
        let Some(matches) = texture_matches(a.raster.kind, t) else { return E_INVALIDARG };
        if rect.is_null() || buf.is_null() {
            return E_INVALIDARG;
        }
        let rc = core::slice::from_raw_parts(rect as *const i32, 4);
        let rc = [rc[0], rc[1], rc[2], rc[3]];
        if rc[2] <= rc[0] || rc[3] <= rc[1] {
            return E_INVALIDARG;
        }
        let bpp = if t == 1 { 3 } else { 1 };
        let need = (rc[2] as i64 - rc[0] as i64) * (rc[3] as i64 - rc[1] as i64) * bpp;
        if need > i64::from(size) {
            return E_INSUFFICIENT_BUFFER;
        }
        let out = core::slice::from_raw_parts_mut(buf as *mut u8, size as usize);
        out.fill(0);
        if matches {
            let tex = a.raster.texture(rc);
            out[..tex.len()].copy_from_slice(&tex);
        }
    }
    S_OK
}

unsafe extern "system" fn a_blend(
    _this: *mut c_void,
    params: *mut c_void,
    gamma: *mut c_void,
    contrast: *mut c_void,
    level: *mut c_void,
) -> HRESULT {
    // SAFETY: the parameters are the caller's COM object (or null).
    unsafe {
        let (g, c, l) = if params.is_null() {
            (1.8, 0.5, 1.0)
        } else {
            let v = vtbl::<IDWriteRenderingParamsVtbl>(params);
            ((v.GetGamma)(params), (v.GetEnhancedContrast)(params), (v.GetClearTypeLevel)(params))
        };
        put_val(gamma, g);
        put_val(contrast, c);
        put_val(level, l);
    }
    S_OK
}

// ----- outlines

#[repr(C)]
struct SinkVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set_fill_mode: unsafe extern "system" fn(*mut c_void, u32),
    set_segment_flags: unsafe extern "system" fn(*mut c_void, u32),
    begin_figure: unsafe extern "system" fn(*mut c_void, u64, u32),
    add_lines: unsafe extern "system" fn(*mut c_void, *const f32, u32),
    add_beziers: unsafe extern "system" fn(*mut c_void, *const f32, u32),
    end_figure: unsafe extern "system" fn(*mut c_void, u32),
    close: unsafe extern "system" fn(*mut c_void) -> i32,
}

fn point_bits(x: f32, y: f32) -> u64 {
    u64::from(x.to_bits()) | (u64::from(y.to_bits()) << 32)
}

pub unsafe extern "system" fn glyph_run_outline(
    this: *mut c_void,
    em: f32,
    glyphs: *mut c_void,
    advances: *mut c_void,
    offsets: *mut c_void,
    count: u32,
    sideways: i32,
    rtl: i32,
    sink: *mut c_void,
) -> HRESULT {
    // SAFETY: our face; the arrays hold `count` elements; the sink is the caller's COM object.
    unsafe {
        if !r::positive(em) || sink.is_null() {
            return E_INVALIDARG;
        }
        let face = face_ref(this);
        let Ok(font) = Font::parse_index(&face.bytes, face.index) else { return DWRITE_E_FILEFORMAT };
        let sv = vtbl::<SinkVtbl>(sink);
        (sv.set_fill_mode)(sink, 1);
        let gl: &[u16] = slice(glyphs, count);
        let adv: Option<&[f32]> = if advances.is_null() { None } else { Some(slice(advances, count)) };
        let offs: Option<Vec<(f32, f32)>> = if offsets.is_null() {
            None
        } else {
            Some(slice::<f32>(offsets, count * 2).chunks_exact(2).map(|c| (c[0], c[1])).collect())
        };
        let placed = r::place_run(&font, em, gl, adv, offs.as_deref(), rtl != 0);
        for p in placed {
            let mut segs = r::glyph_segments(&font, &face.info, p.glyph, em, sideways != 0);
            if face.sims & 2 != 0 {
                r::shear(&mut segs, 0.25);
            }
            emit(&segs, p.x, p.y, sink, sv);
        }
    }
    S_OK
}

unsafe fn emit(segs: &[Segment], ox: f32, oy: f32, sink: *mut c_void, sv: &SinkVtbl) {
    let pt = |p: poly_font::Point| (p.x + ox, p.y + oy);
    let mut open = false;
    let mut start = (0.0, 0.0);
    let mut cur = (0.0f32, 0.0f32);
    let mut lines: Vec<f32> = Vec::new();
    let mut curves: Vec<f32> = Vec::new();
    // SAFETY: `sink` is a live COM sink; the point arrays outlive each call.
    let flush = |lines: &mut Vec<f32>, curves: &mut Vec<f32>| unsafe {
        if !lines.is_empty() {
            (sv.add_lines)(sink, lines.as_ptr(), (lines.len() / 2) as u32);
            lines.clear();
        }
        if !curves.is_empty() {
            (sv.add_beziers)(sink, curves.as_ptr(), (curves.len() / 6) as u32);
            curves.clear();
        }
    };
    for s in segs {
        // SAFETY: `sink` is a live COM sink.
        unsafe {
            match *s {
                Segment::MoveTo(p) => {
                    if open {
                        flush(&mut lines, &mut curves);
                        (sv.end_figure)(sink, 1);
                    }
                    start = pt(p);
                    cur = start;
                    (sv.begin_figure)(sink, point_bits(start.0, start.1), 0);
                    open = true;
                }
                Segment::LineTo(p) => {
                    if !curves.is_empty() {
                        flush(&mut lines, &mut curves);
                    }
                    cur = pt(p);
                    lines.push(cur.0);
                    lines.push(cur.1);
                }
                Segment::QuadTo(c, p) => {
                    if !lines.is_empty() {
                        flush(&mut lines, &mut curves);
                    }
                    let c = pt(c);
                    let e = pt(p);
                    let c1 = (cur.0 + 2.0 / 3.0 * (c.0 - cur.0), cur.1 + 2.0 / 3.0 * (c.1 - cur.1));
                    let c2 = (e.0 + 2.0 / 3.0 * (c.0 - e.0), e.1 + 2.0 / 3.0 * (c.1 - e.1));
                    curves.extend_from_slice(&[c1.0, c1.1, c2.0, c2.1, e.0, e.1]);
                    cur = e;
                }
                Segment::CubicTo(a, b, p) => {
                    if !lines.is_empty() {
                        flush(&mut lines, &mut curves);
                    }
                    let (a, b, e) = (pt(a), pt(b), pt(p));
                    curves.extend_from_slice(&[a.0, a.1, b.0, b.1, e.0, e.1]);
                    cur = e;
                }
                Segment::Close => {
                    if open {
                        // A final line back to the start is implied.
                        if lines.len() >= 2 && lines[lines.len() - 2] == start.0 && lines[lines.len() - 1] == start.1 {
                            lines.truncate(lines.len() - 2);
                        }
                        flush(&mut lines, &mut curves);
                        (sv.end_figure)(sink, 1);
                        open = false;
                    }
                }
            }
        }
    }
    if open {
        // SAFETY: `sink` is a live COM sink.
        unsafe {
            flush(&mut lines, &mut curves);
            (sv.end_figure)(sink, 1);
        }
    }
}

// ----- mode and GDI-compatible metrics

pub unsafe extern "system" fn recommended_mode(
    _this: *mut c_void,
    em: f32,
    ppd: f32,
    measuring: u32,
    params: *mut c_void,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: the parameters are the caller's COM object (or null).
    unsafe {
        if params.is_null() {
            put_val(out, 0u32);
            return E_INVALIDARG;
        }
        let pmode = {
            let v = vtbl::<IDWriteRenderingParamsVtbl>(params);
            (v.GetRenderingMode)(params)
        };
        match r::recommended_mode(em, ppd, measuring, pmode) {
            Some(m) => {
                put_val(out, m);
                S_OK
            }
            None => {
                put_val(out, 0u32);
                E_INVALIDARG
            }
        }
    }
}

pub unsafe extern "system" fn gdi_metrics(
    this: *mut c_void,
    em: f32,
    ppd: f32,
    transform: *mut c_void,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our face; the transform is the caller's.
    unsafe {
        if !r::positive(em) || !r::positive(ppd) {
            return E_INVALIDARG;
        }
        let ppem = em * ppd * read_matrix(transform).scale();
        write_metrics(out, &r::gdi_metrics(&face_ref(this).info.metrics, ppem));
    }
    S_OK
}

pub unsafe extern "system" fn gdi_glyph_metrics(
    this: *mut c_void,
    em: f32,
    ppd: f32,
    transform: *mut c_void,
    natural: i32,
    glyphs: *mut c_void,
    count: u32,
    out: *mut c_void,
    sideways: i32,
) -> HRESULT {
    // SAFETY: our face; arrays of `count` elements.
    unsafe {
        if !r::positive(em) || !r::positive(ppd) {
            return E_INVALIDARG;
        }
        let f = face_ref(this);
        let Some(sf) = f.sfnt() else { return DWRITE_E_FILEFORMAT };
        let ppem = em * ppd * read_matrix(transform).scale();
        let upem = f32::from(f.info.metrics.design_units_per_em);
        let to_design = |px: f32| r::round(px * upem / ppem);
        for (i, &g) in slice::<u16>(glyphs, count).iter().enumerate() {
            let d: GlyphMetrics = poly_dwrite::face::design_glyph_metrics(&sf, &f.info, g);
            let adv_exact = d.advance_width as f32 * ppem / upem;
            let adv_px = if natural != 0 { r::round(adv_exact) } else { r::ceil(adv_exact - 0.5) };
            let ink = (d.advance_width as i32 - d.left_side_bearing - d.right_side_bearing) as f32 * ppem / upem;
            let lsb_px = d.left_side_bearing as f32 * ppem / upem;
            let m = GlyphMetrics {
                left_side_bearing: to_design(lsb_px) as i32,
                advance_width: to_design(adv_px) as u32,
                right_side_bearing: to_design(adv_px - lsb_px - ink) as i32,
                ..d
            };
            let m = if sideways != 0 {
                let adv_h = r::round(d.advance_height as f32 * ppem / upem);
                GlyphMetrics {
                    advance_height: to_design(adv_h) as u32,
                    top_side_bearing: to_design(d.top_side_bearing as f32 * ppem / upem) as i32,
                    ..m
                }
            } else {
                m
            };
            write_glyph_metrics((out as *mut u8).add(28 * i) as *mut c_void, &m);
        }
    }
    S_OK
}

/// `IDWriteFontFace1::GetGdiCompatibleMetrics`: the same into a
/// `DWRITE_FONT_METRICS1`.
pub unsafe extern "system" fn gdi_metrics_1(
    this: *mut c_void,
    em: f32,
    ppd: f32,
    transform: *mut c_void,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our face; the transform is the caller's.
    unsafe {
        if !r::positive(em) || !r::positive(ppd) {
            return E_INVALIDARG;
        }
        let f = face_ref(this);
        let ppem = em * ppd * read_matrix(transform).scale();
        write_metrics_1(out, &f.info, &r::gdi_metrics(&f.info.metrics, ppem));
    }
    S_OK
}

/// `IDWriteFontFace1::GetGdiCompatibleGlyphAdvances`: advances in design
/// units, whole pixels at the size.
pub unsafe extern "system" fn gdi_glyph_advances(
    this: *mut c_void,
    em: f32,
    ppd: f32,
    transform: *mut c_void,
    natural: i32,
    _sideways: i32,
    count: u32,
    glyphs: *mut c_void,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our face; arrays of `count` elements.
    unsafe {
        if !r::positive(em) || !r::positive(ppd) {
            return E_INVALIDARG;
        }
        let f = face_ref(this);
        let Some(sf) = f.sfnt() else { return DWRITE_E_FILEFORMAT };
        let ppem = em * ppd * read_matrix(transform).scale();
        let upem = f32::from(f.info.metrics.design_units_per_em);
        for (i, &g) in slice::<u16>(glyphs, count).iter().enumerate() {
            let g = if g >= f.info.num_glyphs { 0 } else { g };
            let exact = f32::from(poly_dwrite::face::advance(&sf, g)) * ppem / upem;
            let px = if natural != 0 { r::round(exact) } else { r::ceil(exact - 0.5) };
            *(out as *mut i32).add(i) = r::round(px * upem / ppem) as i32;
        }
    }
    S_OK
}

/// `IDWriteFontFace1::GetRecommendedRenderingMode` (DPI form).
pub unsafe extern "system" fn recommended_mode_1(
    _this: *mut c_void,
    em: f32,
    dpi_x: f32,
    _dpi_y: f32,
    transform: *mut c_void,
    _sideways: i32,
    _outline_threshold: u32,
    measuring: u32,
    params: *mut c_void,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: the transform and parameters are the caller's.
    unsafe {
        let px = em * dpi_x / 96.0 * read_matrix(transform).scale();
        let pmode = if params.is_null() {
            0
        } else {
            let v = vtbl::<IDWriteRenderingParamsVtbl>(params);
            (v.GetRenderingMode)(params)
        };
        match r::recommended_mode(px, 1.0, measuring, pmode) {
            Some(m) => {
                put_val(out, m);
                S_OK
            }
            None => {
                put_val(out, 0u32);
                E_INVALIDARG
            }
        }
    }
}
