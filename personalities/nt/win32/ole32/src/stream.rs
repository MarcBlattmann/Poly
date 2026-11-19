//! Memory streams (`CreateStreamOnHGlobal`, `GetHGlobalFromStream`, `IStream`; `objidl.h`).
//!
//! Backed by a `kernel32` `HGLOBAL` block (`GlobalAlloc` / `GlobalLock` / `GlobalUnlock` /
//! `GlobalSize` / `GlobalFree`). Because `GlobalReAlloc` is not yet implemented in Poly's
//! `kernel32`, growth allocates a new `HGLOBAL`, copies the live bytes, and frees the old one;
//! `GetHGlobalFromStream` always returns the current handle (Learn: "The handle that
//! `GetHGlobalFromStream` returns might be different from the original handle").

use crate::{IID_IUNKNOWN, Locked};
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_com::guid::Guid;
use poly_com::hresult::{
    E_INVALIDARG, E_NOINTERFACE, E_OUTOFMEMORY, E_POINTER, Hresult, S_FALSE, S_OK, STG_E_INVALIDFUNCTION,
    STG_E_INVALIDPOINTER, STG_E_READFAULT, STG_E_WRITEFAULT,
};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn GlobalFree(mem: *mut c_void) -> *mut c_void;
    fn GlobalSize(mem: *mut c_void) -> usize;
    fn GlobalLock(mem: *mut c_void) -> *mut u8;
    fn GlobalUnlock(mem: *mut c_void) -> i32;
}

/// `GMEM_MOVEABLE | GMEM_ZEROINIT` (`minwinbase.h`).
const GHND: u32 = 0x0042;
/// `STGTY_STREAM` (`objidl.h`).
const STGTY_STREAM: u32 = 2;
/// `STGM_READWRITE` (`objidl.h`).
const STGM_READWRITE: u32 = 2;

/// `IID_ISequentialStream` (`objidl.h`): `{0C733A30-2A1C-11CE-ADE5-00AA0044773D}`.
const IID_ISEQUENTIAL_STREAM: Guid =
    Guid { data1: 0x0C73_3A30, data2: 0x2A1C, data3: 0x11CE, data4: [0xAD, 0xE5, 0x00, 0xAA, 0x00, 0x44, 0x77, 0x3D] };
