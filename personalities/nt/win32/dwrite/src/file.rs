//! Font files: the shared file source, `IDWriteFontFile`, the built-in
//! local file loader and its streams, and the in-memory loader.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;

use poly_win32_gui::SpinLock;

use crate::abi::*;
use crate::com::*;
use crate::io;
use poly_dwrite::sfnt;

/// What a font file reference stands for: a loader and a key, and the
/// bytes once they have been read.
pub struct FileSource {
    pub loader: *mut c_void,
    pub key: Vec<u8>,
    bytes: SpinLock<Option<Arc<Vec<u8>>>>,
}

// SAFETY: the loader is a COM object the application keeps alive and
// thread-safe; the rest is plain data behind a lock.
unsafe impl Send for FileSource {}
// SAFETY: as above.
unsafe impl Sync for FileSource {}

impl Drop for FileSource {
    fn drop(&mut self) {
        if self.loader != local_loader() {
            // SAFETY: we added a reference in `new`.
            unsafe { com_release(self.loader) };
        }
    }
}

impl FileSource {
    pub fn new(loader: *mut c_void, key: &[u8]) -> Arc<FileSource> {
        if loader != local_loader() {
            // SAFETY: a live loader the caller passed.
            unsafe { com_add_ref(loader) };
        }
        Arc::new(FileSource { loader, key: key.to_vec(), bytes: SpinLock::new(None) })
    }

    /// A source whose bytes are known already.
    pub fn with_bytes(loader: *mut c_void, key: &[u8], bytes: Arc<Vec<u8>>) -> Arc<FileSource> {
        let s = FileSource::new(loader, key);
        s.bytes.with(|b| *b = Some(bytes));
        s
    }

    /// Do two sources name the same file (loader and key)?
    pub fn same(&self, other: &FileSource) -> bool {
        self.loader == other.loader && self.key == other.key
    }

    /// The file's bytes, asking the loader on first use.
    pub fn bytes(&self) -> Result<Arc<Vec<u8>>, HRESULT> {
        if let Some(b) = self.bytes.with(|b| b.clone()) {
            return Ok(b);
        }
        let data = if self.loader == local_loader() {
            let (path, _) = parse_local_key(&self.key).ok_or(DWRITE_E_CACHEFORMAT)?;
            io::read_file(&path)?.0
        } else {
            read_via_loader(self.loader, &self.key)?
        };
        let arc = Arc::new(data);
        self.bytes.with(|b| *b = Some(arc.clone()));
        Ok(arc)
    }
}

/// `CreateStreamFromKey`, `GetFileSize`, `ReadFileFragment`.
fn read_via_loader(loader: *mut c_void, key: &[u8]) -> Result<Vec<u8>, HRESULT> {
    let mut stream: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live loader; the key is valid for the call.
    let hr = unsafe {
        (vtbl::<IDWriteFontFileLoaderVtbl>(loader).CreateStreamFromKey)(
            loader,
            key.as_ptr() as *mut c_void,
            key.len() as u32,
            &mut stream as *mut *mut c_void as *mut c_void,
        )
    };
    if hr < 0 {
        return Err(hr);
    }
    if stream.is_null() {
        return Err(E_FAIL);
    }
    let sv = unsafe { vtbl::<IDWriteFontFileStreamVtbl>(stream) };
    let mut size = 0u64;
    // SAFETY: a live stream.
    let hr = unsafe { (sv.GetFileSize)(stream, &mut size as *mut u64 as *mut c_void) };
    let result = if hr < 0 {
        Err(hr)
    } else if size > 0x4000_0000 {
        Err(E_FAIL)
    } else if size == 0 {
        Ok(Vec::new())
    } else {
        let mut start: *const u8 = core::ptr::null();
        let mut ctx: *mut c_void = core::ptr::null_mut();
        // SAFETY: a live stream; both outputs are writable.
        let hr = unsafe {
            (sv.ReadFileFragment)(
                stream,
                &mut start as *mut *const u8 as *mut c_void,
                0,
                size,
                &mut ctx as *mut *mut c_void as *mut c_void,
            )
        };
        if hr < 0 || start.is_null() {
            Err(if hr < 0 { hr } else { E_FAIL })
        } else {
            // SAFETY: the stream promises `size` bytes at `start` until released.
            let v = unsafe { core::slice::from_raw_parts(start, size as usize) }.to_vec();
            // SAFETY: releasing the fragment we read.
            unsafe { (sv.ReleaseFileFragment)(stream, ctx) };
            Ok(v)
        }
    };
    // SAFETY: we own the stream reference.
    unsafe { com_release(stream) };
    result
}

