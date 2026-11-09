//! Font collections, families, font lists and fonts
//! (`IDWriteFontCollection1`, `IDWriteFontFamily`, `IDWriteFontList`,
//! `IDWriteFont`).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;

use poly_dwrite::face::{Cmap, STYLE_NORMAL};
use poly_dwrite::model::{self, FaceEntry, Family, FontDesc};
use poly_dwrite::sfnt::Sfnt;
use poly_dwrite::system::in_charset;

use crate::abi::*;
use crate::com::*;
use crate::file::FileSource;
use crate::strings::{new_strings, wstr};

/// The immutable contents of a collection.
pub struct CollData {
    pub entries: Vec<FaceEntry>,
    pub srcs: Vec<Arc<FileSource>>,
    pub families: Vec<Family>,
}

// SAFETY: plain data; the sources are `Send + Sync`.
unsafe impl Send for CollData {}
// SAFETY: as above.
unsafe impl Sync for CollData {}

impl CollData {
    pub fn new(items: Vec<(FaceEntry, Arc<FileSource>)>) -> CollData {
        let (entries, srcs): (Vec<_>, Vec<_>) = items.into_iter().unzip();
        let families = model::build_families(&entries);
        CollData { entries, srcs, families }
    }
}

// ----- collection

#[repr(C)]
pub struct Collection {
    pub head: Head,
    pub factory: *mut c_void,
    pub data: Arc<CollData>,
}

static COLL_IIDS: [GUID; 2] = [IID_IDWriteFontCollection, IID_IDWriteFontCollection1];

static COLL_VTBL: IDWriteFontCollection1Vtbl = IDWriteFontCollection1Vtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    GetFontFamilyCount: coll_count,
    GetFontFamily: coll_family,
    FindFamilyName: coll_find,
    GetFontFromFontFace: coll_from_face,
    GetFontSet: coll_font_set,
    GetFontFamily_1: coll_family,
};

unsafe extern "system" fn qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded unchanged.
    unsafe { query_interface(this, riid, out) }
}

unsafe fn coll_dtor(h: *mut Head) {
    // SAFETY: the last reference to a `Collection`.
    unsafe {
        let c = &*(h as *const Collection);
        com_release(c.factory);
        drop_box::<Collection>(h);
    }
}

/// A new collection over `data` (reference count 1). `factory` gets a
/// reference.
pub fn new_collection(factory: *mut c_void, data: Arc<CollData>) -> *mut c_void {
    // SAFETY: a live factory.
    unsafe { com_add_ref(factory) };
    into_raw(Collection {
        head: Head::new(&COLL_VTBL as *const _ as *const c_void, coll_dtor, &COLL_IIDS),
        factory,
        data,
    }) as *mut c_void
}

pub unsafe fn collection_of<'a>(p: *mut c_void) -> Option<&'a Collection> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the first word of a COM object is its vtable pointer.
    let v = unsafe { *(p as *const *const c_void) };
    if v != &COLL_VTBL as *const _ as *const c_void {
        return None;
    }
    // SAFETY: one of our collections.
    Some(unsafe { &*(p as *const Collection) })
}

unsafe fn me<'a>(this: *mut c_void) -> &'a Collection {
    // SAFETY: our collection.
    unsafe { &*(this as *const Collection) }
}

unsafe extern "system" fn coll_count(this: *mut c_void) -> u32 {
    // SAFETY: our collection.
    unsafe { me(this).data.families.len() as u32 }
}

unsafe extern "system" fn coll_family(this: *mut c_void, i: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: our collection; output writable.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        if i as usize >= me(this).data.families.len() {
            return E_FAIL;
        }
        put(out, new_family(this, i as usize));
    }
    S_OK
}

unsafe extern "system" fn coll_find(
    this: *mut c_void,
    name: *mut c_void,
    index: *mut c_void,
    exists: *mut c_void,
) -> HRESULT {
    // SAFETY: our collection; `name` is NUL-terminated.
    unsafe {
        put_val(index, u32::MAX);
        put_val(exists, 0i32);
        if name.is_null() {
            return E_INVALIDARG;
        }
        if let Some(i) = model::find_family(&me(this).data.families, wstr(name as *const u16)) {
            put_val(index, i as u32);
            put_val(exists, 1i32);
        }
    }
    S_OK
}

