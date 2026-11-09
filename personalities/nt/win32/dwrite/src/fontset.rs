//! Font sets (`IDWriteFontSet`), font face references
//! (`IDWriteFontFaceReference`) and the set builder
//! (`IDWriteFontSetBuilder1`) that Chrome's web fonts go through.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;

use poly_dwrite::face::FaceInfo;
use poly_dwrite::model::FaceEntry;
use poly_dwrite::sfnt;
use poly_win32_gui::SpinLock;

use crate::abi::*;
use crate::coll::{CollData, new_collection};
use crate::com::*;
use crate::file::{FileSource, local_key, local_loader, new_font_file, source_of};
use crate::stub;
use crate::strings::wstr;

unsafe extern "system" fn qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded unchanged.
    unsafe { query_interface(this, riid, out) }
}

// ----- the set

#[repr(C)]
pub struct FontSet {
    head: Head,
    factory: *mut c_void,
    data: Arc<CollData>,
}

static SET_IIDS: [GUID; 1] = [IID_IDWriteFontSet];

static SET_VTBL: IDWriteFontSetVtbl = IDWriteFontSetVtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    GetFontCount: set_count,
    GetFontFaceReference: set_face_reference,
    FindFontFaceReference: set_find_reference,
    FindFontFace: set_find_face,
    GetPropertyValues: stub!(),
    GetPropertyValues_ov2: stub!(),
    GetPropertyValues_ov3: stub!(),
    GetPropertyOccurrenceCount: stub!(),
    GetMatchingFonts: stub!(),
    GetMatchingFonts_ov2: stub!(),
};

unsafe fn set_dtor(h: *mut Head) {
    // SAFETY: the last reference to a `FontSet`.
    unsafe {
        com_release((*(h as *const FontSet)).factory);
        drop_box::<FontSet>(h);
    }
}

pub fn new_font_set_from_collection(factory: *mut c_void, data: Arc<CollData>) -> *mut c_void {
    // SAFETY: a live factory.
    unsafe { com_add_ref(factory) };
    into_raw(FontSet { head: Head::new(&SET_VTBL as *const _ as *const c_void, set_dtor, &SET_IIDS), factory, data }) as *mut c_void
}

unsafe fn set_of<'a>(p: *mut c_void) -> Option<&'a FontSet> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the first word of a COM object is its vtable pointer.
    if unsafe { *(p as *const *const c_void) } != &SET_VTBL as *const _ as *const c_void {
        return None;
    }
    // SAFETY: one of our sets.
    Some(unsafe { &*(p as *const FontSet) })
}

unsafe extern "system" fn set_count(this: *mut c_void) -> u32 {
    // SAFETY: our set.
    unsafe { (&*(this as *const FontSet)).data.entries.len() as u32 }
}

unsafe extern "system" fn set_face_reference(this: *mut c_void, i: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: our set.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let s = &*(this as *const FontSet);
        let Some(e) = s.data.entries.get(i as usize) else { return E_INVALIDARG };
        put(out, new_face_ref(s.factory, s.data.srcs[i as usize].clone(), e.clone(), 0));
    }
    S_OK
}

unsafe fn find_in(s: &FontSet, src: &FileSource, index: u32, sims: u32, out_index: *mut c_void, exists: *mut c_void) -> HRESULT {
    // SAFETY: outputs writable.
    unsafe {
        let _ = sims;
        put_val(out_index, u32::MAX);
        put_val(exists, 0i32);
        for (i, e) in s.data.entries.iter().enumerate() {
            if e.index == index && s.data.srcs[i].same(src) {
                put_val(out_index, i as u32);
                put_val(exists, 1i32);
                break;
            }
        }
    }
    S_OK
}

unsafe extern "system" fn set_find_reference(this: *mut c_void, r: *mut c_void, index: *mut c_void, exists: *mut c_void) -> HRESULT {
    // SAFETY: our set; `r` one of our references.
    unsafe {
        let Some(fr) = face_ref_of(r) else { return E_INVALIDARG };
        find_in(&*(this as *const FontSet), &fr.src, fr.entry.index, fr.sims, index, exists)
    }
}

