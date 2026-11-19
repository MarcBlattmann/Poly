//! The S3 delta of the Chrome gap map (ADR 0172, `specs/windows/apps/chrome/gap-map.md`
//! section 8): the user32 calls Chrome's window code makes that no earlier
//! slice had. Where Poly has nothing to say (one layout, no hooks that
//! fire, no display-config adapter) the answers are the plainest ones a
//! browser accepts.

use super::{H, SYS};
use poly_win32k::dispcfg;
use poly_win32k::user::Env;

// `GetWindowPlacement` is slice B's now (`lib.rs`, MonitorFromWindow.yaml `state_rects`).

#[unsafe(no_mangle)]
pub extern "system" fn IsWindowArranged(_hwnd: H) -> i32 {
    0
}

// ----- keyboard layouts: the process's list (`kbdlayout`: the oracle's three installed ones
// with the desktop's in the middle, and what the process loaded)

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetKeyboardLayoutList(n: i32, list: *mut usize) -> i32 {
    if n < 0 {
        // ERROR_NOACCESS (GetKeyboardLayoutList.yaml `small_buffers`: a negative size).
        super::set_last_error(998);
        return 0;
    }
    let layouts = crate::kbdlayout::list();
    if n == 0 || list.is_null() {
        return layouts.len() as i32;
    }
    let k = (n as usize).min(layouts.len());
    for (i, l) in layouts.iter().take(k).enumerate() {
        // SAFETY: `n` HKLs of the caller.
        unsafe { *list.add(i) = *l as usize };
    }
    k as i32
}

/// Whether an `HKL` is one of the layouts of the list (for `imm32`).
#[unsafe(no_mangle)]
pub extern "system" fn PrivateLayoutInstalled(hkl: usize) -> i32 {
    i32::from(crate::kbdlayout::installed(hkl as u32))
}

// ----- small queries

// ----- display configuration: the desktop's monitor as one path (`poly_win32k::dispcfg`)

/// The size of the monitor in physical pixels, for every caller (ADR 0231).
fn monitor_size() -> (u32, u32) {
    let (w, h) = crate::display::desk().phys;
    (w.max(1) as u32, h.max(1) as u32)
}

/// `GetDisplayConfigBufferSizes`: the error code itself; the last error is not touched.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDisplayConfigBufferSizes(flags: u32, paths: *mut u32, modes: *mut u32) -> i32 {
    if paths.is_null() || modes.is_null() {
        return dispcfg::ERROR_INVALID_PARAMETER;
    }
    match dispcfg::buffer_sizes(flags, 1) {
        Ok((p, m)) => {
            // SAFETY: the caller's two counts, checked for NULL.
            unsafe {
                *paths = p;
                *modes = m;
            }
            0
        }
        Err(e) => {
            // SAFETY: as above; the mode count stays as it was.
            unsafe { *paths = 0 };
            e
        }
    }
}

/// `QueryDisplayConfig`: the error code itself; only a missing topology pointer for
/// `QDC_DATABASE_CURRENT` leaves the last error 998 (QueryDisplayConfig.yaml `last_error`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryDisplayConfig(
    flags: u32,
    np: *mut u32,
    paths: *mut u8,
    nm: *mut u32,
    modes: *mut u8,
    topology: *mut u32,
) -> i32 {
    if np.is_null() || nm.is_null() {
        return dispcfg::ERROR_INVALID_PARAMETER;
    }
    let (w, h) = monitor_size();
    // SAFETY: the caller's two counts, checked for NULL.
    let (cp, cm) = unsafe { (*np, *nm) };
    let q = dispcfg::query(flags, cp, cm, !topology.is_null(), w, h);
    if q.code == dispcfg::ERROR_INVALID_PARAMETER
        && flags & 7 == dispcfg::QDC_DATABASE_CURRENT
        && topology.is_null()
        && flags & !7 & !0x70 == 0
    {
        super::set_last_error(998);
    }
    if q.code == 0 || q.code == dispcfg::ERROR_INSUFFICIENT_BUFFER {
        // SAFETY: the arrays have the counts the caller passed (path and mode elements).
        unsafe {
            if !paths.is_null() {
                let n = q.paths.len().min(cp as usize * dispcfg::PATH_INFO_SIZE);
                core::ptr::copy_nonoverlapping(q.paths.as_ptr(), paths, n);
            }
            if !modes.is_null() {
                let n = q.modes.len().min(cm as usize * dispcfg::MODE_INFO_SIZE);
                core::ptr::copy_nonoverlapping(q.modes.as_ptr(), modes, n);
            }
        }
    }
    if let Some((p, m)) = q.counts {
        // SAFETY: the counts checked above.
        unsafe {
            *np = p;
            *nm = m;
        }
    }
    if let Some(t) = q.topology {
        // SAFETY: not NULL: `query` asks for a topology only when one was given.
        unsafe { *topology = t };
    }
    q.code
}

/// `DisplayConfigGetDeviceInfo`: the error code; a size below the header is 31 with the last error 87.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DisplayConfigGetDeviceInfo(packet: *mut u8) -> i32 {
    if packet.is_null() {
        return dispcfg::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: a DISPLAYCONFIG_DEVICE_INFO_HEADER (20 bytes): type, size, adapter, id.
    let size = unsafe { core::ptr::read_unaligned(packet.add(4) as *const u32) } as usize;
    if size < 20 {
        super::set_last_error(87);
        return dispcfg::ERROR_GEN_FAILURE;
    }
    let mut buf = alloc::vec![0u8; size];
    // SAFETY: the header says the packet has `size` bytes.
    unsafe { core::ptr::copy_nonoverlapping(packet, buf.as_mut_ptr(), size) };
    let (w, h) = monitor_size();
    let code = if dispcfg::get32_at(&buf, 0) == dispcfg::GET_DPI_SCALE {
        let d = crate::display::desk();
        let step = poly_win32k::dpiscale::step_of(d.scale) as u32;
        dispcfg::dpi_scale_get(&mut buf, step, poly_win32k::dpiscale::max_step(w, h) as u32)
    } else {
        dispcfg::device_info(&mut buf, w, h)
    };
    if code == 0 {
        // SAFETY: as above.
        unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), packet, size) };
    }
    code
}

/// `DisplayConfigSetDeviceInfo`: the DPI-scale packet changes the monitor's scale (ADR 0231,
/// DisplayConfigSetDeviceInfo.yaml); the return value is the error code.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DisplayConfigSetDeviceInfo(packet: *const u8) -> i32 {
    if packet.is_null() {
        return dispcfg::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: a DISPLAYCONFIG_DEVICE_INFO_HEADER (20 bytes): type, size, adapter, id.
    let size = unsafe { core::ptr::read_unaligned(packet.add(4) as *const u32) } as usize;
    if !(20..=4096).contains(&size) {
        return dispcfg::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the header says the packet has `size` bytes.
    let buf = unsafe { core::slice::from_raw_parts(packet, size) };
    match dispcfg::dpi_scale_set(buf) {
        Ok(step) => match super::call(poly_win32k::proto::Request::SetScale { step }) {
            poly_win32k::proto::Reply::Ok {} => 0,
            _ => dispcfg::ERROR_INVALID_PARAMETER,
        },
        Err(e) => e,
    }
}

#[allow(dead_code)]
fn keep() -> u32 {
    SYS.tid()
}
