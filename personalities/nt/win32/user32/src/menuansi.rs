//! The ANSI forms of the menu functions (the programs built for Windows 9x-style `...A` calls:
//! PuTTY and others), the check mark bitmaps and the geometry queries of a window's menu bar.
//! Text is widened byte by byte (the Latin-1 range of Windows-1252) and narrowed with `?` for
//! what it cannot hold.

use super::menu::{self, fail, h32, with};
use super::menutrack;
use super::{H, info};
use poly_win32k::menu::{self as mn, InfoIn, Sel};

/// Run `f` with the item argument of an ANSI menu call: a string item gets its text widened
/// (and NUL-terminated), anything else (item data, a bitmap) the value as it is.
unsafe fn with_item(flags: u32, item: *const u8, f: impl FnOnce(*const u16) -> i32) -> i32 {
    if flags & (mn::MF_BITMAP | mn::MF_OWNERDRAW | mn::MF_SEPARATOR) == 0 && !item.is_null() {
        // SAFETY: the caller's NUL-terminated string.
        let mut w = unsafe { menu::read_text(item as usize, true) };
        w.push(0);
        f(w.as_ptr())
    } else {
        f(item as *const u16)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn AppendMenuA(menu: H, flags: u32, id: usize, item: *const u8) -> i32 {
    // SAFETY: as the caller's.
    unsafe { with_item(flags, item, |p| menu::AppendMenuW(menu, flags, id, p)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn InsertMenuA(menu: H, pos: u32, flags: u32, id: usize, item: *const u8) -> i32 {
    // SAFETY: as the caller's.
    unsafe { with_item(flags, item, |p| menu::InsertMenuW(menu, pos, flags, id, p)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ModifyMenuA(menu: H, pos: u32, flags: u32, id: usize, item: *const u8) -> i32 {
    // SAFETY: as the caller's.
    unsafe { with_item(flags, item, |p| menu::ModifyMenuW(menu, pos, flags, id, p)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMenuStringA(menu: H, id: u32, buf: *mut u8, max: i32, flags: u32) -> i32 {
    match with(|m| m.string(h32(menu), Sel::of(id, flags))) {
        // SAFETY: the caller's buffer of `max` bytes.
        Ok(t) => unsafe { menu::copy_text_a(&t, buf, max) },
        Err(e) => {
            if !buf.is_null() && max > 0 {
                // SAFETY: the caller's buffer.
                unsafe { *buf = 0 };
            }
            fail(e, 0)
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn InsertMenuItemA(menu: H, item: u32, by_pos: i32, p: *const u8) -> i32 {
    // SAFETY: as the caller's.
    unsafe { menu::insert_info(menu, item, by_pos, p, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetMenuItemInfoA(menu: H, item: u32, by_pos: i32, p: *const u8) -> i32 {
    // SAFETY: as the caller's.
    unsafe { menu::set_info_impl(menu, item, by_pos, p, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMenuItemInfoA(menu: H, item: u32, by_pos: i32, p: *mut u8) -> i32 {
    // SAFETY: as the caller's.
    unsafe { menu::get_info_impl(menu, item, by_pos, p, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadMenuA(instance: usize, name: *const u8) -> H {
    if (name as usize) < 0x1_0000 {
        // SAFETY: a resource number.
        return unsafe { menu::LoadMenuW(instance, name as *const u16) };
    }
    // SAFETY: a NUL-terminated name.
    let mut w = unsafe { menu::read_text(name as usize, true) };
    w.push(0);
    // SAFETY: the widened name.
    unsafe { menu::LoadMenuW(instance, w.as_ptr()) }
}

/// `GetMenuCheckMarkDimensions`: the check mark's size, width in the low word.
#[unsafe(no_mangle)]
pub extern "system" fn GetMenuCheckMarkDimensions() -> u32 {
    15 | (15 << 16)
}

/// `SetMenuItemBitmaps`: the bitmaps are kept for `GetMenuItemInfo` (the drawing shows
/// Poly's own check mark).
#[unsafe(no_mangle)]
pub extern "system" fn SetMenuItemBitmaps(menu: H, pos: u32, flags: u32, checked: usize, unchecked: usize) -> i32 {
    let info =
        InfoIn { mask: mn::MIIM_CHECKMARKS, hbm_checked: checked, hbm_unchecked: unchecked, ..InfoIn::default() };
    match with(|m| m.set_item_info(h32(menu), Sel::of(pos, flags), &info)) {
        Ok(()) => 1,
        Err(e) => fail(e, 0),
    }
}

/// `HiliteMenuItem`: sets or clears the highlight state of an item.
#[unsafe(no_mangle)]
pub extern "system" fn HiliteMenuItem(_hwnd: H, menu: H, id: u32, flags: u32) -> i32 {
    let sel = Sel::of(id, flags);
    let r = with(|m| {
        let it = m.item_info(h32(menu), sel)?;
        let state = if flags & mn::MF_HILITE != 0 { it.state | mn::MFS_HILITE } else { it.state & !mn::MFS_HILITE };
        m.set_item_info(h32(menu), sel, &InfoIn { mask: mn::MIIM_STATE, state, ..InfoIn::default() })
    });
    i32::from(r.is_ok())
}

/// `GetMenuItemRect` for the items of a window's menu bar (a popup's items have no place
/// but while it is shown).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMenuItemRect(hwnd: H, menu: H, pos: u32, rect: *mut i32) -> i32 {
    if rect.is_null() {
        return fail(menu::ERROR_INVALID_PARAMETER, 0);
    }
    if !menu::is_menu(menu) {
        return fail(menu::ERROR_INVALID_MENU_HANDLE, 0);
    }
    if hwnd == 0 || info(hwnd).is_none() {
        return fail(1400, 0);
    }
    match menutrack::bar_item_rect(hwnd, h32(menu), pos as usize) {
        Some(r) => {
            // SAFETY: the caller's RECT.
            unsafe { core::ptr::copy_nonoverlapping(r.as_ptr(), rect, 4) };
            1
        }
        None => fail(1456, 0),
    }
}

/// `MenuItemFromPoint(hWnd, hMenu, ptScreen)`: the `POINT` arrives packed in one register.
#[unsafe(no_mangle)]
pub extern "system" fn MenuItemFromPoint(hwnd: H, menu: H, pt: u64) -> i32 {
    let (x, y) = (pt as u32 as i32, (pt >> 32) as u32 as i32);
    menutrack::bar_item_at(hwnd, h32(menu), x, y)
}