// ----- IDWriteFontFile

#[repr(C)]
pub struct FontFile {
    pub head: Head,
    pub src: Arc<FileSource>,
}

static FILE_VTBL: IDWriteFontFileVtbl = IDWriteFontFileVtbl {
    QueryInterface: query_interface_slot,
    AddRef: add_ref,
    Release: release,
    GetReferenceKey: file_get_reference_key,
    GetLoader: file_get_loader,
    Analyze: file_analyze,
};

unsafe extern "system" fn query_interface_slot(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded unchanged.
    unsafe { query_interface(this, riid, out) }
}

/// A new `IDWriteFontFile` (reference count 1).
pub fn new_font_file(src: Arc<FileSource>) -> *mut c_void {
    into_raw(FontFile {
        head: Head::new(&FILE_VTBL as *const _ as *const c_void, drop_box::<FontFile>, &[IID_IDWriteFontFile]),
        src,
    }) as *mut c_void
}

/// The source behind one of our `IDWriteFontFile` objects.
pub unsafe fn source_of(file: *mut c_void) -> Option<Arc<FileSource>> {
    if file.is_null() {
        return None;
    }
    // SAFETY: the first word of a COM object is its vtable pointer.
    let v = unsafe { *(file as *const *const c_void) };
    if v != &FILE_VTBL as *const _ as *const c_void {
        return None;
    }
    // SAFETY: one of our objects.
    Some(unsafe { (*(file as *const FontFile)).src.clone() })
}

unsafe extern "system" fn file_get_reference_key(this: *mut c_void, key: *mut c_void, size: *mut c_void) -> HRESULT {
    // SAFETY: our object; outputs writable.
    unsafe {
        let f = &*(this as *const FontFile);
        put(key, f.src.key.as_ptr() as *mut u8);
        put_val(size, f.src.key.len() as u32);
    }
    S_OK
}

unsafe extern "system" fn file_get_loader(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our object; output writable.
    unsafe {
        let f = &*(this as *const FontFile);
        com_add_ref(f.src.loader);
        put(out, f.src.loader);
    }
    S_OK
}

unsafe extern "system" fn file_analyze(
    this: *mut c_void,
    supported: *mut c_void,
    file_type: *mut c_void,
    face_type: *mut c_void,
    faces: *mut c_void,
) -> HRESULT {
    // SAFETY: our object; outputs writable.
    unsafe {
        let f = &*(this as *const FontFile);
        match f.src.bytes() {
            Err(hr) => {
                put_val(supported, 0i32);
                put_val(file_type, sfnt::file_type::UNKNOWN);
                put_val(face_type, sfnt::face_type::UNKNOWN);
                put_val(faces, 0u32);
                hr
            }
            Ok(bytes) => {
                let a = sfnt::analyze(&bytes);
                put_val(supported, i32::from(a.supported));
                put_val(file_type, a.file_type);
                put_val(face_type, a.face_type);
                put_val(faces, a.faces);
                S_OK
            }
        }
    }
}

// ----- the local file loader

/// The key of a local file: its path in UTF-16 code units, then its last
/// write time (8 bytes).
pub fn local_key(path: &[u16], time: u64) -> Vec<u8> {
    let mut k: Vec<u8> = path.iter().flat_map(|c| c.to_le_bytes()).collect();
    k.extend_from_slice(&time.to_le_bytes());
    k
}

pub fn parse_local_key(key: &[u8]) -> Option<(Vec<u16>, u64)> {
    if key.len() < 8 || (key.len() - 8) % 2 != 0 {
        return None;
    }
    let (p, t) = key.split_at(key.len() - 8);
    let path = p.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some((path, u64::from_le_bytes(t.try_into().ok()?)))
}

static LOCAL_IIDS: [GUID; 2] = [IID_IDWriteFontFileLoader, IID_IDWriteLocalFontFileLoader];

