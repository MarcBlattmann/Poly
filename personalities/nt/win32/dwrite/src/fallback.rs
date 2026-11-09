//! Font fallback: `IDWriteFontFallback`, the builder, and the system
//! fallback (`IDWriteFactory2::GetSystemFontFallback`).
//!
//! A custom fallback maps code point ranges to families of a collection
//! with a scale; the first mapping that covers a character wins, and a
//! character whose target family lacks it (or has no mapping) gets a run of
//! its own length with no font. The system fallback tries the families
//! `poly_dwrite::system::fallback_families` lists for the character; with a
//! base collection and family it first tries that family.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;

use poly_dwrite::face::Cmap;
use poly_dwrite::model::{self, FontDesc};
use poly_dwrite::names::utf16;
use poly_dwrite::render as r;
use poly_dwrite::sfnt::Sfnt;
use poly_dwrite::system::{fallback_families, in_charset};
use poly_win32_gui::SpinLock;

use crate::abi::*;
use crate::coll::{CollData, Collection, collection_of, new_collection, new_font};
use crate::com::*;
use crate::strings::wstr;

unsafe extern "system" fn qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded unchanged.
    unsafe { query_interface(this, riid, out) }
}

#[derive(Clone)]
struct Mapping {
    ranges: Vec<(u32, u32)>,
    families: Vec<Vec<u16>>,
    /// Null: the system collection.
    collection: *mut c_void,
    scale: f32,
}

// SAFETY: the collection is a COM object kept alive by a reference.
unsafe impl Send for Mapping {}

impl Mapping {
    fn duplicate(&self) -> Mapping {
        // SAFETY: a live collection or null.
        unsafe { com_add_ref(self.collection) };
        self.clone()
    }
}

#[repr(C)]
pub struct Fallback {
    head: Head,
    factory: *mut c_void,
    /// `None`: the system fallback.
    mappings: Option<Vec<Mapping>>,
}

static FALLBACK_IIDS: [GUID; 1] = [IID_IDWriteFontFallback];

static FALLBACK_VTBL: IDWriteFontFallbackVtbl =
    IDWriteFontFallbackVtbl { QueryInterface: qi, AddRef: add_ref, Release: release, MapCharacters: map_characters };

unsafe fn fallback_dtor(h: *mut Head) {
    // SAFETY: the last reference to a `Fallback`.
    unsafe {
        let f = &*(h as *const Fallback);
        com_release(f.factory);
        if let Some(m) = &f.mappings {
            for x in m {
                com_release(x.collection);
            }
        }
        drop_box::<Fallback>(h);
    }
}

fn new_fallback(factory: *mut c_void, mappings: Option<Vec<Mapping>>) -> *mut c_void {
    // SAFETY: a live factory.
    unsafe { com_add_ref(factory) };
    into_raw(Fallback {
        head: Head::new(&FALLBACK_VTBL as *const _ as *const c_void, fallback_dtor, &FALLBACK_IIDS),
        factory,
        mappings,
    }) as *mut c_void
}

pub unsafe extern "system" fn get_system_font_fallback(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: a live factory; output writable.
    unsafe { put(out, new_fallback(this, None)) };
    S_OK
}

// ----- the builder

#[repr(C)]
pub struct FallbackBuilder {
    head: Head,
    factory: *mut c_void,
    mappings: SpinLock<Vec<Mapping>>,
}

static BUILDER_IIDS: [GUID; 1] = [IID_IDWriteFontFallbackBuilder];

static BUILDER_VTBL: IDWriteFontFallbackBuilderVtbl = IDWriteFontFallbackBuilderVtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    AddMapping: b_add_mapping,
    AddMappings: b_add_mappings,
    CreateFontFallback: b_create,
};

unsafe fn builder_dtor(h: *mut Head) {
    // SAFETY: the last reference to a builder.
    unsafe {
        let b = &*(h as *const FallbackBuilder);
        com_release(b.factory);
        b.mappings.with(|m| {
            for x in m.iter() {
                com_release(x.collection);
            }
        });
        drop_box::<FallbackBuilder>(h);
    }
}

pub unsafe extern "system" fn create_font_fallback_builder(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: a live factory; output writable.
    unsafe {
        com_add_ref(this);
        put(
            out,
            into_raw(FallbackBuilder {
                head: Head::new(&BUILDER_VTBL as *const _ as *const c_void, builder_dtor, &BUILDER_IIDS),
                factory: this,
                mappings: SpinLock::new(Vec::new()),
            }),
        );
    }
    S_OK
}