unsafe extern "system" fn set_find_face(this: *mut c_void, face: *mut c_void, index: *mut c_void, exists: *mut c_void) -> HRESULT {
    // SAFETY: our set; `face` one of our faces.
    unsafe {
        let Some((src, idx, sims, _)) = crate::face::identity(face) else { return E_INVALIDARG };
        find_in(&*(this as *const FontSet), &src, idx, sims, index, exists)
    }
}

// ----- face references

#[repr(C)]
pub struct FaceRef {
    head: Head,
    factory: *mut c_void,
    src: Arc<FileSource>,
    entry: FaceEntry,
    sims: u32,
}

static REF_IIDS: [GUID; 1] = [IID_IDWriteFontFaceReference];

static REF_VTBL: IDWriteFontFaceReferenceVtbl = IDWriteFontFaceReferenceVtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    CreateFontFace: ref_create_face,
    CreateFontFaceWithSimulations: ref_create_face_sims,
    Equals: ref_equals,
    GetFontFaceIndex: ref_index,
    GetSimulations: ref_sims,
    GetFontFile: ref_file,
    GetLocalFileSize: ref_size,
    GetFileSize: ref_size,
    GetFileTime: ref_time,
    GetLocality: ref_locality,
    EnqueueFontDownloadRequest: ok0,
    EnqueueCharacterDownloadRequest: ok2,
    EnqueueGlyphDownloadRequest: ok2,
    EnqueueFileFragmentDownloadRequest: ok_frag,
};

unsafe extern "system" fn ok0(_: *mut c_void) -> HRESULT {
    S_OK
}
unsafe extern "system" fn ok2(_: *mut c_void, _: *mut c_void, _: u32) -> HRESULT {
    S_OK
}
unsafe extern "system" fn ok_frag(_: *mut c_void, _: u64, _: u64) -> HRESULT {
    S_OK
}

unsafe fn ref_dtor(h: *mut Head) {
    // SAFETY: the last reference to a `FaceRef`.
    unsafe {
        com_release((*(h as *const FaceRef)).factory);
        drop_box::<FaceRef>(h);
    }
}

fn new_face_ref(factory: *mut c_void, src: Arc<FileSource>, entry: FaceEntry, sims: u32) -> *mut c_void {
    // SAFETY: a live factory.
    unsafe { com_add_ref(factory) };
    into_raw(FaceRef { head: Head::new(&REF_VTBL as *const _ as *const c_void, ref_dtor, &REF_IIDS), factory, src, entry, sims }) as *mut c_void
}

unsafe fn face_ref_of<'a>(p: *mut c_void) -> Option<&'a FaceRef> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the first word of a COM object is its vtable pointer.
    if unsafe { *(p as *const *const c_void) } != &REF_VTBL as *const _ as *const c_void {
        return None;
    }
    // SAFETY: one of our references.
    Some(unsafe { &*(p as *const FaceRef) })
}

unsafe extern "system" fn ref_create_face(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our reference.
    unsafe {
        let r = &*(this as *const FaceRef);
        make_face(r, r.sims, out)
    }
}

unsafe fn make_face(r: &FaceRef, sims: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: output writable.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        if sims & !3 != 0 {
            return E_INVALIDARG;
        }
        match crate::face::get_face(r.factory, &r.src, &r.entry, sims) {
            Ok(f) => {
                put(out, f);
                S_OK
            }
            Err(hr) => hr,
        }
    }
}

unsafe extern "system" fn ref_create_face_sims(this: *mut c_void, sims: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: our reference.
    unsafe { make_face(&*(this as *const FaceRef), sims, out) }
}