/// `IID_IStream` (`objidl.h`): `{0000000C-0000-0000-C000-000000000046}`.
const IID_ISTREAM: Guid = Guid { data1: 0x0C, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// `STATSTG` (`objidl.h`).
#[repr(C)]
struct StatStg {
    pwcs_name: *mut u16,
    type_: u32,
    cb_size: u64,
    mtime: [u32; 2],
    ctime: [u32; 2],
    atime: [u32; 2],
    grf_mode: u32,
    grf_locks_supported: u32,
    clsid: Guid,
    grf_state_bits: u32,
    reserved: u32,
}

struct Buf {
    hglobal: *mut c_void,
    len: usize,
    cap: usize,
    delete_on_release: bool,
}

// SAFETY: accessed only under `Locked<Buf>`.
unsafe impl Send for Buf {}

impl Drop for Buf {
    fn drop(&mut self) {
        if self.delete_on_release && !self.hglobal.is_null() {
            // SAFETY: an HGLOBAL owned by this stream.
            unsafe { GlobalFree(self.hglobal) };
        }
    }
}

impl Buf {
    fn ensure_cap(&mut self, min_cap: usize) -> Result<(), Hresult> {
        if min_cap <= self.cap {
            return Ok(());
        }
        let new_cap = min_cap.max(self.cap.saturating_mul(2)).max(64);
        // SAFETY: kernel32's GlobalAlloc.
        let new_h = unsafe { GlobalAlloc(GHND, new_cap) };
        if new_h.is_null() {
            return Err(E_OUTOFMEMORY);
        }
        if self.len > 0 && !self.hglobal.is_null() {
            // SAFETY: both handles are valid HGLOBALs; `self.len <= self.cap < new_cap`.
            unsafe {
                let src = GlobalLock(self.hglobal);
                let dst = GlobalLock(new_h);
                if !src.is_null() && !dst.is_null() {
                    core::ptr::copy_nonoverlapping(src, dst, self.len);
                }
                GlobalUnlock(new_h);
                GlobalUnlock(self.hglobal);
            }
        }
        if !self.hglobal.is_null() {
            // SAFETY: old handle replaced by `new_h`.
            unsafe { GlobalFree(self.hglobal) };
        }
        self.hglobal = new_h;
        self.cap = new_cap;
        Ok(())
    }
}

#[repr(C)]
struct HGlobalStream {
    vtbl: *const StreamVtbl,
    refs: AtomicU32,
    buf: Arc<Locked<Buf>>,
    pos: Locked<u64>,
}

#[repr(C)]
struct StreamVtbl {
    qi: unsafe extern "system" fn(*mut HGlobalStream, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut HGlobalStream) -> u32,
    release: unsafe extern "system" fn(*mut HGlobalStream) -> u32,
    read: unsafe extern "system" fn(*mut HGlobalStream, *mut c_void, u32, *mut u32) -> Hresult,
    write: unsafe extern "system" fn(*mut HGlobalStream, *const c_void, u32, *mut u32) -> Hresult,
    seek: unsafe extern "system" fn(*mut HGlobalStream, i64, u32, *mut u64) -> Hresult,
    set_size: unsafe extern "system" fn(*mut HGlobalStream, u64) -> Hresult,
    copy_to: unsafe extern "system" fn(*mut HGlobalStream, *mut c_void, u64, *mut u64, *mut u64) -> Hresult,
    commit: unsafe extern "system" fn(*mut HGlobalStream, u32) -> Hresult,
    revert: unsafe extern "system" fn(*mut HGlobalStream) -> Hresult,
    lock_region: unsafe extern "system" fn(*mut HGlobalStream, u64, u64, u32) -> Hresult,
    unlock_region: unsafe extern "system" fn(*mut HGlobalStream, u64, u64, u32) -> Hresult,
    stat: unsafe extern "system" fn(*mut HGlobalStream, *mut StatStg, u32) -> Hresult,
    clone: unsafe extern "system" fn(*mut HGlobalStream, *mut *mut c_void) -> Hresult,
}

// SAFETY: table of function pointers.
unsafe impl Sync for StreamVtbl {}

static STREAM_VTBL: StreamVtbl = StreamVtbl {
    qi: stm_qi,
    add_ref: stm_add_ref,
    release: stm_release,
    read: stm_read,
    write: stm_write,
    seek: stm_seek,
    set_size: stm_set_size,
    copy_to: stm_copy_to,
    commit: stm_commit,
    revert: stm_revert,
    lock_region: stm_lock_region,
    unlock_region: stm_unlock_region,
    stat: stm_stat,
    clone: stm_clone,
};

unsafe extern "system" fn stm_qi(this: *mut HGlobalStream, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter and IID; a live stream.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_POINTER;
        }
        let g = *iid;
        if g == IID_IUNKNOWN || g == IID_ISEQUENTIAL_STREAM || g == IID_ISTREAM {
            (*this).refs.fetch_add(1, Ordering::AcqRel);
            *out = this.cast();
            S_OK
        } else {
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn stm_add_ref(this: *mut HGlobalStream) -> u32 {
    // SAFETY: a live stream.
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn stm_release(this: *mut HGlobalStream) -> u32 {
    // SAFETY: a live stream created by `Box::into_raw`.
    let n = unsafe { (*this).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n == 0 {
        // SAFETY: last reference.
        drop(unsafe { Box::from_raw(this) });
    }
    n
}

unsafe extern "system" fn stm_read(this: *mut HGlobalStream, pv: *mut c_void, cb: u32, pcb_read: *mut u32) -> Hresult {
    if pv.is_null() {
        return STG_E_INVALIDPOINTER;
    }
    // SAFETY: a live stream.
    let s = unsafe { &*this };
    let mut pos = s.pos.with(|p| *p);
    let read = s.buf.with(|b| {
        let pos_usize = usize::try_from(pos).unwrap_or(usize::MAX);
        let avail = b.len.saturating_sub(pos_usize);
        let n = (cb as usize).min(avail);
        if n > 0 && !b.hglobal.is_null() {
            // SAFETY: `pos_usize + n <= b.len <= b.cap`; `pv` has `cb >= n` writable bytes.
            unsafe {
                let src = GlobalLock(b.hglobal);
                if !src.is_null() {
                    core::ptr::copy_nonoverlapping(src.add(pos_usize), pv.cast(), n);
                }
                GlobalUnlock(b.hglobal);
            }
        }
        n
    });
    pos = pos.saturating_add(read as u64);
    s.pos.with(|p| *p = pos);
    if !pcb_read.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *pcb_read = read as u32 };
    }
    if read == cb as usize { S_OK } else { S_FALSE }
}

unsafe extern "system" fn stm_write(
    this: *mut HGlobalStream,
    pv: *const c_void,
    cb: u32,
    pcb_written: *mut u32,
) -> Hresult {
    if pv.is_null() && cb > 0 {
        return STG_E_INVALIDPOINTER;
    }
    // SAFETY: a live stream.
    let s = unsafe { &*this };
    let pos = s.pos.with(|p| *p);
    let Some(pos_usize) = usize::try_from(pos).ok() else { return E_OUTOFMEMORY };
    let Some(end) = pos_usize.checked_add(cb as usize) else { return E_OUTOFMEMORY };
    let res = s.buf.with(|b| -> Result<(), Hresult> {
        b.ensure_cap(end)?;
        if !b.hglobal.is_null() {
            // SAFETY: `end <= b.cap`; `pv` has `cb` readable bytes.
            unsafe {
                let dst = GlobalLock(b.hglobal);
                if dst.is_null() {
                    return Err(E_OUTOFMEMORY);
                }
                if pos_usize > b.len {
                    core::ptr::write_bytes(dst.add(b.len), 0, pos_usize - b.len);
                }
                if cb > 0 {
                    core::ptr::copy_nonoverlapping(pv.cast::<u8>(), dst.add(pos_usize), cb as usize);
                }
                GlobalUnlock(b.hglobal);
            }
        }
        if end > b.len {
            b.len = end;
        }
        Ok(())
    });
    if let Err(hr) = res {
        if !pcb_written.is_null() {
            // SAFETY: the caller's out parameter.
            unsafe { *pcb_written = 0 };
        }
        return hr;
    }
    s.pos.with(|p| *p = end as u64);
    if !pcb_written.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *pcb_written = cb };
    }
    S_OK
}

