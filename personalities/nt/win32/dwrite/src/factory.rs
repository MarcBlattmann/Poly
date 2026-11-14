//! The factory: `DWriteCreateFactory` and the whole `IDWriteFactory`..
//! `IDWriteFactory7` table (one object answers for all of them), loader
//! registration and the cache of live faces.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};

use poly_dwrite::face::FaceInfo;
use poly_dwrite::model::FaceEntry;
use poly_dwrite::sfnt;
use poly_win32_gui::SpinLock;

use crate::abi::*;
use crate::coll::{CollData, new_collection};
use crate::com::*;
use crate::file::{FileSource, local_key, local_loader, new_font_file, source_of};
use crate::strings::wstr;
use crate::stub;

/// What identifies a face while someone holds it.
#[derive(Clone, PartialEq, Eq)]
pub struct FaceKey {
    pub loader: *mut c_void,
    pub key: Vec<u8>,
    pub index: u32,
    pub sims: u32,
    pub variant: u32,
}

struct State {
    file_loaders: Vec<*mut c_void>,
    coll_loaders: Vec<*mut c_void>,
    faces: Vec<(FaceKey, *mut crate::face::Face)>,
}

/// The `IUnknown` identity of the factory.
#[repr(C)]
struct Ident {
    vtbl: *const IUnknownVtbl,
    factory: *mut Factory,
}

#[repr(C)]
pub struct Factory {
    head: Head,
    ident: Ident,
    isolated: bool,
    st: SpinLock<State>,
}

// SAFETY: the registered loaders are COM objects the application keeps
// usable from any thread; the state is behind a lock.
unsafe impl Send for Factory {}
// SAFETY: as above.
unsafe impl Sync for Factory {}

static FACTORY_IIDS: [GUID; 7] = [
    IID_IDWriteFactory,
    IID_IDWriteFactory1,
    IID_IDWriteFactory2,
    IID_IDWriteFactory3,
    IID_IDWriteFactory4,
    IID_IDWriteFactory5,
    IID_IDWriteFactory6,
];
static FACTORY7_IID: GUID = IID_IDWriteFactory7;

static IDENT_VTBL: IUnknownVtbl =
    IUnknownVtbl { QueryInterface: ident_qi, AddRef: ident_add_ref, Release: ident_release };

unsafe extern "system" fn ident_qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: the identity object of one of our factories.
    unsafe {
        let f = (*(this as *const Ident)).factory as *mut c_void;
        if riid.is_null() || out.is_null() {
            return E_POINTER;
        }
        let iid = *(riid as *const GUID);
        if iid == IID_IUnknown {
            add_ref(f);
            *(out as *mut *mut c_void) = this;
            return S_OK;
        }
        factory_qi(f, riid, out)
    }
}

unsafe extern "system" fn ident_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: the identity object of one of our factories.
    unsafe { add_ref((*(this as *const Ident)).factory as *mut c_void) }
}

unsafe extern "system" fn ident_release(this: *mut c_void) -> u32 {
    // SAFETY: the identity object of one of our factories.
    unsafe { release((*(this as *const Ident)).factory as *mut c_void) }
}

unsafe extern "system" fn factory_qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our factory.
    unsafe {
        let out = out as *mut *mut c_void;
        if out.is_null() {
            return E_POINTER;
        }
        *out = core::ptr::null_mut();
        if riid.is_null() {
            return E_INVALIDARG;
        }
        let iid = *(riid as *const GUID);
        if iid == IID_IUnknown {
            let f = &mut *(this as *mut Factory);
            add_ref(this);
            *out = &mut f.ident as *mut Ident as *mut c_void;
            return S_OK;
        }
        if FACTORY_IIDS.contains(&iid) || iid == FACTORY7_IID {
            add_ref(this);
            *out = this;
            return S_OK;
        }
        E_NOINTERFACE
    }
}

static SHARED: AtomicPtr<Factory> = AtomicPtr::new(core::ptr::null_mut());
static SHARED_LOCK: SpinLock<()> = SpinLock::new(());

unsafe fn factory_dtor(h: *mut Head) {
    // SAFETY: the last reference to an isolated factory.
    unsafe {
        let f = &*(h as *const Factory);
        f.st.with(|s| {
            for &l in s.file_loaders.iter().chain(&s.coll_loaders) {
                com_release(l);
            }
        });
        drop_box::<Factory>(h);
    }
}