unsafe extern "system" fn b_add_mapping(
    this: *mut c_void,
    ranges: *mut c_void,
    n_ranges: u32,
    families: *mut c_void,
    n_families: u32,
    collection: *mut c_void,
    _locale: *mut c_void,
    _base_family: *mut c_void,
    scale: f32,
) -> HRESULT {
    // SAFETY: our builder; arrays of the stated lengths.
    unsafe {
        if n_ranges == 0 || n_families == 0 || ranges.is_null() || families.is_null() || !r::non_negative(scale) {
            return E_INVALIDARG;
        }
        let rs: &[u32] = slice(ranges, n_ranges * 2);
        let ranges: Vec<(u32, u32)> = rs.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let fams: Vec<Vec<u16>> = slice::<*const u16>(families, n_families)
            .iter()
            .map(|&p| if p.is_null() { Vec::new() } else { wstr(p).to_vec() })
            .collect();
        com_add_ref(collection);
        (*(this as *const FallbackBuilder))
            .mappings
            .with(|m| m.push(Mapping { ranges, families: fams, collection, scale }));
    }
    S_OK
}

unsafe extern "system" fn b_add_mappings(this: *mut c_void, fallback: *mut c_void) -> HRESULT {
    // SAFETY: our builder; `fallback` one of ours.
    unsafe {
        if fallback.is_null() || *(fallback as *const *const c_void) != &FALLBACK_VTBL as *const _ as *const c_void {
            return E_INVALIDARG;
        }
        let fb = &*(fallback as *const Fallback);
        let b = &*(this as *const FallbackBuilder);
        match &fb.mappings {
            Some(ms) => b.mappings.with(|m| m.extend(ms.iter().map(Mapping::duplicate))),
            None => {
                // The system fallback: its single mapping over everything.
                b.mappings.with(|m| {
                    m.push(Mapping {
                        ranges: alloc::vec![(0, 0x10FFFF)],
                        families: fallback_families(0x41).iter().map(|s| utf16(s)).collect(),
                        collection: core::ptr::null_mut(),
                        scale: 1.0,
                    })
                });
            }
        }
    }
    S_OK
}

unsafe extern "system" fn b_create(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our builder; output writable.
    unsafe {
        let b = &*(this as *const FallbackBuilder);
        let ms: Vec<Mapping> = b.mappings.with(|m| m.iter().map(Mapping::duplicate).collect());
        put(out, new_fallback(b.factory, Some(ms)));
    }
    S_OK
}

// ----- MapCharacters

#[repr(C)]
struct SourceVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    at: unsafe extern "system" fn(*mut c_void, u32, *mut *const u16, *mut u32) -> i32,
}

/// The `len` units of the text from `pos`.
unsafe fn read_text(src: *mut c_void, pos: u32, len: u32) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    let mut at = pos;
    // SAFETY: `src` is the application's text analysis source.
    unsafe {
        while (out.len() as u32) < len {
            let mut p: *const u16 = core::ptr::null();
            let mut n = 0u32;
            let hr = (vtbl::<SourceVtbl>(src).at)(src, at, &mut p, &mut n);
            if hr < 0 || p.is_null() || n == 0 {
                break;
            }
            let take = n.min(len - out.len() as u32);
            out.extend_from_slice(core::slice::from_raw_parts(p, take as usize));
            at += take;
        }
    }
    out
}

/// The code point at the start of `t` and its length in units.
fn first_char(t: &[u16]) -> (u32, usize) {
    match t {
        [] => (0, 0),
        [hi @ 0xD800..=0xDBFF, lo @ 0xDC00..=0xDFFF, ..] => {
            (0x10000 + ((u32::from(*hi) - 0xD800) << 10) + (u32::from(*lo) - 0xDC00), 2)
        }
        [c, ..] => (u32::from(*c), 1),
    }
}

/// A font found for a character: the collection, family and font.
struct Hit {
    data: Arc<CollData>,
    family: usize,
    desc: FontDesc,
    scale: f32,
    /// Which mappings cover the character (a run never spans a change).
    cover: Vec<(usize, usize)>,
}