unsafe extern "system" fn ref_equals(this: *mut c_void, other: *mut c_void) -> i32 {
    // SAFETY: our reference.
    unsafe {
        let a = &*(this as *const FaceRef);
        match face_ref_of(other) {
            Some(b) => i32::from(a.src.same(&b.src) && a.entry.index == b.entry.index && a.sims == b.sims),
            None => 0,
        }
    }
}

unsafe extern "system" fn ref_index(this: *mut c_void) -> u32 {
    // SAFETY: our reference.
    unsafe { (*(this as *const FaceRef)).entry.index }
}

unsafe extern "system" fn ref_sims(this: *mut c_void) -> u32 {
    // SAFETY: our reference.
    unsafe { (*(this as *const FaceRef)).sims }
}

unsafe extern "system" fn ref_file(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our reference.
    unsafe { put(out, new_font_file((*(this as *const FaceRef)).src.clone())) };
    S_OK
}

unsafe extern "system" fn ref_size(this: *mut c_void) -> u64 {
    // SAFETY: our reference.
    unsafe { (*(this as *const FaceRef)).src.bytes().map(|b| b.len() as u64).unwrap_or(0) }
}

unsafe extern "system" fn ref_time(_this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: output writable.
    unsafe { put_val(out, 0u64) };
    S_OK
}

unsafe extern "system" fn ref_locality(_: *mut c_void) -> u32 {
    2
}

// ----- the builder

#[repr(C)]
pub struct Builder {
    head: Head,
    factory: *mut c_void,
    items: SpinLock<Vec<(FaceEntry, Arc<FileSource>)>>,
}

static BUILDER_IIDS: [GUID; 2] = [IID_IDWriteFontSetBuilder, IID_IDWriteFontSetBuilder1];

static BUILDER_VTBL: IDWriteFontSetBuilder1Vtbl = IDWriteFontSetBuilder1Vtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    AddFontFaceReference: b_add_reference,
    AddFontFaceReference_ov2: b_add_reference_props,
    AddFontSet: b_add_set,
    CreateFontSet: b_create_set,
    AddFontFile: b_add_file,
};

unsafe fn builder_dtor(h: *mut Head) {
    // SAFETY: the last reference to a `Builder`.
    unsafe {
        com_release((*(h as *const Builder)).factory);
        drop_box::<Builder>(h);
    }
}

pub unsafe extern "system" fn create_font_set_builder(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: a live factory; output writable.
    unsafe {
        com_add_ref(this);
        put(
            out,
            into_raw(Builder {
                head: Head::new(&BUILDER_VTBL as *const _ as *const c_void, builder_dtor, &BUILDER_IIDS),
                factory: this,
                items: SpinLock::new(Vec::new()),
            }),
        );
    }
    S_OK
}

unsafe fn add_ref_to(this: *mut c_void, r: *mut c_void) -> HRESULT {
    // SAFETY: our builder; `r` a reference.
    unsafe {
        let b = &*(this as *const Builder);
        let Some(fr) = face_ref_of(r) else { return E_INVALIDARG };
        b.items.with(|v| v.push((fr.entry.clone(), fr.src.clone())));
    }
    S_OK
}

unsafe extern "system" fn b_add_reference(this: *mut c_void, r: *mut c_void) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { add_ref_to(this, r) }
}

unsafe extern "system" fn b_add_reference_props(this: *mut c_void, r: *mut c_void, _props: *mut c_void, _n: u32) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { add_ref_to(this, r) }
}

unsafe extern "system" fn b_add_set(this: *mut c_void, set: *mut c_void) -> HRESULT {
    // SAFETY: our builder and set.
    unsafe {
        let b = &*(this as *const Builder);
        let Some(s) = set_of(set) else { return E_INVALIDARG };
        b.items.with(|v| {
            for (i, e) in s.data.entries.iter().enumerate() {
                v.push((e.clone(), s.data.srcs[i].clone()));
            }
        });
    }
    S_OK
}

unsafe extern "system" fn b_create_set(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our builder.
    unsafe {
        let b = &*(this as *const Builder);
        let items = b.items.with(|v| v.clone());
        put(out, new_font_set_from_collection(b.factory, Arc::new(CollData::new(items))));
    }
    S_OK
}