fn new_factory(isolated: bool) -> *mut Factory {
    let p = into_raw(Factory {
        head: Head::new(&FACTORY_VTBL as *const _ as *const c_void, factory_dtor, &[]),
        ident: Ident { vtbl: &IDENT_VTBL, factory: core::ptr::null_mut() },
        isolated,
        st: SpinLock::new(State { file_loaders: Vec::new(), coll_loaders: Vec::new(), faces: Vec::new() }),
    });
    // SAFETY: just allocated.
    unsafe { (*p).ident.factory = p };
    p
}

/// `DWriteCreateFactory`.
pub unsafe extern "system" fn dwrite_create_factory(kind: u32, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's output pointer.
    unsafe { *out = core::ptr::null_mut() };
    if kind > 1 {
        return E_INVALIDARG;
    }
    if iid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller passes a GUID.
    let want = unsafe { *iid };
    if want != IID_IUnknown && !FACTORY_IIDS.contains(&want) && want != FACTORY7_IID {
        return E_NOINTERFACE;
    }
    let f: *mut Factory = if kind == 0 {
        SHARED_LOCK.with(|_| {
            let p = SHARED.load(Ordering::Acquire);
            if p.is_null() {
                // The process keeps one reference of its own.
                let n = new_factory(false);
                SHARED.store(n, Ordering::Release);
                n
            } else {
                p
            }
        })
    } else {
        new_factory(true)
    };
    // The creation reference (isolated) or one more for the caller (shared).
    let fp = f as *mut c_void;
    if kind == 0 {
        // SAFETY: a live factory.
        unsafe { add_ref(fp) };
    }
    // SAFETY: `fp` is a live factory; `iid` is valid.
    let hr = unsafe { factory_qi(fp, iid as *mut c_void, out as *mut c_void) };
    // Drop the creation / caller reference taken above now that the
    // QueryInterface reference stands for it.
    // SAFETY: as above.
    unsafe { release(fp) };
    hr
}

// ----- the face cache

pub fn find_face(factory: *mut c_void, key: &FaceKey) -> Option<*mut crate::face::Face> {
    // SAFETY: a live factory.
    let f = unsafe { &*(factory as *const Factory) };
    f.st.with(|s| {
        for (k, p) in s.faces.iter() {
            if k == key {
                // SAFETY: the face is in the cache, so not freed yet; add a
                // reference only if it is still alive.
                let h = unsafe { &(*(*p as *const Head)).refs };
                let mut n = h.load(Ordering::Acquire);
                while n > 0 {
                    match h.compare_exchange(n, n + 1, Ordering::AcqRel, Ordering::Acquire) {
                        Ok(_) => return Some(*p),
                        Err(c) => n = c,
                    }
                }
            }
        }
        None
    })
}

pub fn remember_face(factory: *mut c_void, key: FaceKey, face: *mut crate::face::Face) {
    // SAFETY: a live factory.
    let f = unsafe { &*(factory as *const Factory) };
    f.st.with(|s| {
        s.faces.retain(|e| e.0 != key);
        s.faces.push((key, face));
    });
}

pub fn forget_face(factory: *mut c_void, face: *mut crate::face::Face) {
    // SAFETY: a live factory.
    let f = unsafe { &*(factory as *const Factory) };
    f.st.with(|s| s.faces.retain(|e| e.1 != face));
}

/// Is `loader` registered with `factory` (the built-in local loader always
/// is)?
pub unsafe fn file_loader_registered(factory: *mut c_void, loader: *mut c_void) -> bool {
    if loader == local_loader() {
        return true;
    }
    // SAFETY: one of our factories.
    let f = unsafe { &*(factory as *const Factory) };
    f.st.with(|s| s.file_loaders.contains(&loader))
}

// ----- the table