static LOCAL_VTBL: IDWriteLocalFontFileLoaderVtbl = IDWriteLocalFontFileLoaderVtbl {
    QueryInterface: query_interface_slot,
    AddRef: static_add_ref,
    Release: static_release,
    CreateStreamFromKey: local_create_stream,
    GetFilePathLengthFromKey: local_path_length,
    GetFilePathFromKey: local_path,
    GetLastWriteTimeFromKey: local_write_time,
};

unsafe extern "system" fn static_add_ref(_: *mut c_void) -> u32 {
    2
}

unsafe extern "system" fn static_release(_: *mut c_void) -> u32 {
    1
}

unsafe fn static_dtor(_: *mut Head) {}

struct LocalLoader(core::cell::UnsafeCell<Option<Head>>);
// SAFETY: written once at start-up (`local_loader`), read-only afterwards.
unsafe impl Sync for LocalLoader {}
static LOCAL: LocalLoader = LocalLoader(core::cell::UnsafeCell::new(None));
static LOCAL_INIT: poly_win32_gui::SpinLock<bool> = poly_win32_gui::SpinLock::new(false);

/// The process-wide built-in local file loader object.
pub fn local_loader() -> *mut c_void {
    LOCAL_INIT.with(|done| {
        if !*done {
            // SAFETY: under the lock, once.
            unsafe {
                *LOCAL.0.get() = Some(Head::new(&LOCAL_VTBL as *const _ as *const c_void, static_dtor, &LOCAL_IIDS));
            }
            *done = true;
        }
    });
    // SAFETY: initialised above.
    unsafe { (*LOCAL.0.get()).as_mut().expect("local loader") as *mut Head as *mut c_void }
}

unsafe extern "system" fn local_create_stream(
    _this: *mut c_void,
    key: *mut c_void,
    size: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: the caller passes `size` bytes at `key`.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let key: &[u8] = slice(key, size);
        let Some((path, _)) = parse_local_key(key) else { return DWRITE_E_CACHEFORMAT };
        match io::read_file(&path) {
            Ok((data, time)) => {
                put(out, new_stream(Arc::new(data), time));
                S_OK
            }
            Err(hr) => hr,
        }
    }
}

unsafe extern "system" fn local_path_length(_: *mut c_void, key: *mut c_void, size: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: the caller passes `size` bytes at `key`.
    unsafe {
        match parse_local_key(slice(key, size)) {
            Some((p, _)) => {
                put_val(out, p.len() as u32);
                S_OK
            }
            None => {
                put_val(out, u32::MAX);
                DWRITE_E_CACHEFORMAT
            }
        }
    }
}

unsafe extern "system" fn local_path(
    _: *mut c_void,
    key: *mut c_void,
    size: u32,
    buf: *mut c_void,
    cap: u32,
) -> HRESULT {
    // SAFETY: the caller passes `size` bytes at `key` and room for `cap` units.
    unsafe {
        let Some((p, _)) = parse_local_key(slice(key, size)) else { return DWRITE_E_CACHEFORMAT };
        if (cap as usize) < p.len() + 1 {
            return E_INSUFFICIENT_BUFFER;
        }
        let out = buf as *mut u16;
        for (i, c) in p.iter().enumerate() {
            *out.add(i) = *c;
        }
        *out.add(p.len()) = 0;
    }
    S_OK
}

unsafe extern "system" fn local_write_time(_: *mut c_void, key: *mut c_void, size: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: the caller passes `size` bytes at `key`.
    unsafe {
        match parse_local_key(slice(key, size)) {
            Some((path, t)) => {
                put_val(out, if t != 0 { t } else { io::write_time(&path) });
                S_OK
            }
            None => DWRITE_E_CACHEFORMAT,
        }
    }
}

// ----- streams

#[repr(C)]
struct Stream {
    head: Head,
    data: Arc<Vec<u8>>,
    time: u64,
}

static STREAM_IIDS: [GUID; 1] = [IID_IDWriteFontFileStream];

static STREAM_VTBL: IDWriteFontFileStreamVtbl = IDWriteFontFileStreamVtbl {
    QueryInterface: query_interface_slot,
    AddRef: add_ref,
    Release: release,
    ReadFileFragment: stream_read,
    ReleaseFileFragment: stream_release_fragment,
    GetFileSize: stream_size,
    GetLastWriteTime: stream_time,
};