unsafe extern "system" fn stm_seek(
    this: *mut HGlobalStream,
    dlib_move: i64,
    origin: u32,
    new_pos: *mut u64,
) -> Hresult {
    // SAFETY: a live stream.
    let s = unsafe { &*this };
    let base: i64 = match origin {
        0 => 0,
        1 => s.pos.with(|p| *p) as i64,
        2 => s.buf.with(|b| b.len as i64),
        _ => return STG_E_INVALIDFUNCTION,
    };
    let Some(target) = base.checked_add(dlib_move) else { return STG_E_INVALIDFUNCTION };
    if target < 0 {
        return STG_E_INVALIDFUNCTION;
    }
    let target = target as u64;
    s.pos.with(|p| *p = target);
    if !new_pos.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *new_pos = target };
    }
    S_OK
}

unsafe extern "system" fn stm_set_size(this: *mut HGlobalStream, new_size: u64) -> Hresult {
    let Some(n) = usize::try_from(new_size).ok() else { return E_OUTOFMEMORY };
    // SAFETY: a live stream.
    let s = unsafe { &*this };
    let res = s.buf.with(|b| -> Result<(), Hresult> {
        if n > b.len {
            b.ensure_cap(n)?;
            if !b.hglobal.is_null() {
                // SAFETY: `n <= b.cap`.
                unsafe {
                    let dst = GlobalLock(b.hglobal);
                    if !dst.is_null() {
                        core::ptr::write_bytes(dst.add(b.len), 0, n - b.len);
                    }
                    GlobalUnlock(b.hglobal);
                }
            }
        }
        b.len = n;
        Ok(())
    });
    res.err().unwrap_or(S_OK)
}