static FACTORY_VTBL: IDWriteFactory7Vtbl = IDWriteFactory7Vtbl {
    QueryInterface: factory_qi,
    AddRef: add_ref,
    Release: release,
    GetSystemFontCollection: get_system_font_collection,
    CreateCustomFontCollection: create_custom_font_collection,
    RegisterFontCollectionLoader: register_collection_loader,
    UnregisterFontCollectionLoader: unregister_collection_loader,
    CreateFontFileReference: create_font_file_reference,
    CreateCustomFontFileReference: create_custom_font_file_reference,
    CreateFontFace: create_font_face,
    CreateRenderingParams: crate::render::create_rendering_params,
    CreateMonitorRenderingParams: crate::render::create_monitor_rendering_params,
    CreateCustomRenderingParams: crate::render::create_custom_rendering_params,
    RegisterFontFileLoader: register_file_loader,
    UnregisterFontFileLoader: unregister_file_loader,
    CreateTextFormat: stub!(),
    CreateTypography: stub!(),
    GetGdiInterop: stub!(),
    CreateTextLayout: stub!(),
    CreateGdiCompatibleTextLayout: stub!(),
    CreateEllipsisTrimmingSign: stub!(),
    CreateTextAnalyzer: stub!(),
    CreateNumberSubstitution: crate::numsub::create_number_substitution,
    CreateGlyphRunAnalysis: crate::render::create_glyph_run_analysis,
    GetEudcFontCollection: get_eudc_font_collection,
    CreateCustomRenderingParams_1: crate::render::create_custom_rendering_params_1,
    GetSystemFontFallback: crate::fallback::get_system_font_fallback,
    CreateFontFallbackBuilder: crate::fallback::create_font_fallback_builder,
    TranslateColorGlyphRun: translate_color_glyph_run,
    CreateCustomRenderingParams_2: crate::render::create_custom_rendering_params_2,
    CreateGlyphRunAnalysis_2: crate::render::create_glyph_run_analysis_2,
    CreateGlyphRunAnalysis_3: crate::render::create_glyph_run_analysis_2,
    CreateCustomRenderingParams_3: crate::render::create_custom_rendering_params_2,
    CreateFontFaceReference: crate::fontset::create_font_face_reference_path,
    CreateFontFaceReference_ov2: crate::fontset::create_font_face_reference_file,
    GetSystemFontSet: crate::fontset::get_system_font_set,
    CreateFontSetBuilder: crate::fontset::create_font_set_builder,
    CreateFontCollectionFromFontSet: crate::fontset::create_font_collection_from_font_set,
    GetSystemFontCollection_3: get_system_font_collection_3,
    GetFontDownloadQueue: stub!(),
    TranslateColorGlyphRun_4: translate_color_glyph_run_4,
    ComputeGlyphOrigins: stub!(),
    ComputeGlyphOrigins_ov2: stub!(),
    CreateFontSetBuilder_5: crate::fontset::create_font_set_builder,
    CreateInMemoryFontFileLoader: create_in_memory_font_file_loader,
    CreateHttpFontFileLoader: stub!(),
    AnalyzeContainerType: analyze_container_type,
    UnpackFontFile: stub!(),
    CreateFontFaceReference_6: stub!(),
    CreateFontResource: stub!(),
    GetSystemFontSet_6: crate::fontset::get_system_font_set_6,
    GetSystemFontCollection_6: get_system_font_collection_6,
    CreateFontCollectionFromFontSet_6: crate::fontset::create_font_collection_from_font_set_6,
    CreateFontSetBuilder_6: crate::fontset::create_font_set_builder,
    CreateTextFormat_6: stub!(),
    GetSystemFontSet_7: crate::fontset::get_system_font_set_6,
    GetSystemFontCollection_7: get_system_font_collection_6,
};

unsafe fn fac<'a>(this: *mut c_void) -> &'a Factory {
    // SAFETY: our factory.
    unsafe { &*(this as *const Factory) }
}

unsafe extern "system" fn get_system_font_collection(this: *mut c_void, out: *mut c_void, _check: i32) -> HRESULT {
    // SAFETY: our factory; output writable.
    unsafe {
        put(out, new_collection(this, crate::system::system_data()));
    }
    S_OK
}

unsafe extern "system" fn get_system_font_collection_3(
    this: *mut c_void,
    _downloadable: i32,
    out: *mut c_void,
    _check: i32,
) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { get_system_font_collection(this, out, 0) }
}

unsafe extern "system" fn get_system_font_collection_6(
    this: *mut c_void,
    _downloadable: i32,
    _model: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: forwarded.
    unsafe { get_system_font_collection(this, out, 0) }
}