fn covers(data: &CollData, desc: &FontDesc, cp: u32) -> bool {
    let e = &data.entries[desc.entry];
    if !in_charset(e.charset, cp) {
        return false;
    }
    let Ok(bytes) = data.srcs[desc.entry].bytes() else { return false };
    Sfnt::parse(&bytes, e.index).is_some_and(|sf| Cmap::of(&sf).glyph(cp) != 0)
}

fn find_in(data: &Arc<CollData>, family: &[u16], cp: u32, w: u32, s: u32, st: u32, scale: f32) -> Option<Hit> {
    let fi = model::find_family(&data.families, family)?;
    let desc = model::first_matching(&data.entries, &data.families[fi], w, s, st)?;
    covers(data, &desc, cp).then(|| Hit { data: data.clone(), family: fi, desc, scale, cover: Vec::new() })
}

unsafe fn lookup(
    fb: &Fallback,
    cp: u32,
    base: *mut c_void,
    base_family: &[u16],
    w: u32,
    s: u32,
    st: u32,
) -> Option<Hit> {
    // SAFETY: the base collection is the application's COM object or null.
    unsafe {
        match &fb.mappings {
            Some(ms) => {
                let cover: Vec<(usize, usize)> = ms
                    .iter()
                    .enumerate()
                    .filter_map(|(i, m)| m.ranges.iter().position(|&(a, b)| (a..=b).contains(&cp)).map(|r| (i, r)))
                    .collect();
                let m = &ms[cover.first()?.0];
                let data = if m.collection.is_null() {
                    crate::system::system_data()
                } else {
                    collection_of(m.collection)?.data.clone()
                };
                let mut hit = m.families.iter().find_map(|f| find_in(&data, f, cp, w, s, st, m.scale))?;
                hit.cover = cover;
                Some(hit)
            }
            None => {
                if let Some(c) = collection_of(base).filter(|_| !base_family.is_empty())
                    && let Some(h) = find_in(&c.data, base_family, cp, w, s, st, 1.0)
                {
                    return Some(h);
                }
                let data = crate::system::system_data();
                fallback_families(cp).iter().find_map(|f| find_in(&data, &utf16(f), cp, w, s, st, 1.0))
            }
        }
    }
}

unsafe extern "system" fn map_characters(
    this: *mut c_void,
    source: *mut c_void,
    pos: u32,
    len: u32,
    base: *mut c_void,
    base_family: *mut c_void,
    weight: u32,
    style: u32,
    stretch: u32,
    mapped_len: *mut c_void,
    mapped_font: *mut c_void,
    scale: *mut c_void,
) -> HRESULT {
    // SAFETY: our fallback; the source and outputs are the caller's.
    unsafe {
        put::<c_void>(mapped_font, core::ptr::null_mut());
        put_val(scale, 1.0f32);
        put_val(mapped_len, 0u32);
        if source.is_null() {
            return E_INVALIDARG;
        }
        let fb = &*(this as *const Fallback);
        let text = read_text(source, pos, len);
        let (cp, n) = first_char(&text);
        if n == 0 {
            return S_OK;
        }
        let bf: &[u16] = if base_family.is_null() { &[] } else { wstr(base_family as *const u16) };
        let first = lookup(fb, cp, base, bf, weight, stretch, style);
        let Some(hit) = first else {
            put_val(mapped_len, n as u32);
            return S_OK;
        };
        // Extend the run over the characters the same font covers.
        let mut total = n;
        while total < text.len() {
            let (c2, n2) = first_char(&text[total..]);
            if n2 == 0 || !covers(&hit.data, &hit.desc, c2) {
                break;
            }
            // The same mapping must send it to this font too.
            match lookup(fb, c2, base, bf, weight, stretch, style) {
                Some(h2)
                    if h2.desc.entry == hit.desc.entry
                        && h2.family == hit.family
                        && h2.scale == hit.scale
                        && h2.cover == hit.cover => {}
                _ => break,
            }
            total += n2;
        }
        let coll = new_collection(Collection_factory(fb), hit.data.clone());
        let font = new_font(coll, hit.family, hit.desc.clone());
        com_release(coll);
        put(mapped_font, font);
        put_val(mapped_len, total as u32);
        put_val(scale, hit.scale);
    }
    S_OK
}

#[allow(non_snake_case)]
fn Collection_factory(fb: &Fallback) -> *mut c_void {
    fb.factory
}

#[allow(dead_code)]
fn _unused(_: &Collection) {}
