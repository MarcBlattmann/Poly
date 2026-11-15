//! Menus of `user32.dll`: the functions that make and change them and link them to
//! windows (`specs/windows/user32.dll/*Menu*.yaml`); the tree itself is
//! `poly_win32k::menu`. The popup loop and the drawing are in `menutrack.rs`.
//!
//! A top-level window's menu bar is the handle in its `GWLP_ID` (the window long a
//! child window keeps its id in), as on Windows; the window owns it and destroys it
//! with itself (`Env::window_freed`).

use super::{H, SYS, call, info};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_win32_gui::{SpinLock, set_last_error, wide};
use poly_win32k::menu::{self as mn, Info, InfoIn, Item, MENUINFO_SIZE, MII_SIZE, MII_SIZE_SHORT, Menus, Sel};
use poly_win32k::metrics;
use poly_win32k::proto::{Reply, Request};
use poly_win32k::user as u;

pub(crate) const ERROR_INVALID_PARAMETER: u32 = 87;
pub(crate) const ERROR_INVALID_MENU_HANDLE: u32 = 1401;
const ERROR_CHILD_WINDOW_MENU: u32 = 1436;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;

/// The process's menus.
pub(crate) static MENUS: SpinLock<Menus> = SpinLock::new(Menus::new());
/// The system menu of each window that asked for one: window -> menu.
static SYSMENUS: SpinLock<BTreeMap<u32, u32>> = SpinLock::new(BTreeMap::new());

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn FindResourceExW(module: *mut u8, ty: *const u16, name: *const u16, lang: u16) -> *mut u8;
    fn LoadResource(module: *mut u8, res: *mut u8) -> *mut u8;
    fn LockResource(data: *mut u8) -> *mut u8;
    fn SizeofResource(module: *mut u8, res: *mut u8) -> u32;
}

const RT_MENU: usize = 4;
const WS_CHILD: u32 = 0x4000_0000;

/// The 32-bit handle of an `HMENU` (0 when the upper half is not 0).
pub(crate) fn h32(menu: H) -> u32 {
    if (menu as u64) >> 32 != 0 { 0 } else { menu as u32 }
}

pub(crate) fn with<R>(f: impl FnOnce(&mut Menus) -> R) -> R {
    MENUS.with(f)
}

pub(crate) fn is_menu(menu: H) -> bool {
    let h = h32(menu);
    h != 0 && with(|m| m.is_menu(h))
}

pub(crate) fn fail<T>(code: u32, v: T) -> T {
    set_last_error(code);
    v
}

fn bool_of(r: Result<(), u32>) -> i32 {
    match r {
        Ok(()) => 1,
        Err(e) => fail(e, 0),
    }
}

// ----- making menus