unsafe extern "system" fn coll_from_face(this: *mut c_void, face: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our collection; `face` is a face of this DLL.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let c = me(this);
        let Some((src, index, sims, variant)) = crate::face::identity(face) else { return E_INVALIDARG };
        // Prefer the entry of the same variant.
        let mut found: Option<(usize, usize, usize)> = None;
        for pass in 0..2 {
            for (fi, fam) in c.data.families.iter().enumerate() {
                for (k, f) in fam.fonts.iter().enumerate() {
                    let e = &c.data.entries[f.entry];
                    if f.simulations == sims
                        && e.index == index
                        && c.data.srcs[f.entry].same(&src)
                        && (pass == 1 || e.variant == variant)
                    {
                        found = Some((fi, k, f.entry));
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        if found.is_none() && sims != 0 {
            // A simulation the family does not list: simulate the real font.
            for pass in 0..2 {
                for (fi, fam) in c.data.families.iter().enumerate() {
                    for f in fam.fonts.iter().filter(|f| f.simulations == 0) {
                        let e = &c.data.entries[f.entry];
                        if e.index == index && c.data.srcs[f.entry].same(&src) && (pass == 1 || e.variant == variant) {
                            put(out, new_font(this, fi, model::simulate(&c.data.entries, f, sims)));
                            return S_OK;
                        }
                    }
                }
            }
        }
        match found {
            Some((fi, k, _)) => {
                put(out, new_font(this, fi, c.data.families[fi].fonts[k].clone()));
                S_OK
            }
            None => DWRITE_E_NOFONT,
        }
    }
}

unsafe extern "system" fn coll_font_set(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our collection.
    unsafe {
        let c = me(this);
        put(out, crate::fontset::new_font_set_from_collection(c.factory, c.data.clone()));
    }
    S_OK
}

// ----- family and font list

#[repr(C)]
pub struct FamilyObj {
    pub head: Head,
    pub coll: *mut c_void,
    pub idx: usize,
    /// For a font list from `GetMatchingFonts`: the fonts in order.
    pub list: Option<Vec<FontDesc>>,
}

static FAMILY_IIDS: [GUID; 2] = [IID_IDWriteFontFamily, IID_IDWriteFontList];
static LIST_IIDS: [GUID; 1] = [IID_IDWriteFontList];

static FAMILY_VTBL: IDWriteFontFamilyVtbl = IDWriteFontFamilyVtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    GetFontCollection: fam_collection,
    GetFontCount: fam_count,
    GetFont: fam_font,
    GetFamilyNames: fam_names,
    GetFirstMatchingFont: fam_first,
    GetMatchingFonts: fam_matching,
};

static LIST_VTBL: IDWriteFontListVtbl = IDWriteFontListVtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    GetFontCollection: fam_collection,
    GetFontCount: fam_count,
    GetFont: fam_font,
};

unsafe fn fam_dtor(h: *mut Head) {
    // SAFETY: the last reference to a `FamilyObj`.
    unsafe {
        com_release((*(h as *const FamilyObj)).coll);
        drop_box::<FamilyObj>(h);
    }
}

fn new_family(coll: *mut c_void, idx: usize) -> *mut c_void {
    // SAFETY: a live collection.
    unsafe { com_add_ref(coll) };
    into_raw(FamilyObj {
        head: Head::new(&FAMILY_VTBL as *const _ as *const c_void, fam_dtor, &FAMILY_IIDS),
        coll,
        idx,
        list: None,
    }) as *mut c_void
}

fn new_list(coll: *mut c_void, idx: usize, fonts: Vec<FontDesc>) -> *mut c_void {
    // SAFETY: a live collection.
    unsafe { com_add_ref(coll) };
    into_raw(FamilyObj {
        head: Head::new(&LIST_VTBL as *const _ as *const c_void, fam_dtor, &LIST_IIDS),
        coll,
        idx,
        list: Some(fonts),
    }) as *mut c_void
}

unsafe fn fam<'a>(this: *mut c_void) -> (&'a FamilyObj, &'a Collection, &'a Family) {
    // SAFETY: our family object over a live collection.
    unsafe {
        let f = &*(this as *const FamilyObj);
        let c = &*(f.coll as *const Collection);
        (f, c, &c.data.families[f.idx])
    }
}

unsafe extern "system" fn fam_collection(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our family object.
    unsafe {
        let (f, _, _) = fam(this);
        com_add_ref(f.coll);
        put(out, f.coll);
    }
    S_OK
}

unsafe extern "system" fn fam_count(this: *mut c_void) -> u32 {
    // SAFETY: our family object.
    unsafe {
        let (f, _, family) = fam(this);
        f.list.as_ref().map_or(family.fonts.len(), Vec::len) as u32
    }
}

unsafe extern "system" fn fam_font(this: *mut c_void, i: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: our family object.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let (f, _, family) = fam(this);
        let desc = match &f.list {
            Some(l) => l.get(i as usize),
            None => family.fonts.get(i as usize),
        };
        let Some(d) = desc else { return E_FAIL };
        put(out, new_font(f.coll, f.idx, d.clone()));
    }
    S_OK
}

unsafe extern "system" fn fam_names(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our family object.
    unsafe {
        let (_, _, family) = fam(this);
        put(out, new_strings(family.names.clone()));
    }
    S_OK
}

unsafe extern "system" fn fam_first(
    this: *mut c_void,
    weight: u32,
    stretch: u32,
    style: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our family object.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let (f, c, family) = fam(this);
        match model::first_matching(&c.data.entries, family, weight, stretch, style) {
            Some(d) => {
                put(out, new_font(f.coll, f.idx, d));
                S_OK
            }
            None => E_FAIL,
        }
    }
}

unsafe extern "system" fn fam_matching(
    this: *mut c_void,
    weight: u32,
    stretch: u32,
    style: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our family object.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let (f, c, family) = fam(this);
        let fonts = model::matching_fonts(&c.data.entries, family, weight, stretch, style);
        put(out, new_list(f.coll, f.idx, fonts));
    }
    S_OK
}