unsafe extern "system" fn stm_copy_to(
    this: *mut HGlobalStream,
    dest: *mut c_void,
    cb: u64,
    pcb_read: *mut u64,
    pcb_written: *mut u64,
) -> Hresult {
    if dest.is_null() {
        return STG_E_INVALIDPOINTER;
    }
    // SAFETY: a live stream.
    let s = unsafe { &*this };
    let pos = s.pos.with(|p| *p);
    let data = s.buf.with(|b| {
        let pos_usize = usize::try_from(pos).unwrap_or(usize::MAX);
        let avail = b.len.saturating_sub(pos_usize);
        let n = usize::try_from(cb).unwrap_or(usize::MAX).min(avail);
        let mut tmp = alloc::vec![0u8; n];
        if n > 0 && !b.hglobal.is_null() {
            // SAFETY: `pos_usize + n <= b.len <= b.cap`.
            unsafe {
                let src = GlobalLock(b.hglobal);
                if !src.is_null() {
                    core::ptr::copy_nonoverlapping(src.add(pos_usize), tmp.as_mut_ptr(), n);
                }
                GlobalUnlock(b.hglobal);
            }
        }
        tmp
    });
    let mut written_total = 0u64;
    let mut off = 0usize;
    let mut hr = S_OK;
    while off < data.len() {
        let chunk = (data.len() - off).min(u32::MAX as usize) as u32;
        let mut w = 0u32;
        // SAFETY: `dest` is the caller's IStream; slot 4 is `ISequentialStream::Write`.
        hr = unsafe {
            let vtbl = *(dest as *const *const usize);
            let f: unsafe extern "system" fn(*mut c_void, *const c_void, u32, *mut u32) -> Hresult =
                core::mem::transmute(*vtbl.add(4));
            f(dest, data.as_ptr().add(off).cast(), chunk, &mut w)
        };
        written_total += u64::from(w);
        off += w as usize;
        if hr < 0 || w == 0 {
            break;
        }
    }
    s.pos.with(|p| *p = pos.saturating_add(data.len() as u64));
    if !pcb_read.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *pcb_read = data.len() as u64 };
    }
    if !pcb_written.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *pcb_written = written_total };
    }
    hr
}

unsafe extern "system" fn stm_commit(_this: *mut HGlobalStream, _flags: u32) -> Hresult {
    S_OK
}

unsafe extern "system" fn stm_revert(_this: *mut HGlobalStream) -> Hresult {
    S_OK
}

unsafe extern "system" fn stm_lock_region(_this: *mut HGlobalStream, _off: u64, _cb: u64, _kind: u32) -> Hresult {
    STG_E_INVALIDFUNCTION
}

unsafe extern "system" fn stm_unlock_region(_this: *mut HGlobalStream, _off: u64, _cb: u64, _kind: u32) -> Hresult {
    STG_E_INVALIDFUNCTION
}

unsafe extern "system" fn stm_stat(this: *mut HGlobalStream, out: *mut StatStg, _flag: u32) -> Hresult {
    if out.is_null() {
        return STG_E_INVALIDPOINTER;
    }
    // SAFETY: a live stream and the caller's STATSTG.
    let len = unsafe { &*this }.buf.with(|b| b.len as u64);
    // SAFETY: the caller's STATSTG.
    unsafe {
        out.write(StatStg {
            pwcs_name: core::ptr::null_mut(),
            type_: STGTY_STREAM,
            cb_size: len,
            mtime: [0, 0],
            ctime: [0, 0],
            atime: [0, 0],
            grf_mode: STGM_READWRITE,
            grf_locks_supported: 0,
            clsid: Guid::default(),
            grf_state_bits: 0,
            reserved: 0,
        });
    }
    S_OK
}

unsafe extern "system" fn stm_clone(this: *mut HGlobalStream, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return STG_E_INVALIDPOINTER;
    }
    // SAFETY: a live stream.
    let s = unsafe { &*this };
    let pos = s.pos.with(|p| *p);
    let cloned = Box::into_raw(Box::new(HGlobalStream {
        vtbl: &STREAM_VTBL,
        refs: AtomicU32::new(1),
        buf: Arc::clone(&s.buf),
        pos: Locked::new(pos),
    }));
    // SAFETY: the caller's out parameter.
    unsafe { *out = cloned.cast() };
    S_OK
}