unsafe extern "system" fn b_add_file(this: *mut c_void, file: *mut c_void) -> HRESULT {
    // SAFETY: our builder.
    unsafe {
        let b = &*(this as *const Builder);
        let Some(src) = source_of(file) else { return E_INVALIDARG };
        let bytes = match src.bytes() {
            Ok(b) => b,
            Err(hr) => return hr,
        };
        let a = sfnt::analyze(&bytes);
        if !a.supported {
            return DWRITE_E_FILEFORMAT;
        }
        let mut added = Vec::new();
        for idx in 0..a.faces {
            match FaceInfo::parse(&bytes, idx) {
                Some(info) => added.push((FaceEntry::new(0, idx, Arc::new(info)), src.clone())),
                None => return DWRITE_E_FILEFORMAT,
            }
        }
        b.items.with(|v| v.extend(added));
    }
    S_OK
}

// ----- factory entry points

pub unsafe extern "system" fn get_system_font_set(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: a live factory; output writable.
    unsafe { put(out, new_font_set_from_collection(this, crate::system::system_data())) };
    S_OK
}

pub unsafe extern "system" fn get_system_font_set_6(this: *mut c_void, _downloadable: i32, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { get_system_font_set(this, out) }
}

pub unsafe extern "system" fn create_font_collection_from_font_set(this: *mut c_void, set: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: a live factory; output writable.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let Some(s) = set_of(set) else { return E_INVALIDARG };
        let items: Vec<(FaceEntry, Arc<FileSource>)> =
            s.data.entries.iter().cloned().zip(s.data.srcs.iter().cloned()).collect();
        put(out, new_collection(this, Arc::new(CollData::new(items))));
    }
    S_OK
}

pub unsafe extern "system" fn create_font_collection_from_font_set_6(this: *mut c_void, set: *mut c_void, _model: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { create_font_collection_from_font_set(this, set, out) }
}

/// `IDWriteFactory3::CreateFontFaceReference(filePath, lastWriteTime, ...)`.
pub unsafe extern "system" fn create_font_face_reference_path(
    this: *mut c_void,
    path: *mut c_void,
    write_time: *mut c_void,
    index: u32,
    sims: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: a live factory; `path` NUL-terminated.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        if path.is_null() || sims & !3 != 0 {
            return E_INVALIDARG;
        }
        let p = wstr(path as *const u16);
        if crate::io::kind(p) != crate::io::Kind::File {
            return DWRITE_E_FILENOTFOUND;
        }
        let t = if write_time.is_null() { crate::io::write_time(p) } else { *(write_time as *const u64) };
        let src = FileSource::new(local_loader(), &local_key(p, t));
        make_ref(this, src, index, sims, out)
    }
}

/// `IDWriteFactory3::CreateFontFaceReference(fontFile, ...)`.
pub unsafe extern "system" fn create_font_face_reference_file(
    this: *mut c_void,
    file: *mut c_void,
    index: u32,
    sims: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: a live factory.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let Some(src) = source_of(file) else { return E_INVALIDARG };
        if sims & !3 != 0 {
            return E_INVALIDARG;
        }
        make_ref(this, src, index, sims, out)
    }
}

unsafe fn make_ref(factory: *mut c_void, src: Arc<FileSource>, index: u32, sims: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: output writable.
    unsafe {
        let bytes = match src.bytes() {
            Ok(b) => b,
            Err(hr) => return hr,
        };
        let a = sfnt::analyze(&bytes);
        if !a.supported {
            return DWRITE_E_FILEFORMAT;
        }
        if index >= a.faces {
            return E_INVALIDARG;
        }
        let Some(info) = FaceInfo::parse(&bytes, index) else { return DWRITE_E_FILEFORMAT };
        put(out, new_face_ref(factory, src, FaceEntry::new(0, index, Arc::new(info)), sims));
    }
    S_OK
}