fn new_stream(data: Arc<Vec<u8>>, time: u64) -> *mut c_void {
    into_raw(Stream {
        head: Head::new(&STREAM_VTBL as *const _ as *const c_void, drop_box::<Stream>, &STREAM_IIDS),
        data,
        time,
    }) as *mut c_void
}

unsafe extern "system" fn stream_read(
    this: *mut c_void,
    start: *mut c_void,
    offset: u64,
    size: u64,
    ctx: *mut c_void,
) -> HRESULT {
    // SAFETY: our stream; outputs writable.
    unsafe {
        let s = &*(this as *const Stream);
        put::<u8>(start, core::ptr::null_mut());
        put::<c_void>(ctx, core::ptr::null_mut());
        match offset.checked_add(size) {
            Some(end) if end <= s.data.len() as u64 => {
                put(start, s.data.as_ptr().add(offset as usize) as *mut u8);
                S_OK
            }
            _ => E_FAIL,
        }
    }
}

unsafe extern "system" fn stream_release_fragment(_: *mut c_void, _: *mut c_void) {}

unsafe extern "system" fn stream_size(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our stream; output writable.
    unsafe { put_val(out, (*(this as *const Stream)).data.len() as u64) };
    S_OK
}

unsafe extern "system" fn stream_time(this: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: our stream; output writable.
    unsafe { put_val(out, (*(this as *const Stream)).time) };
    S_OK
}

// ----- the in-memory loader (IDWriteFactory5)

#[repr(C)]
pub struct InMemoryLoader {
    pub head: Head,
    files: SpinLock<Vec<Arc<Vec<u8>>>>,
    count: core::sync::atomic::AtomicU32,
}

static INMEM_IIDS: [GUID; 2] = [IID_IDWriteFontFileLoader, IID_IDWriteInMemoryFontFileLoader];

static INMEM_VTBL: IDWriteInMemoryFontFileLoaderVtbl = IDWriteInMemoryFontFileLoaderVtbl {
    QueryInterface: query_interface_slot,
    AddRef: add_ref,
    Release: release,
    CreateStreamFromKey: inmem_create_stream,
    CreateInMemoryFontFileReference: inmem_create_reference,
    GetFileCount: inmem_count,
};

pub fn new_in_memory_loader() -> *mut c_void {
    into_raw(InMemoryLoader {
        head: Head::new(&INMEM_VTBL as *const _ as *const c_void, drop_box::<InMemoryLoader>, &INMEM_IIDS),
        files: SpinLock::new(Vec::new()),
        count: core::sync::atomic::AtomicU32::new(0),
    }) as *mut c_void
}

unsafe extern "system" fn inmem_create_stream(
    this: *mut c_void,
    key: *mut c_void,
    size: u32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our loader; the caller passes `size` bytes at `key`.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let l = &*(this as *const InMemoryLoader);
        let key: &[u8] = slice(key, size);
        let Ok(k) = <[u8; 4]>::try_from(key) else { return E_INVALIDARG };
        let n = u32::from_le_bytes(k) as usize;
        match l.files.with(|f| f.get(n).cloned()) {
            Some(data) => {
                put(out, new_stream(data, 0));
                S_OK
            }
            None => E_INVALIDARG,
        }
    }
}

unsafe extern "system" fn inmem_create_reference(
    this: *mut c_void,
    factory: *mut c_void,
    data: *mut c_void,
    size: u32,
    _owner: *mut c_void,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: our loader and factory; `data` holds `size` bytes.
    unsafe {
        put::<c_void>(out, core::ptr::null_mut());
        let l = &*(this as *const InMemoryLoader);
        if size > 0 {
            // Counted even when the reference is refused.
            l.count.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        if !crate::factory::file_loader_registered(factory, this) || size == 0 || data.is_null() {
            return E_INVALIDARG;
        }
        let bytes = Arc::new(slice::<u8>(data, size).to_vec());
        let n = l.files.with(|f| {
            f.push(bytes.clone());
            f.len() - 1
        });
        let src = FileSource::with_bytes(this, &(n as u32).to_le_bytes(), bytes);
        put(out, new_font_file(src));
    }
    S_OK
}

unsafe extern "system" fn inmem_count(this: *mut c_void) -> u32 {
    // SAFETY: our loader.
    unsafe { (*(this as *const InMemoryLoader)).count.load(core::sync::atomic::Ordering::Relaxed) }
}