#[unsafe(no_mangle)]
pub extern "system" fn CreateMenu() -> H {
    with(|m| m.create(false)) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn CreatePopupMenu() -> H {
    with(|m| m.create(true)) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn DestroyMenu(menu: H) -> i32 {
    let h = h32(menu);
    if h != 0 && with(|m| m.destroy(h)) { 1 } else { fail(ERROR_INVALID_MENU_HANDLE, 0) }
}

#[unsafe(no_mangle)]
pub extern "system" fn IsMenu(menu: H) -> i32 {
    if is_menu(menu) { 1 } else { fail(ERROR_INVALID_MENU_HANDLE, 0) }
}

// ----- AppendMenuW, InsertMenuW, ModifyMenuW, DeleteMenu, RemoveMenu

/// The text a `lpNewItem` names, when the flags make it a string.
unsafe fn flag_text(flags: u32, item: *const u16) -> Option<Vec<u16>> {
    if flags & (mn::MF_BITMAP | mn::MF_OWNERDRAW | mn::MF_SEPARATOR) == 0 && !item.is_null() {
        // SAFETY: the caller's NUL-terminated string.
        Some(unsafe { wide(item, -1) })
    } else {
        None
    }
}

pub(crate) unsafe fn insert_impl(menu: H, sel: Sel, flags: u32, id: usize, item: *const u16) -> i32 {
    // SAFETY: lpNewItem is a string when the flags say so.
    let text = unsafe { flag_text(flags, item) };
    let h = h32(menu);
    let r = with(|m| {
        if !m.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let (it, bad) = m.item_from_flags(flags, id as u64, item as u64, text);
        m.insert_item(h, sel, it).map(|()| bad)
    });
    match r {
        Ok(bad) => {
            if bad {
                set_last_error(ERROR_INVALID_MENU_HANDLE);
            }
            1
        }
        Err(e) => fail(e, 0),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn AppendMenuW(menu: H, flags: u32, id: usize, item: *const u16) -> i32 {
    // SAFETY: as the caller's.
    unsafe { insert_impl(menu, Sel::Pos(u32::MAX), flags, id, item) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn InsertMenuW(menu: H, pos: u32, flags: u32, id: usize, item: *const u16) -> i32 {
    // SAFETY: as the caller's.
    unsafe { insert_impl(menu, Sel::of(pos, flags), flags, id, item) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ModifyMenuW(menu: H, pos: u32, flags: u32, id: usize, item: *const u16) -> i32 {
    // SAFETY: lpNewItem is a string when the flags say so.
    let text = unsafe { flag_text(flags, item) };
    let h = h32(menu);
    let r = with(|m| {
        if !m.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let (it, bad) = m.item_from_flags(flags, id as u64, item as u64, text);
        m.modify_item(h, Sel::of(pos, flags), flags, it).map(|()| bad)
    });
    match r {
        Ok(bad) => {
            if bad {
                set_last_error(ERROR_INVALID_MENU_HANDLE);
            }
            1
        }
        Err(e) => fail(e, 0),
    }
}

fn remove(menu: H, pos: u32, flags: u32, destroy: bool) -> i32 {
    let h = h32(menu);
    match with(|m| {
        if m.is_menu(h) { m.remove_item(h, Sel::of(pos, flags), destroy) } else { Err(ERROR_INVALID_MENU_HANDLE) }
    }) {
        Ok(found) => i32::from(found),
        Err(e) => fail(e, 0),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DeleteMenu(menu: H, pos: u32, flags: u32) -> i32 {
    remove(menu, pos, flags, true)
}

#[unsafe(no_mangle)]
pub extern "system" fn RemoveMenu(menu: H, pos: u32, flags: u32) -> i32 {
    remove(menu, pos, flags, false)
}

// ----- queries

#[unsafe(no_mangle)]
pub extern "system" fn GetMenuItemCount(menu: H) -> i32 {
    match with(|m| m.item_count(h32(menu))) {
        Some(n) => n as i32,
        None => fail(ERROR_INVALID_MENU_HANDLE, -1),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetSubMenu(menu: H, pos: i32) -> H {
    match with(|m| m.sub_menu(h32(menu), pos)) {
        Ok(s) => s as H,
        Err(e) => fail(e, 0),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetMenuItemID(menu: H, pos: i32) -> u32 {
    match with(|m| m.item_id(h32(menu), pos)) {
        Ok(id) => id,
        Err(e) => fail(e, u32::MAX),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetMenuState(menu: H, id: u32, flags: u32) -> u32 {
    match with(|m| m.state(h32(menu), Sel::of(id, flags))) {
        Ok(s) => s,
        Err(e) => fail(e, u32::MAX),
    }
}

/// Copy `text` as `GetMenuStringW` and `MIIM_STRING` do: at most `max - 1`
/// units and a NUL; the result is the number copied (the full length when
/// nothing can be written).
unsafe fn copy_text(text: &[u16], buf: *mut u16, max: i32) -> i32 {
    let n = text.len();
    if buf.is_null() || max == 0 {
        return n as i32;
    }
    if max < 0 {
        // SAFETY: a buffer the caller gave (a negative size writes the NUL only).
        unsafe { *buf = 0 };
        return n as i32;
    }
    let k = n.min(max as usize - 1);
    // SAFETY: room for `max` units.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), buf, k);
        *buf.add(k) = 0;
    }
    k as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMenuStringW(menu: H, id: u32, buf: *mut u16, max: i32, flags: u32) -> i32 {
    match with(|m| m.string(h32(menu), Sel::of(id, flags))) {
        // SAFETY: the caller's buffer of `max` units.
        Ok(t) => unsafe { copy_text(&t, buf, max) },
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
pub extern "system" fn CheckMenuItem(menu: H, id: u32, flags: u32) -> u32 {
    let sel = Sel::of(id, flags);
    match with(|m| m.check_item(h32(menu), sel, flags & mn::MF_CHECKED != 0)) {
        Ok(v) => v,
        Err(e) => fail(e, u32::MAX),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn EnableMenuItem(menu: H, id: u32, flags: u32) -> u32 {
    let sel = Sel::of(id, flags);
    match with(|m| m.enable_item(h32(menu), sel, flags & mn::MFS_GRAYED)) {
        Ok(v) => v,
        Err(e) => fail(e, u32::MAX),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn CheckMenuRadioItem(menu: H, first: u32, last: u32, check: u32, flags: u32) -> i32 {
    match with(|m| m.check_radio(h32(menu), first, last, check, flags)) {
        Ok(v) => i32::from(v),
        Err(e) => fail(e, 0),
    }
}

// ----- MENUITEMINFOW and MENUINFO

unsafe fn rd32(p: *const u8, o: usize) -> u32 {
    // SAFETY: inside the caller's structure.
    unsafe { (p.add(o) as *const u32).read_unaligned() }
}

unsafe fn rd64(p: *const u8, o: usize) -> u64 {
    // SAFETY: inside the caller's structure.
    unsafe { (p.add(o) as *const u64).read_unaligned() }
}

unsafe fn wr32(p: *mut u8, o: usize, v: u32) {
    // SAFETY: inside the caller's structure.
    unsafe { (p.add(o) as *mut u32).write_unaligned(v) }
}

unsafe fn wr64(p: *mut u8, o: usize, v: u64) {
    // SAFETY: inside the caller's structure.
    unsafe { (p.add(o) as *mut u64).write_unaligned(v) }
}

/// A `MENUITEMINFOW` read (`winuser.h`: cbSize 0, fMask 4, fType 8, fState 12, wID 16,
/// hSubMenu 24, hbmpChecked 32, hbmpUnchecked 40, dwItemData 48, dwTypeData 56, cch 64,
/// hbmpItem 72).
pub(crate) unsafe fn read_info(p: *const u8, ansi: bool) -> Result<InfoIn, u32> {
    if p.is_null() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's structure; its size field says how much is there.
    unsafe {
        let size = rd32(p, 0);
        if size != MII_SIZE && size != MII_SIZE_SHORT {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let mask = rd32(p, 4);
        let ftype = rd32(p, 8);
        let type_data = rd64(p, 56) as usize;
        let string_type = ftype & (mn::MFT_BITMAP | mn::MFT_OWNERDRAW | mn::MFT_SEPARATOR) == 0;
        let wants_text = mask & mn::MIIM_STRING != 0 || (mask & mn::MIIM_TYPE != 0 && string_type);
        Ok(InfoIn {
            mask,
            ftype,
            state: rd32(p, 12),
            id: rd32(p, 16),
            sub: rd64(p, 24) as u32,
            hbm_checked: rd64(p, 32) as usize,
            hbm_unchecked: rd64(p, 40) as usize,
            data: rd64(p, 48) as usize,
            text: if wants_text && type_data != 0 { Some(read_text(type_data, ansi)) } else { None },
            hbm_item: if size == MII_SIZE { rd64(p, 72) as usize } else { 0 },
        })
    }
}

/// A NUL-terminated string at `p`: UTF-16, or ANSI widened (Windows-1252's Latin-1 range).
pub(crate) unsafe fn read_text(p: usize, ansi: bool) -> Vec<u16> {
    if !ansi {
        // SAFETY: the caller's string.
        return unsafe { wide(p as *const u16, -1) };
    }
    let mut v = Vec::new();
    let mut q = p as *const u8;
    // SAFETY: the caller's NUL-terminated string.
    unsafe {
        while *q != 0 {
            v.push(u16::from(*q));
            q = q.add(1);
        }
    }
    v
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn InsertMenuItemW(menu: H, item: u32, by_pos: i32, p: *const u8) -> i32 {
    // SAFETY: as the caller's.
    unsafe { insert_info(menu, item, by_pos, p, false) }
}

pub(crate) unsafe fn insert_info(menu: H, item: u32, by_pos: i32, p: *const u8, ansi: bool) -> i32 {
    let h = h32(menu);
    if !with(|m| m.is_menu(h)) {
        return fail(ERROR_INVALID_MENU_HANDLE, 0);
    }
    // SAFETY: the caller's structure.
    let info = match unsafe { read_info(p, ansi) } {
        Ok(i) => i,
        Err(e) => return fail(e, 0),
    };
    let sel = if by_pos != 0 { Sel::Pos(item) } else { Sel::Cmd(item) };
    bool_of(with(|m| {
        let it = m.item_from_info(&info)?;
        m.insert_item(h, sel, it)
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetMenuItemInfoW(menu: H, item: u32, by_pos: i32, p: *const u8) -> i32 {
    // SAFETY: as the caller's.
    unsafe { set_info_impl(menu, item, by_pos, p, false) }
}

pub(crate) unsafe fn set_info_impl(menu: H, item: u32, by_pos: i32, p: *const u8, ansi: bool) -> i32 {
    let h = h32(menu);
    if !with(|m| m.is_menu(h)) {
        return fail(ERROR_INVALID_MENU_HANDLE, 0);
    }
    // SAFETY: the caller's structure.
    let info = match unsafe { read_info(p, ansi) } {
        Ok(i) => i,
        Err(e) => return fail(e, 0),
    };
    let sel = if by_pos != 0 { Sel::Pos(item) } else { Sel::Cmd(item) };
    bool_of(with(|m| m.set_item_info(h, sel, &info)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMenuItemInfoW(menu: H, item: u32, by_pos: i32, p: *mut u8) -> i32 {
    // SAFETY: as the caller's.
    unsafe { get_info_impl(menu, item, by_pos, p, false) }
}

/// Copy `text` as ANSI (characters above Latin-1 become `?`): as [`copy_text`].
pub(crate) unsafe fn copy_text_a(text: &[u16], buf: *mut u8, max: i32) -> i32 {
    let n = text.len();
    if buf.is_null() || max == 0 {
        return n as i32;
    }
    if max < 0 {
        // SAFETY: a buffer the caller gave (a negative size writes the NUL only).
        unsafe { *buf = 0 };
        return n as i32;
    }
    let k = n.min(max as usize - 1);
    // SAFETY: room for `max` bytes.
    unsafe {
        for (i, c) in text.iter().take(k).enumerate() {
            *buf.add(i) = if *c < 0x100 { *c as u8 } else { b'?' };
        }
        *buf.add(k) = 0;
    }
    k as i32
}

pub(crate) unsafe fn get_info_impl(menu: H, item: u32, by_pos: i32, p: *mut u8, ansi: bool) -> i32 {
    if p.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    // SAFETY: the caller's structure.
    let (size, mask) = unsafe { (rd32(p, 0), rd32(p, 4)) };
    if size != MII_SIZE && size != MII_SIZE_SHORT {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let sel = if by_pos != 0 { Sel::Pos(item) } else { Sel::Cmd(item) };
    let it = match with(|m| m.item_info(h32(menu), sel)) {
        Ok(i) => i,
        Err(e) => return fail(e, 0),
    };
    // SAFETY: the caller's structure of `size` bytes.
    unsafe {
        if mask & mn::MIIM_STATE != 0 {
            wr32(p, 12, it.state);
        }
        if mask & mn::MIIM_ID != 0 {
            wr32(p, 16, it.id);
        }
        if mask & mn::MIIM_SUBMENU != 0 {
            wr64(p, 24, u64::from(it.sub));
        }
        if mask & mn::MIIM_CHECKMARKS != 0 {
            wr64(p, 32, it.hbm_checked as u64);
            wr64(p, 40, it.hbm_unchecked as u64);
        }
        if mask & mn::MIIM_DATA != 0 {
            wr64(p, 48, it.data as u64);
        }
        if mask & (mn::MIIM_FTYPE | mn::MIIM_TYPE) != 0 {
            wr32(p, 8, it.ftype);
        }
        if mask & mn::MIIM_BITMAP != 0 && size == MII_SIZE {
            wr64(p, 72, it.hbm_item as u64);
        }
        if mask & (mn::MIIM_STRING | mn::MIIM_TYPE) != 0 {
            let buf = rd64(p, 56) as usize;
            let cap = rd32(p, 64) as i32;
            let text = it.string();
            if buf == 0 {
                wr32(p, 64, text.len() as u32);
            } else {
                // A buffer of 0 units gets nothing written; cch is the length then.
                let n =
                    if ansi { copy_text_a(text, buf as *mut u8, cap) } else { copy_text(text, buf as *mut u16, cap) };
                wr32(p, 64, n as u32);
            }
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMenuInfo(menu: H, p: *mut u8) -> i32 {
    if p.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    // SAFETY: the caller's MENUINFO.
    if unsafe { rd32(p, 0) } != MENUINFO_SIZE {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let v = match with(|m| m.info(h32(menu))) {
        Ok(v) => v,
        Err(e) => return fail(e, 0),
    };
    // SAFETY: a MENUINFO (cbSize 0, fMask 4, dwStyle 8, cyMax 12, hbrBack 16, dwContextHelpID 24, dwMenuData 32).
    unsafe {
        let mask = rd32(p, 4);
        if mask & mn::MIM_STYLE != 0 {
            wr32(p, 8, v.style);
        }
        if mask & mn::MIM_MAXHEIGHT != 0 {
            wr32(p, 12, v.cy_max);
        }
        if mask & mn::MIM_BACKGROUND != 0 {
            wr64(p, 16, v.back as u64);
        }
        if mask & mn::MIM_HELPID != 0 {
            wr32(p, 24, v.help_id);
        }
        if mask & mn::MIM_MENUDATA != 0 {
            wr64(p, 32, v.data as u64);
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetMenuInfo(menu: H, p: *const u8) -> i32 {
    if p.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    // SAFETY: the caller's MENUINFO.
    let (mask, v) = unsafe {
        if rd32(p, 0) != MENUINFO_SIZE {
            return fail(ERROR_INVALID_PARAMETER, 0);
        }
        (
            rd32(p, 4),
            Info {
                style: rd32(p, 8),
                cy_max: rd32(p, 12),
                back: rd64(p, 16) as usize,
                help_id: rd32(p, 24),
                data: rd64(p, 32) as usize,
            },
        )
    };
    bool_of(with(|m| m.set_info(h32(menu), mask, &v)))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetMenuDefaultItem(menu: H, by_pos: u32, flags: u32) -> u32 {
    match with(|m| m.default_item(h32(menu), by_pos != 0, flags)) {
        Ok(v) => v,
        Err(e) => fail(e, u32::MAX),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn SetMenuDefaultItem(menu: H, item: u32, by_pos: u32) -> i32 {
    match with(|m| m.set_default_item(h32(menu), item, by_pos != 0)) {
        Ok(v) => i32::from(v),
        Err(e) => fail(e, 0),
    }
}

// ----- resources

/// The bytes of a menu resource of `module` (0: the executable), if it has one.
pub(crate) unsafe fn menu_resource(module: usize, name: *const u16) -> Option<&'static [u8]> {
    let module = if module == 0 {
        // SAFETY: NULL names the executable.
        unsafe { GetModuleHandleW(core::ptr::null()) }
    } else {
        module
    };
    // SAFETY: kernel32's resource calls check their handles.
    unsafe {
        let module = module as *mut u8;
        let hres = FindResourceExW(module, RT_MENU as *const u16, name, 0);
        if hres.is_null() {
            return None;
        }
        let data = LoadResource(module, hres);
        let size = SizeofResource(module, hres);
        if data.is_null() || size == 0 {
            return None;
        }
        let p = LockResource(data);
        (!p.is_null()).then(|| core::slice::from_raw_parts(p as *const u8, size as usize))
    }
}

/// `LoadMenuW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadMenuW(instance: usize, name: *const u16) -> H {
    // SAFETY: a name or a resource number.
    let Some(bytes) = (unsafe { menu_resource(instance, name) }) else { return 0 };
    match with(|m| m.load_template(bytes)) {
        Some(h) => h as H,
        None => fail(ERROR_INVALID_PARAMETER, 0),
    }
}

/// The menu a class names (`WNDCLASS.lpszMenuName`), loaded for a window.
pub(crate) fn class_menu(instance: usize, name: u64) -> H {
    if name == 0 {
        return 0;
    }
    let before = poly_win32_gui::last_error();
    // SAFETY: a name or a resource number the class was registered with.
    let h = unsafe { LoadMenuW(instance, name as *const u16) };
    set_last_error(before);
    h
}

// ----- menus and windows

/// The menu handle a window has in its `GWLP_ID`, if it is a top-level window.
fn bar_of(hwnd: H) -> Option<(poly_win32k::proto::WinInfo, u32)> {
    let i = info(hwnd)?;
    let top = i.style & WS_CHILD == 0;
    let id = i.id;
    Some((i, if top { id as u32 } else { 0 }))
}

/// A window was made with a menu: the bar's room in its surface.
pub(crate) fn after_create(hwnd: H) {
    let Some((_, menu)) = bar_of(hwnd) else { return };
    if bar_shown(u64::from(menu)) {
        super::menutrack::set_bar_height(hwnd, super::menutrack::bar_height());
    }
}

/// Whether a bar has a row of items (it takes room from the client area).
pub(crate) fn bar_shown(menu: u64) -> bool {
    menu >> 32 == 0 && with(|m| m.item_count(menu as u32)).is_some_and(|n| n > 0)
}

/// Make the window's frame follow its menu bar: the client area changes when the bar
/// appears or goes (the frame is recalculated), and the bar is drawn again.
fn refresh_bar(hwnd: H) {
    let Some((i, menu)) = bar_of(hwnd) else { return };
    if i.desktop {
        return;
    }
    let want = bar_shown(u64::from(menu));
    super::menutrack::set_bar_height(hwnd, if want { super::menutrack::bar_height() } else { 0 });
    let i = info(hwnd).unwrap_or(i);
    let base = metrics::client_insets(i.style, i.ex_style, false).top;
    let row = metrics::menu_for_dpi(96);
    let have = (i.client_rel.top - i.window_rel.top) - base >= row && !i.minimized;
    if want != have {
        let flags = u::SWP_NOMOVE | u::SWP_NOSIZE | u::SWP_NOZORDER | u::SWP_NOACTIVATE | u::SWP_FRAMECHANGED;
        u::set_window_pos(&SYS, hwnd as u64, 0, 0, 0, 0, 0, flags);
    }
    super::menutrack::draw_bar(hwnd);
}

#[unsafe(no_mangle)]
pub extern "system" fn GetMenu(hwnd: H) -> H {
    let Some(i) = info(hwnd) else { return fail(ERROR_INVALID_WINDOW_HANDLE, 0) };
    if i.desktop { 0 } else { i.id as H }
}

#[unsafe(no_mangle)]
pub extern "system" fn SetMenu(hwnd: H, menu: H) -> i32 {
    let Some(i) = info(hwnd) else { return fail(ERROR_INVALID_WINDOW_HANDLE, 0) };
    if i.style & WS_CHILD != 0 {
        return fail(ERROR_CHILD_WINDOW_MENU, 0);
    }
    if menu != 0 {
        let h = h32(menu);
        match with(|m| m.get(h).map(|x| x.popup)) {
            None => return fail(ERROR_INVALID_MENU_HANDLE, 0),
            Some(true) => return fail(ERROR_INVALID_PARAMETER, 0),
            Some(false) => {}
        }
    }
    match call(Request::SetLong { hwnd: hwnd as u64, index: -12, size: 8, value: menu as u64 }) {
        Reply::Error { code } => return fail(code, 0),
        _ => {}
    }
    refresh_bar(hwnd);
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn DrawMenuBar(hwnd: H) -> i32 {
    let Some(i) = info(hwnd) else { return fail(ERROR_INVALID_WINDOW_HANDLE, 0) };
    if i.desktop {
        return 0;
    }
    refresh_bar(hwnd);
    1
}

/// `SetWindowLongPtrW(GWLP_ID)` of a top-level window names a menu: false (with the
/// error set) when the value is not one.
pub(crate) fn check_top_level_id(hwnd: H, value: u64) -> bool {
    if value == 0 {
        return true;
    }
    match info(hwnd) {
        Some(i) if i.style & WS_CHILD == 0 && !is_menu(value as usize) => {
            set_last_error(ERROR_INVALID_MENU_HANDLE);
            false
        }
        _ => true,
    }
}

/// A window is going: its menu bar and system menu go with it.
pub(crate) fn window_freed(hwnd: u32, menu: u64) {
    if menu != 0 && menu >> 32 == 0 {
        with(|m| m.destroy(menu as u32));
    }
    let sys = SYSMENUS.with(|s| s.remove(&hwnd));
    if let Some(sm) = sys {
        with(|m| m.destroy(sm));
    }
}

// ----- the system menu

const SC_SIZE: usize = 0xF000;
const SC_MOVE: usize = 0xF010;
const SC_MINIMIZE: usize = 0xF020;
const SC_MAXIMIZE: usize = 0xF030;
const SC_CLOSE: usize = 0xF060;
const SC_RESTORE: usize = 0xF120;
const WS_SYSMENU: u32 = 0x0008_0000;

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn make_system_menu() -> u32 {
    with(|m| {
        let h = m.create(true);
        let mk = |m: &mut Menus, id: usize, text: &str, gray: bool| {
            let (mut it, _) = m.item_from_flags(0, id as u64, 0, Some(utf16(text)));
            if gray {
                it.state |= mn::MFS_GRAYED;
            }
            m.append_item(h, it).expect("new menu");
        };
        mk(m, SC_RESTORE, "&Restore", true);
        mk(m, SC_MOVE, "&Move", false);
        mk(m, SC_SIZE, "&Size", false);
        mk(m, SC_MINIMIZE, "Mi&nimize", false);
        mk(m, SC_MAXIMIZE, "Ma&ximize", false);
        let (sep, _) = m.item_from_flags(mn::MF_SEPARATOR, 0, 0, None);
        m.append_item(h, sep).expect("new menu");
        mk(m, SC_CLOSE, "&Close\tAlt+F4", false);
        h
    })
}

/// `GetSystemMenu`: the window's own copy of the system menu (made on the first call);
/// `revert` TRUE gives the copy up and returns NULL.
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMenu(hwnd: H, revert: i32) -> H {
    let Some(i) = info(hwnd) else { return fail(ERROR_INVALID_WINDOW_HANDLE, 0) };
    if i.style & WS_SYSMENU == 0 || i.style & WS_CHILD != 0 {
        return 0;
    }
    let key = i.hwnd;
    if revert != 0 {
        if let Some(sm) = SYSMENUS.with(|s| s.remove(&key)) {
            with(|m| m.destroy(sm));
        }
        return 0;
    }
    if let Some(sm) = SYSMENUS.with(|s| s.get(&key).copied()) {
        if with(|m| m.is_menu(sm)) {
            return sm as H;
        }
    }
    let h = make_system_menu();
    SYSMENUS.with(|s| s.insert(key, h));
    h as H
}

/// The menu a window's system menu is (0: it has not asked for one).
pub(crate) fn system_menu_of(hwnd: u32) -> u32 {
    SYSMENUS.with(|s| s.get(&hwnd).copied().unwrap_or(0))
}

/// The items of menu `h` (a copy), for drawing.
pub(crate) fn items_of(h: u32) -> Vec<Item> {
    with(|m| m.get(h).map(|x| x.items.clone()).unwrap_or_default())
}