// ----- font

#[repr(C)]
pub struct FontObj {
    pub head: Head,
    pub coll: *mut c_void,
    pub family: usize,
    pub desc: FontDesc,
}

static FONT_IIDS: [GUID; 1] = [IID_IDWriteFont];

static FONT_VTBL: IDWriteFontVtbl = IDWriteFontVtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    GetFontFamily: font_family,
    GetWeight: font_weight,
    GetStretch: font_stretch,
    GetStyle: font_style,
    IsSymbolFont: font_is_symbol,
    GetFaceNames: font_face_names,
    GetInformationalStrings: font_info_strings,
    GetSimulations: font_simulations,
    GetMetrics: font_metrics,
    HasCharacter: font_has_character,
    CreateFontFace: font_create_face,
};

unsafe fn font_dtor(h: *mut Head) {
    // SAFETY: the last reference to a `FontObj`.
    unsafe {
        com_release((*(h as *const FontObj)).coll);
        drop_box::<FontObj>(h);
    }
}

pub fn new_font(coll: *mut c_void, family: usize, desc: FontDesc) -> *mut c_void {
    // SAFETY: a live collection.
    unsafe { com_add_ref(coll) };
    into_raw(FontObj {
        head: Head::new(&FONT_VTBL as *const _ as *const c_void, font_dtor, &FONT_IIDS),
        coll,
        family,
        desc,
    }) as *mut c_void
}

unsafe fn fnt<'a>(this: *mut c_void) -> (&'a FontObj, &'a Collection) {
    // SAFETY: our font over a live collection.
    unsafe {
        let f = &*(this as *const FontObj);
        (f, &*(f.coll as *const Collection))
    }
}

unsafe extern "system" fn font_family(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our font.
    unsafe {
        let (f, _) = fnt(this);
        put(out, new_family(f.coll, f.family));
    }
    S_OK
}

unsafe extern "system" fn font_weight(this: *mut c_void) -> u32 {
    // SAFETY: our font.
    unsafe { fnt(this).0.desc.weight }
}

unsafe extern "system" fn font_stretch(this: *mut c_void) -> u32 {
    // SAFETY: our font.
    unsafe { fnt(this).0.desc.stretch }
}

unsafe extern "system" fn font_style(this: *mut c_void) -> u32 {
    // SAFETY: our font.
    unsafe { fnt(this).0.desc.style }
}

unsafe extern "system" fn font_is_symbol(this: *mut c_void) -> i32 {
    // SAFETY: our font.
    unsafe {
        let (f, c) = fnt(this);
        let e = &c.data.entries[f.desc.entry];
        i32::from(e.symbol.unwrap_or(e.info.is_symbol))
    }
}

unsafe extern "system" fn font_face_names(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our font.
    unsafe {
        let (f, _) = fnt(this);
        put(out, new_strings(f.desc.face_names.clone()));
    }
    S_OK
}

unsafe extern "system" fn font_info_strings(
    this: *mut c_void,
    id: u32,
    out: *mut c_void,
    exists: *mut c_void,
) -> HRESULT {
    // SAFETY: our font.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        put_val(exists, 0i32);
        let (f, c) = fnt(this);
        if let Some(l) = c.data.entries[f.desc.entry].info.informational(id) {
            put(out, new_strings(l));
            put_val(exists, 1i32);
        }
    }
    S_OK
}

unsafe extern "system" fn font_simulations(this: *mut c_void) -> u32 {
    // SAFETY: our font.
    unsafe { fnt(this).0.desc.simulations }
}

unsafe extern "system" fn font_metrics(this: *mut c_void, out: *mut c_void) {
    // SAFETY: our font; `out` is a `DWRITE_FONT_METRICS`.
    unsafe {
        let (f, c) = fnt(this);
        crate::face::write_metrics(out, &c.data.entries[f.desc.entry].info.metrics);
    }
}

unsafe extern "system" fn font_has_character(this: *mut c_void, cp: u32, exists: *mut c_void) -> HRESULT {
    // SAFETY: our font.
    unsafe {
        put_val(exists, 0i32);
        let (f, c) = fnt(this);
        let e = &c.data.entries[f.desc.entry];
        if !in_charset(e.charset, cp) {
            return S_OK;
        }
        let Ok(bytes) = c.data.srcs[f.desc.entry].bytes() else { return S_OK };
        if let Some(sf) = Sfnt::parse(&bytes, e.index) {
            put_val(exists, i32::from(Cmap::of(&sf).glyph(cp) != 0));
        }
    }
    S_OK
}

unsafe extern "system" fn font_create_face(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our font.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let (f, c) = fnt(this);
        let e = &c.data.entries[f.desc.entry];
        match crate::face::get_face(c.factory, &c.data.srcs[f.desc.entry], e, f.desc.simulations) {
            Ok(face) => {
                put(out, face);
                S_OK
            }
            Err(hr) => hr,
        }
    }
}

#[allow(dead_code)]
const _: u32 = STYLE_NORMAL;