/// `CreateStreamOnHGlobal` (`combaseapi.h`, Learn): an `IStream` backed by an `HGLOBAL`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateStreamOnHGlobal(
    hglobal: *mut c_void,
    delete_on_release: i32,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    let (h, len, cap) = if hglobal.is_null() {
        // SAFETY: kernel32's GlobalAlloc.
        let h = unsafe { GlobalAlloc(GHND, 0) };
        if h.is_null() {
            return E_OUTOFMEMORY;
        }
        (h, 0, 0)
    } else {
        // SAFETY: the caller's HGLOBAL.
        let n = unsafe { GlobalSize(hglobal) };
        (hglobal, n, n)
    };
    let stm = Box::into_raw(Box::new(HGlobalStream {
        vtbl: &STREAM_VTBL,
        refs: AtomicU32::new(1),
        buf: Arc::new(Locked::new(Buf { hglobal: h, len, cap, delete_on_release: delete_on_release != 0 })),
        pos: Locked::new(0),
    }));
    // SAFETY: the caller's out parameter.
    unsafe { *out = stm.cast() };
    S_OK
}

/// `GetHGlobalFromStream` (`combaseapi.h`, Learn): the `HGLOBAL` of a stream made by
/// [`CreateStreamOnHGlobal`].
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetHGlobalFromStream(pstm: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if pstm.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's COM pointer: check its vtable against ours.
    let v = unsafe { *(pstm as *const usize) };
    if v != (&raw const STREAM_VTBL) as usize {
        // SAFETY: the caller's out parameter.
        unsafe { *out = core::ptr::null_mut() };
        return E_INVALIDARG;
    }
    // SAFETY: verified above to be one of our `HGlobalStream` instances.
    let h = unsafe { &*(pstm as *const HGlobalStream) }.buf.with(|b| b.hglobal);
    // SAFETY: the caller's out parameter.
    unsafe { *out = h };
    S_OK
}

/// Write all of `bytes` to an arbitrary `IStream*` (slot 4: `ISequentialStream::Write`).
pub(crate) unsafe fn write_all(stm: *mut c_void, mut bytes: &[u8]) -> Hresult {
    while !bytes.is_empty() {
        let mut written = 0u32;
        let chunk = bytes.len().min(u32::MAX as usize) as u32;
        // SAFETY: the caller's IStream pointer; slot 4 is Write.
        let hr = unsafe {
            let vtbl = *(stm as *const *const usize);
            let f: unsafe extern "system" fn(*mut c_void, *const c_void, u32, *mut u32) -> Hresult =
                core::mem::transmute(*vtbl.add(4));
            f(stm, bytes.as_ptr().cast(), chunk, &mut written)
        };
        if hr < 0 {
            return hr;
        }
        if written == 0 {
            return STG_E_WRITEFAULT;
        }
        bytes = &bytes[written as usize..];
    }
    S_OK
}

/// Read exactly `buf.len()` bytes from an arbitrary `IStream*` (slot 3: `ISequentialStream::Read`),
/// returning `STG_E_READFAULT` if the stream ends early.
pub(crate) unsafe fn read_exact(stm: *mut c_void, buf: &mut [u8]) -> Hresult {
    let mut at = 0usize;
    while at < buf.len() {
        let mut got = 0u32;
        let chunk = (buf.len() - at).min(u32::MAX as usize) as u32;
        // SAFETY: the caller's IStream pointer; slot 3 is Read.
        let hr = unsafe {
            let vtbl = *(stm as *const *const usize);
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> Hresult =
                core::mem::transmute(*vtbl.add(3));
            f(stm, buf.as_mut_ptr().add(at).cast(), chunk, &mut got)
        };
        if hr < 0 {
            return hr;
        }
        if got == 0 {
            return STG_E_READFAULT;
        }
        at += got as usize;
    }
    S_OK
}