unsafe extern "system" fn get_eudc_font_collection(this: *mut c_void, out: *mut c_void, _check: i32) -> HRESULT {
    // SAFETY: our factory; output writable.
    unsafe {
        put(out, new_collection(this, Arc::new(CollData::new(Vec::new()))));
    }
    S_OK
}

/// The entries of every font of `srcs` that is a font; unreadable files
/// and data that is not a font are skipped.
pub fn entries_from_sources(srcs: &[Arc<FileSource>]) -> Vec<(FaceEntry, Arc<FileSource>)> {
    let mut out = Vec::new();
    for (n, src) in srcs.iter().enumerate() {
        let Ok(bytes) = src.bytes() else { continue };
        let a = sfnt::analyze(&bytes);
        if !a.supported {
            continue;
        }
        for idx in 0..a.faces {
            if let Some(info) = FaceInfo::parse(&bytes, idx) {
                out.push((FaceEntry::new(n, idx, Arc::new(info)), src.clone()));
            }
        }
    }
    out
}

unsafe extern "system" fn create_custom_font_collection(
    this: *mut c_void,
    loader: *mut c_void,
    key: *mut c_void,
    key_size: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our factory; the loader is an application COM object.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let registered = fac(this).st.with(|s| s.coll_loaders.contains(&loader));
        if !registered {
            return E_INVALIDARG;
        }
        let mut en: *mut c_void = core::ptr::null_mut();
        let hr = (vtbl::<IDWriteFontCollectionLoaderVtbl>(loader).CreateEnumeratorFromKey)(
            loader,
            this,
            key,
            key_size,
            &mut en as *mut *mut c_void as *mut c_void,
        );
        if hr < 0 {
            return hr;
        }
        if en.is_null() {
            return E_FAIL;
        }
        let ev = vtbl::<IDWriteFontFileEnumeratorVtbl>(en);
        let mut srcs: Vec<Arc<FileSource>> = Vec::new();
        loop {
            let mut has = 0i32;
            let hr = (ev.MoveNext)(en, &mut has as *mut i32 as *mut c_void);
            if hr < 0 {
                com_release(en);
                return hr;
            }
            if has == 0 {
                break;
            }
            let mut file: *mut c_void = core::ptr::null_mut();
            let hr = (ev.GetCurrentFontFile)(en, &mut file as *mut *mut c_void as *mut c_void);
            if hr < 0 {
                com_release(en);
                return hr;
            }
            if let Some(src) = source_of(file) {
                srcs.push(src);
            }
            com_release(file);
        }
        com_release(en);
        let data = Arc::new(CollData::new(entries_from_sources(&srcs)));
        put(out, new_collection(this, data));
    }
    S_OK
}

unsafe extern "system" fn register_collection_loader(this: *mut c_void, loader: *mut c_void) -> HRESULT {
    // SAFETY: our factory.
    unsafe {
        if loader.is_null() {
            return E_INVALIDARG;
        }
        let new = fac(this).st.with(|s| {
            if s.coll_loaders.contains(&loader) {
                false
            } else {
                s.coll_loaders.push(loader);
                true
            }
        });
        if new {
            com_add_ref(loader);
            S_OK
        } else {
            DWRITE_E_ALREADYREGISTERED
        }
    }
}

unsafe extern "system" fn unregister_collection_loader(this: *mut c_void, loader: *mut c_void) -> HRESULT {
    // SAFETY: our factory.
    unsafe {
        let had = fac(this).st.with(|s| {
            let n = s.coll_loaders.len();
            s.coll_loaders.retain(|&l| l != loader);
            s.coll_loaders.len() != n
        });
        if had {
            com_release(loader);
            S_OK
        } else {
            E_INVALIDARG
        }
    }
}

unsafe extern "system" fn register_file_loader(this: *mut c_void, loader: *mut c_void) -> HRESULT {
    // SAFETY: our factory.
    unsafe {
        if loader.is_null() {
            return E_INVALIDARG;
        }
        let new = fac(this).st.with(|s| {
            if s.file_loaders.contains(&loader) {
                false
            } else {
                s.file_loaders.push(loader);
                true
            }
        });
        if new {
            com_add_ref(loader);
            S_OK
        } else {
            DWRITE_E_ALREADYREGISTERED
        }
    }
}

unsafe extern "system" fn unregister_file_loader(this: *mut c_void, loader: *mut c_void) -> HRESULT {
    // SAFETY: our factory.
    unsafe {
        let had = fac(this).st.with(|s| {
            let n = s.file_loaders.len();
            s.file_loaders.retain(|&l| l != loader);
            s.file_loaders.len() != n
        });
        if had {
            com_release(loader);
            S_OK
        } else {
            E_INVALIDARG
        }
    }
}

unsafe extern "system" fn create_font_file_reference(
    this: *mut c_void,
    path: *mut c_void,
    last_write: *mut c_void,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our factory; `path` is NUL-terminated.
    unsafe {
        let _ = this;
        put::<c_void>(out, core::ptr::null_mut());
        if path.is_null() {
            return E_INVALIDARG;
        }
        let p = wstr(path as *const u16);
        let invalid = p.iter().any(|&c| matches!(c, 0x3F | 0x2A | 0x3C | 0x3E | 0x7C));
        match crate::io::kind(p) {
            _ if invalid => return DWRITE_E_FILENOTFOUND,
            crate::io::Kind::Missing => return DWRITE_E_FILENOTFOUND,
            crate::io::Kind::Directory => return DWRITE_E_FILEACCESS,
            crate::io::Kind::File => {}
        }
        let time = if last_write.is_null() { crate::io::write_time(p) } else { *(last_write as *const u64) };
        let src = FileSource::new(local_loader(), &local_key(p, time));
        put(out, new_font_file(src));
    }
    S_OK
}

unsafe extern "system" fn create_custom_font_file_reference(
    this: *mut c_void,
    key: *mut c_void,
    key_size: u32,
    loader: *mut c_void,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our factory; the key holds `key_size` bytes.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        if key.is_null() || loader.is_null() || !file_loader_registered(this, loader) {
            return E_INVALIDARG;
        }
        let src = FileSource::new(loader, slice(key, key_size));
        put(out, new_font_file(src));
    }
    S_OK
}

unsafe extern "system" fn create_font_face(
    this: *mut c_void,
    face_type: u32,
    n: u32,
    files: *mut c_void,
    index: u32,
    sims: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our factory; `files` holds `n` file pointers.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        if matches!(face_type, 3..=5) || face_type > 7 {
            return E_INVALIDARG;
        }
        if sims & !3 != 0 || n != 1 || files.is_null() {
            return E_INVALIDARG;
        }
        let file = *(files as *const *mut c_void);
        let Some(src) = source_of(file) else { return E_INVALIDARG };
        let bytes = match src.bytes() {
            Ok(b) => b,
            Err(hr) => return hr,
        };
        let a = sfnt::analyze(&bytes);
        if !a.supported || face_type == 2 || face_type == 7 {
            return DWRITE_E_FILEFORMAT;
        }
        if index >= a.faces {
            return E_INVALIDARG;
        }
        let Some(info) = FaceInfo::parse(&bytes, index) else { return DWRITE_E_FILEFORMAT };
        let entry = FaceEntry::new(0, index, Arc::new(info));
        match crate::face::get_face(this, &src, &entry, sims) {
            Ok(f) => {
                put(out, f);
                S_OK
            }
            Err(hr) => hr,
        }
    }
}

unsafe extern "system" fn translate_color_glyph_run(
    _this: *mut c_void,
    _x: f32,
    _y: f32,
    _run: *mut c_void,
    _desc: *mut c_void,
    _measuring: u32,
    _transform: *mut c_void,
    _palette: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: output writable.
    unsafe { put::<c_void>(out, core::ptr::null_mut()) };
    DWRITE_E_NOCOLOR
}

unsafe extern "system" fn translate_color_glyph_run_4(
    _this: *mut c_void,
    _origin: u64,
    _run: *mut c_void,
    _desc: *mut c_void,
    _format: u32,
    _measuring: u32,
    _transform: *mut c_void,
    _palette: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: output writable.
    unsafe { put::<c_void>(out, core::ptr::null_mut()) };
    DWRITE_E_NOCOLOR
}

unsafe extern "system" fn create_in_memory_font_file_loader(_this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: output writable.
    unsafe { put(out, crate::file::new_in_memory_loader()) };
    S_OK
}

unsafe extern "system" fn analyze_container_type(_this: *mut c_void, _data: *mut c_void, _size: u32) -> u32 {
    0
}
