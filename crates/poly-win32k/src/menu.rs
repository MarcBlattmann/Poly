//! Menus: the handle table, the items, and the rules of the menu functions
//! of `user32.dll` (`specs/windows/user32.dll/{CreateMenu,AppendMenuW,...}.yaml`;
//! Learn, "About Menus", "Using Menus", "MENUITEMINFOW", "MENUINFO"; `winuser.h`
//! for the constants).
//!
//! A menu lives in the process that made it. This module holds no window
//! state: `user32` keeps one [`Menus`] table per process and links a window to
//! its menu bar through the window's long `GWLP_ID` (which a top-level window
//! shares with its menu handle, as on Windows).
//!
//! Handles are 32-bit values that are never used twice, so a stale handle
//! (a menu destroyed while a window still names it) stays invalid.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

// ----- constants (winuser.h)

pub const MF_BYCOMMAND: u32 = 0x0000;
pub const MF_ENABLED: u32 = 0x0000;
pub const MF_GRAYED: u32 = 0x0001;
pub const MF_DISABLED: u32 = 0x0002;
pub const MF_BITMAP: u32 = 0x0004;
pub const MF_CHECKED: u32 = 0x0008;
pub const MF_POPUP: u32 = 0x0010;
pub const MF_MENUBARBREAK: u32 = 0x0020;
pub const MF_MENUBREAK: u32 = 0x0040;
pub const MF_HILITE: u32 = 0x0080;
pub const MF_END: u32 = 0x0080;
pub const MF_OWNERDRAW: u32 = 0x0100;
pub const MF_BYPOSITION: u32 = 0x0400;
pub const MF_SEPARATOR: u32 = 0x0800;
pub const MF_DEFAULT: u32 = 0x1000;
pub const MF_HELP: u32 = 0x4000;

pub const MFT_BITMAP: u32 = 0x0004;
pub const MFT_MENUBARBREAK: u32 = 0x0020;
pub const MFT_MENUBREAK: u32 = 0x0040;
pub const MFT_OWNERDRAW: u32 = 0x0100;
pub const MFT_RADIOCHECK: u32 = 0x0200;
pub const MFT_SEPARATOR: u32 = 0x0800;
pub const MFT_RIGHTORDER: u32 = 0x2000;
pub const MFT_RIGHTJUSTIFY: u32 = 0x4000;

pub const MFS_GRAYED: u32 = 0x0003;
pub const MFS_CHECKED: u32 = 0x0008;
pub const MFS_HILITE: u32 = 0x0080;
pub const MFS_DEFAULT: u32 = 0x1000;

pub const MIIM_STATE: u32 = 0x0001;
pub const MIIM_ID: u32 = 0x0002;
pub const MIIM_SUBMENU: u32 = 0x0004;
pub const MIIM_CHECKMARKS: u32 = 0x0008;
pub const MIIM_TYPE: u32 = 0x0010;
pub const MIIM_DATA: u32 = 0x0020;
pub const MIIM_STRING: u32 = 0x0040;
pub const MIIM_BITMAP: u32 = 0x0080;
pub const MIIM_FTYPE: u32 = 0x0100;

pub const MIM_MAXHEIGHT: u32 = 0x0000_0001;
pub const MIM_BACKGROUND: u32 = 0x0000_0002;
pub const MIM_HELPID: u32 = 0x0000_0004;
pub const MIM_MENUDATA: u32 = 0x0000_0008;
pub const MIM_STYLE: u32 = 0x0000_0010;
pub const MIM_APPLYTOSUBMENUS: u32 = 0x8000_0000;

/// `GMDI_USEDISABLED`, `GMDI_GOINTOPOPUPS` (`GetMenuDefaultItem`).
pub const GMDI_USEDISABLED: u32 = 1;
pub const GMDI_GOINTOPOPUPS: u32 = 2;

pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INVALID_MENU_HANDLE: u32 = 1401;
pub const ERROR_MENU_ITEM_NOT_FOUND: u32 = 1456;

/// `MENUITEMINFOW` sizes `user32` accepts: the structure without
/// `hbmpItem` (`MENUITEMINFO_V3`... 72 bytes on x64) and with it (80).
pub const MII_SIZE_SHORT: u32 = 72;
pub const MII_SIZE: u32 = 80;
/// `sizeof(MENUINFO)` on x64.
pub const MENUINFO_SIZE: u32 = 40;

/// Handles start here (a table of 32-bit values, never reused).
const FIRST_HANDLE: u32 = 0x0001_0101;
/// Deepest nesting the recursive walks follow.
const MAX_DEPTH: usize = 64;

// ----- items

/// One menu item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    /// `wID`; for a popup item the low 32 bits of the sub menu's handle.
    pub id: u32,
    /// `MFT_*` bits (0 is `MFT_STRING`).
    pub ftype: u32,
    /// `MFS_*` bits.
    pub state: u32,
    /// The popup's handle (0: none).
    pub sub: u32,
    /// The text, if the item has one.
    pub text: Option<Vec<u16>>,
    pub data: usize,
    pub hbm_checked: usize,
    pub hbm_unchecked: usize,
    pub hbm_item: usize,
}

impl Item {
    pub fn is_separator(&self) -> bool {
        self.ftype & MFT_SEPARATOR != 0
    }

    /// An item that has a string to show (`MFT_STRING`: no bitmap, owner
    /// draw or separator type).
    pub fn has_string_type(&self) -> bool {
        self.ftype & (MFT_BITMAP | MFT_OWNERDRAW | MFT_SEPARATOR) == 0
    }

    /// The text the item holds (an item made a separator or owner-draw by
    /// `ModifyMenuW` keeps the string it had).
    pub fn string(&self) -> &[u16] {
        self.text.as_deref().unwrap_or(&[])
    }

    /// The item's `MF_*` state word as `GetMenuState` shows it, given the
    /// number of items of its popup.
    pub fn menu_state(&self, sub_count: usize) -> u32 {
        let mut s = self.state & (MFS_GRAYED | MFS_CHECKED | MFS_HILITE);
        s |= self.ftype & (MFT_BITMAP | MFT_MENUBARBREAK | MFT_MENUBREAK | MFT_OWNERDRAW | MFT_SEPARATOR);
        if self.sub != 0 {
            s |= MF_POPUP | ((sub_count as u32 & 0xFF) << 8);
        }
        s
    }
}

/// One menu.
#[derive(Clone, Debug, Default)]
pub struct Menu {
    pub items: Vec<Item>,
    /// Made by `CreatePopupMenu` (a bar may not be a popup and the reverse).
    pub popup: bool,
    pub style: u32,
    pub cy_max: u32,
    pub help_id: u32,
    pub data: usize,
    pub back: usize,
    /// Popups the menu let go of with `ModifyMenuW`: destroyed with it.
    orphans: Vec<u32>,
}

/// Where an item is named: by position or by command id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sel {
    Pos(u32),
    Cmd(u32),
}

impl Sel {
    /// `uPosition` with the `MF_BYPOSITION` bit of `flags` (0: by command).
    pub fn of(value: u32, flags: u32) -> Sel {
        if flags & MF_BYPOSITION != 0 { Sel::Pos(value) } else { Sel::Cmd(value) }
    }
}

/// What `MENUITEMINFOW` carries (`SetMenuItemInfoW`, `InsertMenuItemW`).
#[derive(Clone, Debug, Default)]
pub struct InfoIn {
    pub mask: u32,
    pub ftype: u32,
    pub state: u32,
    pub id: u32,
    pub sub: u32,
    pub hbm_checked: usize,
    pub hbm_unchecked: usize,
    pub data: usize,
    /// `dwTypeData` read as a string (only when the mask asks for one and it was not NULL).
    pub text: Option<Vec<u16>>,
    pub hbm_item: usize,
}

/// `MENUINFO` as `GetMenuInfo` reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub style: u32,
    pub cy_max: u32,
    pub help_id: u32,
    pub data: usize,
    pub back: usize,
}

/// The handle table.
#[derive(Debug)]
pub struct Menus {
    map: BTreeMap<u32, Menu>,
    next: u32,
}

impl Default for Menus {
    fn default() -> Self {
        Menus::new()
    }
}

/// Whether two UTF-16 names are equal ignoring case (ASCII and the Latin-1
/// range are enough for resource names).
fn upper(c: u16) -> u16 {
    if (0x61..=0x7A).contains(&c) || (0xE0..=0xFE).contains(&c) && c != 0xF7 { c - 0x20 } else { c }
}

/// Compare resource names without regard to case.
pub fn names_equal(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| upper(*x) == upper(*y))
}

impl Menus {
    pub const fn new() -> Menus {
        Menus { map: BTreeMap::new(), next: FIRST_HANDLE }
    }

    pub fn is_menu(&self, h: u32) -> bool {
        h != 0 && self.map.contains_key(&h)
    }

    pub fn get(&self, h: u32) -> Option<&Menu> {
        self.map.get(&h)
    }

    pub fn get_mut(&mut self, h: u32) -> Option<&mut Menu> {
        self.map.get_mut(&h)
    }

    /// `CreateMenu` (`popup` false) and `CreatePopupMenu`.
    pub fn create(&mut self, popup: bool) -> u32 {
        let h = self.next;
        self.next += 1;
        self.map.insert(h, Menu { popup, ..Menu::default() });
        h
    }

    /// `DestroyMenu`: the menu and every popup below it (and the popups it
    /// let go of); false for a handle that is not a menu.
    pub fn destroy(&mut self, h: u32) -> bool {
        if !self.is_menu(h) {
            return false;
        }
        let mut stack = alloc::vec![h];
        let mut guard = 0usize;
        while let Some(m) = stack.pop() {
            guard += 1;
            if guard > 100_000 {
                break;
            }
            if let Some(menu) = self.map.remove(&m) {
                for i in &menu.items {
                    if i.sub != 0 {
                        stack.push(i.sub);
                    }
                }
                stack.extend(menu.orphans.iter().copied());
            }
        }
        true
    }

    pub fn item_count(&self, h: u32) -> Option<usize> {
        self.map.get(&h).map(|m| m.items.len())
    }

    /// The number of items of the popup of `item`.
    fn sub_count(&self, item: &Item) -> usize {
        if item.sub == 0 { 0 } else { self.map.get(&item.sub).map_or(0, |m| m.items.len()) }
    }

    // ----- lookup

    /// The menu and index of the first item with command `id`: the menu's
    /// items in order, a popup item's sub menu searched where the item is
    /// (depth first, pre-order).
    fn find_cmd(&self, h: u32, id: u32, depth: usize) -> Option<(u32, usize)> {
        if depth > MAX_DEPTH {
            return None;
        }
        let m = self.map.get(&h)?;
        for (i, it) in m.items.iter().enumerate() {
            if it.sub != 0 {
                if let Some(r) = self.find_cmd(it.sub, id, depth + 1) {
                    return Some(r);
                }
                if it.id == id && id != u32::MAX {
                    return Some((h, i));
                }
            } else if it.id == id {
                return Some((h, i));
            }
        }
        None
    }

    /// The (menu, index) of an item named by `sel` in menu `h`.
    pub fn find(&self, h: u32, sel: Sel) -> Option<(u32, usize)> {
        let m = self.map.get(&h)?;
        match sel {
            Sel::Pos(p) => ((p as usize) < m.items.len()).then_some((h, p as usize)),
            Sel::Cmd(id) => self.find_cmd(h, id, 0),
        }
    }

    /// The item named by `sel`.
    pub fn item(&self, h: u32, sel: Sel) -> Option<&Item> {
        let (m, i) = self.find(h, sel)?;
        self.map.get(&m)?.items.get(i)
    }

    fn item_mut(&mut self, h: u32, sel: Sel) -> Option<&mut Item> {
        let (m, i) = self.find(h, sel)?;
        self.map.get_mut(&m)?.items.get_mut(i)
    }

    // ----- building items from the `MF_*` form

    /// The item `AppendMenuW`/`InsertMenuW`/`ModifyMenuW` make: `flags`
    /// (`MF_*`), `id` (`uIDNewItem`; a popup's handle with `MF_POPUP`),
    /// `ptr` (`lpNewItem` as a number: item data or a bitmap handle) and
    /// `text` (read from it when it names a string). The second value is
    /// true when `MF_POPUP` named a handle that is not a menu.
    pub fn item_from_flags(&self, flags: u32, id: u64, ptr: u64, text: Option<Vec<u16>>) -> (Item, bool) {
        let mut it = Item { id: id as u32, ..Item::default() };
        let mut bad_popup = false;
        if flags & MF_CHECKED != 0 {
            it.state |= MFS_CHECKED;
        }
        it.state |= flags & (MF_GRAYED | MF_DISABLED);
        if flags & MF_HILITE != 0 {
            it.state |= MFS_HILITE;
        }
        it.ftype |= flags & (MFT_MENUBARBREAK | MFT_MENUBREAK | MFT_RIGHTJUSTIFY);
        if flags & MF_SEPARATOR != 0 {
            it.ftype |= MFT_SEPARATOR;
            it.state |= MFS_GRAYED;
        } else if flags & MF_OWNERDRAW != 0 {
            it.ftype |= MFT_OWNERDRAW;
            it.data = ptr as usize;
        } else if flags & MF_BITMAP != 0 {
            it.ftype |= MFT_BITMAP;
            it.hbm_item = ptr as usize;
        } else if text.is_none() && flags & MF_POPUP == 0 {
            // A string item without a string is a separator.
            it.ftype |= MFT_SEPARATOR;
            it.state |= MFS_GRAYED;
        } else {
            it.text = text;
        }
        if flags & MF_POPUP != 0 {
            let h = id as u32;
            if id >> 32 == 0 && self.is_menu(h) {
                it.sub = h;
            } else if id != 0 {
                bad_popup = true;
            }
        }
        (it, bad_popup)
    }

    // ----- AppendMenuW, InsertMenuW, ModifyMenuW, DeleteMenu, RemoveMenu

    /// Add `item` before the item `sel` names (a position past the end, or
    /// an id that is not found, appends).
    pub fn insert_item(&mut self, h: u32, sel: Sel, item: Item) -> Result<(), u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let (m, at) = match sel {
            Sel::Pos(p) => (h, (p as usize).min(self.map[&h].items.len())),
            Sel::Cmd(_) => self.find(h, sel).unwrap_or((h, self.map[&h].items.len())),
        };
        let menu = self.map.get_mut(&m).expect("found");
        menu.items.insert(at, item);
        Ok(())
    }

    /// `AppendMenuW`.
    pub fn append_item(&mut self, h: u32, item: Item) -> Result<(), u32> {
        self.insert_item(h, Sel::Pos(u32::MAX), item)
    }

    /// `ModifyMenuW`: replace the item `sel` names with `new` (the flags
    /// that made it are given for the cases that keep part of the old item).
    pub fn modify_item(&mut self, h: u32, sel: Sel, flags: u32, new: Item) -> Result<(), u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let (m, i) = self.find(h, sel).ok_or(ERROR_MENU_ITEM_NOT_FOUND)?;
        let menu = self.map.get_mut(&m).expect("found");
        let old = core::mem::take(&mut menu.items[i]);
        let mut it = new;
        if flags & MF_POPUP == 0 && old.sub != 0 {
            // An item that held a popup keeps it when the new flags name none
            // (observed: GetSubMenu and GetMenuItemID still show it), and its
            // text when the new item brings none.
            it.sub = old.sub;
            it.id = old.id;
            if it.text.is_none() {
                it.text = old.text.clone();
            }
        } else if it.is_separator() && it.text.is_none() {
            // The string is not cleared.
            it.text = old.text.clone();
        }
        if old.sub != 0 && old.sub != it.sub {
            menu.orphans.push(old.sub);
        }
        menu.items[i] = it;
        Ok(())
    }

    /// `DeleteMenu` (`destroy` true) and `RemoveMenu`: false when the item
    /// is not there; `Err` for a handle that is not a menu.
    pub fn remove_item(&mut self, h: u32, sel: Sel, destroy: bool) -> Result<bool, u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let Some((m, i)) = self.find(h, sel) else { return Ok(false) };
        let it = self.map.get_mut(&m).expect("found").items.remove(i);
        if destroy && it.sub != 0 {
            self.destroy(it.sub);
        }
        Ok(true)
    }

    // ----- queries

    /// `GetSubMenu`: 0 for no popup at `pos`.
    pub fn sub_menu(&self, h: u32, pos: i32) -> Result<u32, u32> {
        let m = self.map.get(&h).ok_or(ERROR_INVALID_MENU_HANDLE)?;
        Ok(usize::try_from(pos).ok().and_then(|p| m.items.get(p)).map_or(0, |i| i.sub))
    }

    /// `GetMenuItemID`: -1 (as `u32::MAX`) for a popup item; `Err(87)` for a
    /// position that is not there.
    pub fn item_id(&self, h: u32, pos: i32) -> Result<u32, u32> {
        let m = self.map.get(&h).ok_or(ERROR_INVALID_MENU_HANDLE)?;
        let it = usize::try_from(pos).ok().and_then(|p| m.items.get(p)).ok_or(ERROR_INVALID_PARAMETER)?;
        Ok(if it.sub != 0 { u32::MAX } else { it.id })
    }

    /// `GetMenuState`: `u32::MAX` for an item that is not there.
    pub fn state(&self, h: u32, sel: Sel) -> Result<u32, u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        Ok(self.item(h, sel).map_or(u32::MAX, |i| i.menu_state(self.sub_count(i))))
    }

    /// `GetMenuStringW`'s text.
    pub fn string(&self, h: u32, sel: Sel) -> Result<Vec<u16>, u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let it = self.item(h, sel).ok_or(ERROR_MENU_ITEM_NOT_FOUND)?;
        if it.sub != 0 && matches!(sel, Sel::Cmd(_)) {
            return Ok(Vec::new());
        }
        Ok(it.string().to_vec())
    }

    /// `CheckMenuItem`: the previous `MF_CHECKED` or `MF_UNCHECKED`; `u32::MAX`
    /// when the item is not there.
    pub fn check_item(&mut self, h: u32, sel: Sel, check: bool) -> Result<u32, u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let Some(it) = self.item_mut(h, sel) else { return Ok(u32::MAX) };
        let was = it.state & MFS_CHECKED;
        if check {
            it.state |= MFS_CHECKED;
        } else {
            it.state &= !MFS_CHECKED;
        }
        Ok(was)
    }

    /// `EnableMenuItem`: `enable` is the `MF_GRAYED | MF_DISABLED` bits; the
    /// previous ones, `u32::MAX` when the item is not there.
    pub fn enable_item(&mut self, h: u32, sel: Sel, gray_bits: u32) -> Result<u32, u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let Some(it) = self.item_mut(h, sel) else { return Ok(u32::MAX) };
        let was = it.state & MFS_GRAYED;
        it.state = (it.state & !MFS_GRAYED) | (gray_bits & MFS_GRAYED);
        Ok(was)
    }

    /// `CheckMenuRadioItem`: `Ok(true)` when the range was applied,
    /// `Ok(false)` when `check` is outside it (the range is unchecked).
    pub fn check_radio(&mut self, h: u32, first: u32, last: u32, check: u32, flags: u32) -> Result<bool, u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let not_found = Err(ERROR_MENU_ITEM_NOT_FOUND);
        let by_pos = flags & MF_BYPOSITION != 0;
        let (m, lo, hi, chk) = if by_pos {
            if first > last {
                return not_found;
            }
            let len = self.map[&h].items.len();
            if check as usize >= len || first as usize >= len {
                return not_found;
            }
            (h, first as usize, (last as usize).min(len - 1), Some(check as usize))
        } else {
            let Some((m1, i1)) = self.find_cmd(h, first, 0) else { return not_found };
            let Some((m2, i2)) = self.find_cmd(h, last, 0) else { return not_found };
            if m1 != m2 || i1 > i2 {
                return not_found;
            }
            let Some((mc, ic)) = self.find_cmd(h, check, 0) else { return not_found };
            (m1, i1, i2, (mc == m1).then_some(ic))
        };
        let menu = self.map.get_mut(&m).expect("found");
        for i in lo..=hi {
            let it = &mut menu.items[i];
            if it.is_separator() || it.sub != 0 {
                continue;
            }
            it.state &= !MFS_CHECKED;
        }
        match chk {
            Some(c) if (lo..=hi).contains(&c) => {
                let it = &mut menu.items[c];
                it.state |= MFS_CHECKED;
                it.ftype |= MFT_RADIOCHECK;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    // ----- MENUITEMINFOW

    /// The item `GetMenuItemInfoW` reads (a copy), with the id it shows.
    pub fn item_info(&self, h: u32, sel: Sel) -> Result<Item, u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let it = self.item(h, sel).ok_or(ERROR_MENU_ITEM_NOT_FOUND)?;
        Ok(it.clone())
    }

    /// The item `InsertMenuItemW` makes from `info`.
    pub fn item_from_info(&self, info: &InfoIn) -> Result<Item, u32> {
        let m = info.mask;
        let mut it = Item::default();
        if m & MIIM_ID != 0 {
            it.id = info.id;
        }
        if m & MIIM_STATE != 0 {
            it.state = info.state;
        }
        if m & MIIM_DATA != 0 {
            it.data = info.data;
        }
        if m & MIIM_CHECKMARKS != 0 {
            it.hbm_checked = info.hbm_checked;
            it.hbm_unchecked = info.hbm_unchecked;
        }
        if m & MIIM_BITMAP != 0 {
            it.hbm_item = info.hbm_item;
        }
        if m & (MIIM_FTYPE | MIIM_TYPE) != 0 {
            it.ftype = info.ftype;
        }
        if m & (MIIM_STRING | MIIM_TYPE) != 0 {
            it.text = info.text.clone();
        }
        if m & MIIM_SUBMENU != 0 && info.sub != 0 {
            if !self.is_menu(info.sub) {
                return Err(ERROR_INVALID_PARAMETER);
            }
            it.sub = info.sub;
            it.id = info.sub;
        }
        if m & (MIIM_STRING | MIIM_TYPE | MIIM_FTYPE | MIIM_SUBMENU | MIIM_BITMAP) == 0 {
            it.ftype |= MFT_SEPARATOR;
            if m & MIIM_STATE == 0 {
                it.state = MFS_GRAYED;
            }
        } else if it.ftype & MFT_SEPARATOR != 0 && m & MIIM_STATE == 0 {
            it.state = MFS_GRAYED;
        }
        Ok(it)
    }

    /// `SetMenuItemInfoW`: change the item `sel` names as `info` says.
    pub fn set_item_info(&mut self, h: u32, sel: Sel, info: &InfoIn) -> Result<(), u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let (m, i) = self.find(h, sel).ok_or(ERROR_MENU_ITEM_NOT_FOUND)?;
        let mask = info.mask;
        if mask & MIIM_SUBMENU != 0 && info.sub != 0 && !self.is_menu(info.sub) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let old_sub = self.map[&m].items[i].sub;
        let mut kill = 0;
        {
            let it = &mut self.map.get_mut(&m).expect("found").items[i];
            if mask & MIIM_ID != 0 && it.sub == 0 {
                it.id = info.id;
            }
            if mask & MIIM_DATA != 0 {
                it.data = info.data;
            }
            if mask & MIIM_CHECKMARKS != 0 {
                it.hbm_checked = info.hbm_checked;
                it.hbm_unchecked = info.hbm_unchecked;
            }
            if mask & MIIM_BITMAP != 0 {
                it.hbm_item = info.hbm_item;
            }
            if mask & (MIIM_FTYPE | MIIM_TYPE) != 0 {
                it.ftype = info.ftype;
                if it.ftype & MFT_SEPARATOR != 0 && mask & MIIM_STATE == 0 {
                    it.state = MFS_GRAYED;
                }
            }
            if mask & (MIIM_STRING | MIIM_TYPE) != 0 && info.text.is_some() {
                it.text = info.text.clone();
            }
            if mask & MIIM_STATE != 0 {
                it.state = info.state;
            }
            if mask & MIIM_SUBMENU != 0 {
                if old_sub != info.sub {
                    kill = old_sub;
                }
                it.sub = info.sub;
                if info.sub != 0 {
                    it.id = info.sub;
                }
            }
        }
        if kill != 0 {
            self.destroy(kill);
        }
        Ok(())
    }

    // ----- MENUINFO

    pub fn info(&self, h: u32) -> Result<Info, u32> {
        let m = self.map.get(&h).ok_or(ERROR_INVALID_MENU_HANDLE)?;
        Ok(Info { style: m.style, cy_max: m.cy_max, help_id: m.help_id, data: m.data, back: m.back })
    }

    /// `SetMenuInfo`: what `mask` names; with `MIM_APPLYTOSUBMENUS` also
    /// the popups below (at any depth).
    pub fn set_info(&mut self, h: u32, mask: u32, v: &Info) -> Result<(), u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let mut todo = alloc::vec![h];
        let mut n = 0;
        while let Some(x) = todo.pop() {
            n += 1;
            if n > 10_000 {
                break;
            }
            let Some(m) = self.map.get_mut(&x) else { continue };
            if mask & MIM_STYLE != 0 {
                m.style = v.style;
            }
            if mask & MIM_MAXHEIGHT != 0 {
                m.cy_max = v.cy_max;
            }
            if mask & MIM_HELPID != 0 {
                m.help_id = v.help_id;
            }
            if mask & MIM_MENUDATA != 0 {
                m.data = v.data;
            }
            if mask & MIM_BACKGROUND != 0 {
                m.back = v.back;
            }
            if mask & MIM_APPLYTOSUBMENUS != 0 {
                todo.extend(m.items.iter().filter(|i| i.sub != 0).map(|i| i.sub));
            }
        }
        Ok(())
    }

    // ----- the default item

    /// `GetMenuDefaultItem`: the id (or position) of the default item, -1 when
    /// there is none.
    pub fn default_item(&self, h: u32, by_pos: bool, flags: u32) -> Result<u32, u32> {
        let m = self.map.get(&h).ok_or(ERROR_INVALID_MENU_HANDLE)?;
        for (pos, it) in m.items.iter().enumerate() {
            if it.state & MFS_DEFAULT == 0 {
                continue;
            }
            if it.state & MFS_GRAYED != 0 && flags & GMDI_USEDISABLED == 0 {
                continue;
            }
            if it.sub != 0 && flags & GMDI_GOINTOPOPUPS != 0 {
                if let Ok(r) = self.default_item(it.sub, by_pos, flags)
                    && r != u32::MAX
                {
                    return Ok(r);
                }
                continue;
            }
            return Ok(if by_pos { pos as u32 } else { it.id });
        }
        Ok(u32::MAX)
    }

    /// `SetMenuDefaultItem`: false when the item is not there.
    pub fn set_default_item(&mut self, h: u32, item: u32, by_pos: bool) -> Result<bool, u32> {
        if !self.is_menu(h) {
            return Err(ERROR_INVALID_MENU_HANDLE);
        }
        let sel = if by_pos { Sel::Pos(item) } else { Sel::Cmd(item) };
        let target = if item == u32::MAX { None } else { self.find(h, sel) };
        if item != u32::MAX && target.is_none() {
            return Ok(false);
        }
        let menu = self.map.get_mut(&h).expect("menu");
        for it in &mut menu.items {
            it.state &= !MFS_DEFAULT;
        }
        if let Some((m, i)) = target {
            self.map.get_mut(&m).expect("found").items[i].state |= MFS_DEFAULT;
        }
        Ok(true)
    }

    // ----- resources

    /// Make the menu a menu resource describes (`LoadMenuW`,
    /// `LoadMenuIndirectW`): the handle of a new menu bar with its popups.
    pub fn load_template(&mut self, bytes: &[u8]) -> Option<u32> {
        let items = parse_template(bytes)?;
        Some(self.build(&items, false))
    }

    fn build(&mut self, items: &[TItem], popup: bool) -> u32 {
        let h = self.create(popup);
        for t in items {
            let mut it =
                Item { id: t.id, ftype: t.ftype, state: t.state, text: Some(t.text.clone()), ..Item::default() };
            if let Some(sub) = &t.sub {
                let s = self.build(sub, true);
                it.sub = s;
                it.id = s;
            } else if t.ftype & MFT_SEPARATOR != 0 {
                it.text = None;
            }
            self.map.get_mut(&h).expect("new").items.push(it);
        }
        h
    }
}

// ----- the menu resource

/// One item of a menu resource.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TItem {
    pub ftype: u32,
    pub state: u32,
    pub id: u32,
    pub text: Vec<u16>,
    /// The popup's items (`None`: not a popup).
    pub sub: Option<Vec<TItem>>,
}

struct Rd<'a> {
    b: &'a [u8],
    at: usize,
}

impl Rd<'_> {
    fn u16(&mut self) -> Option<u16> {
        let v = self.b.get(self.at..self.at + 2)?;
        self.at += 2;
        Some(u16::from_le_bytes([v[0], v[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        let v = self.b.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
    }

    fn string(&mut self) -> Option<Vec<u16>> {
        let mut s = Vec::new();
        loop {
            let c = self.u16()?;
            if c == 0 {
                return Some(s);
            }
            s.push(c);
        }
    }

    fn align4(&mut self) {
        self.at = (self.at + 3) & !3;
    }
}

/// Parse an `RT_MENU` resource: the old format (`MENUITEMTEMPLATE`, version
/// 0) or the extended one (`MENUEX_TEMPLATE_ITEM`, version 1) (Learn, "Menu
/// Resource Formats" under "Resource Formats").
pub fn parse_template(bytes: &[u8]) -> Option<Vec<TItem>> {
    let mut r = Rd { b: bytes, at: 0 };
    let version = r.u16()?;
    let offset = r.u16()? as usize;
    match version {
        0 => {
            r.at = 4 + offset;
            parse_old(&mut r, 0)
        }
        1 => {
            // The header continues with dwHelpId; the first item follows `offset` bytes after wOffset.
            r.at = 4 + offset;
            r.align4();
            parse_ex(&mut r, 0)
        }
        _ => None,
    }
}

fn parse_old(r: &mut Rd<'_>, depth: usize) -> Option<Vec<TItem>> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut out = Vec::new();
    loop {
        let opt = u32::from(r.u16()?);
        let popup = opt & MF_POPUP != 0;
        let id = if popup { 0 } else { u32::from(r.u16()?) };
        let text = r.string()?;
        let mut t = TItem {
            ftype: opt & (MFT_MENUBARBREAK | MFT_MENUBREAK | MFT_RIGHTJUSTIFY),
            state: opt & (MF_GRAYED | MF_DISABLED | MF_CHECKED),
            id,
            text,
            sub: None,
        };
        if popup {
            t.sub = Some(parse_old(r, depth + 1)?);
        } else if id == 0 && t.text.is_empty() && opt & (MF_GRAYED | MF_DISABLED | MF_CHECKED) == 0 {
            t.ftype |= MFT_SEPARATOR;
            t.state = MFS_GRAYED;
        }
        out.push(t);
        if opt & MF_END != 0 {
            return Some(out);
        }
    }
}

fn parse_ex(r: &mut Rd<'_>, depth: usize) -> Option<Vec<TItem>> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut out = Vec::new();
    loop {
        let ftype = r.u32()?;
        let state = r.u32()?;
        let id = r.u32()?;
        let res = r.u16()?;
        let text = r.string()?;
        r.align4();
        let mut t = TItem { ftype, state, id, text, sub: None };
        if ftype & MFT_SEPARATOR != 0 {
            t.state |= MFS_GRAYED;
        }
        if res & 1 != 0 {
            let _help = r.u32()?;
            t.sub = Some(parse_ex(r, depth + 1)?);
        }
        out.push(t);
        if res & 0x80 != 0 {
            return Some(out);
        }
    }
}

// ----- layout (what the drawing of a menu needs, without a font)

/// The size and place of one item of a menu being drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Cell {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// The metrics a layout needs (Alpine Shell's look, `metrics.rs` for the bar).
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// A line of menu text, a separator, and the padding of an item (each side).
    pub row: i32,
    pub separator: i32,
    pub pad_x: i32,
    /// Room for the check mark at the left of a popup's items, for the arrow at the right.
    pub check: i32,
    pub arrow: i32,
    /// The gap between an item's text and its accelerator text.
    pub gap: i32,
    /// The border of a popup.
    pub border: i32,
}

impl Metrics {
    pub const POLY: Metrics = Metrics { row: 24, separator: 9, pad_x: 12, check: 22, arrow: 16, gap: 24, border: 1 };
}

/// The text of an item and the text after its tab (the accelerator).
pub fn split_tab(text: &[u16]) -> (&[u16], &[u16]) {
    match text.iter().position(|&c| c == 9) {
        Some(p) => (&text[..p], &text[p + 1..]),
        None => (text, &[]),
    }
}

/// The mnemonic of a text: the character after the first single `&`
/// (upper case), `&&` is a literal ampersand.
pub fn mnemonic(text: &[u16]) -> Option<u16> {
    let mut i = 0;
    while i < text.len() {
        if text[i] == u16::from(b'&') {
            match text.get(i + 1) {
                Some(&c) if c == u16::from(b'&') => i += 2,
                Some(&c) => return Some(upper(c)),
                None => return None,
            }
        } else {
            i += 1;
        }
    }
    None
}

/// A popup's layout: the size of the window and the cell of each item, in the
/// popup's client coordinates. `width_of` measures a text (without the `&`
/// that marks a mnemonic).
pub fn popup_layout(items: &[Item], m: &Metrics, width_of: &dyn Fn(&[u16]) -> i32) -> (i32, i32, Vec<Cell>) {
    let mut label = 0;
    let mut accel = 0;
    for it in items {
        if it.is_separator() {
            continue;
        }
        let (a, b) = split_tab(it.string());
        label = label.max(width_of(&strip_amp(a)));
        if !b.is_empty() {
            accel = accel.max(width_of(b));
        }
    }
    let inner = m.check + label + if accel > 0 { m.gap + accel } else { 0 } + m.arrow + 2 * m.pad_x;
    let width = inner + 2 * m.border;
    let mut y = m.border + 2;
    let mut cells = Vec::with_capacity(items.len());
    for it in items {
        let h = if it.is_separator() { m.separator } else { m.row };
        cells.push(Cell { left: m.border, top: y, right: width - m.border, bottom: y + h });
        y += h;
    }
    (width, y + m.border + 2, cells)
}

/// A menu bar's layout in a client width: the cells of the items on one row
/// (items that do not fit wrap to another row). Returns the cells and the
/// number of rows.
pub fn bar_layout(
    items: &[Item],
    row: i32,
    pad_x: i32,
    width: i32,
    width_of: &dyn Fn(&[u16]) -> i32,
) -> (Vec<Cell>, i32) {
    let mut cells = Vec::with_capacity(items.len());
    let (mut x, mut y, mut rows) = (0, 0, 1);
    for it in items {
        let w = if it.is_separator() { 8 } else { width_of(&strip_amp(it.string())) + 2 * pad_x };
        if it.ftype & MFT_RIGHTJUSTIFY != 0 {
            // The help item goes to the right edge of its row.
            x = (width - w).max(x);
        }
        if x + w > width && x > 0 {
            x = 0;
            y += row;
            rows += 1;
        }
        cells.push(Cell { left: x, top: y, right: x + w, bottom: y + row });
        x += w;
    }
    (cells, rows)
}

/// The text with `&x` shown as `x` and `&&` as `&`.
pub fn strip_amp(t: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        if t[i] == u16::from(b'&') {
            if let Some(&c) = t.get(i + 1) {
                out.push(c);
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        out.push(t[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn add(ms: &mut Menus, h: u32, flags: u32, id: u64, text: Option<&str>) {
        let (it, _) = ms.item_from_flags(flags, id, 0, text.map(w));
        ms.append_item(h, it).unwrap();
    }

    #[test]
    fn handles_are_distinct_and_never_reused() {
        let mut ms = Menus::new();
        let a = ms.create(false);
        let b = ms.create(true);
        assert_ne!(a, b);
        assert!(ms.destroy(a));
        assert!(!ms.is_menu(a));
        assert!(!ms.destroy(a));
        let c = ms.create(false);
        assert_ne!(c, a);
    }

    #[test]
    fn destroy_follows_popups_but_not_removed_ones() {
        let mut ms = Menus::new();
        let a = ms.create(false);
        let c = ms.create(true);
        let d = ms.create(true);
        let p = ms.create(true);
        add(&mut ms, c, MF_POPUP, u64::from(d), Some("d"));
        add(&mut ms, a, MF_POPUP, u64::from(c), Some("c"));
        add(&mut ms, a, MF_POPUP, u64::from(p), Some("p"));
        assert_eq!(ms.remove_item(a, Sel::Pos(1), false), Ok(true));
        assert!(ms.destroy(a));
        assert!(!ms.is_menu(c) && !ms.is_menu(d));
        assert!(ms.is_menu(p));
    }

    #[test]
    fn by_command_search_is_depth_first() {
        let mut ms = Menus::new();
        let top = ms.create(true);
        let s = ms.create(true);
        add(&mut ms, s, MF_STRING_FLAG, 5, Some("inner"));
        add(&mut ms, top, MF_POPUP, u64::from(s), Some("sub"));
        add(&mut ms, top, MF_STRING_FLAG, 5, Some("outer"));
        assert_eq!(ms.find(top, Sel::Cmd(5)), Some((s, 0)));
        ms.remove_item(top, Sel::Cmd(5), true).unwrap();
        assert_eq!(ms.find(top, Sel::Cmd(5)), Some((top, 1)));
        assert_eq!(ms.find(top, Sel::Cmd(77)), None);
    }

    const MF_STRING_FLAG: u32 = 0;

    #[test]
    fn items_from_flags() {
        let ms = Menus::new();
        let (it, bad) = ms.item_from_flags(MF_SEPARATOR, 7, 0, Some(w("ignored")));
        assert!(!bad && it.is_separator() && it.state == 3 && it.id == 7);
        let (it, _) = ms.item_from_flags(0, 1, 0, None);
        assert!(it.is_separator());
        let (it, _) = ms.item_from_flags(0, 1, 0, Some(vec![]));
        assert!(!it.is_separator());
        let (it, _) = ms.item_from_flags(MF_OWNERDRAW | MF_CHECKED, 3, 0x1234, None);
        assert_eq!((it.ftype, it.state, it.data), (MFT_OWNERDRAW, MFS_CHECKED, 0x1234));
        let (it, bad) = ms.item_from_flags(MF_POPUP, 0xbad0bad, 0, Some(w("x")));
        assert!(bad && it.sub == 0 && it.id == 0xbad0bad);
        let (it, bad) = ms.item_from_flags(MF_POPUP, 0, 0, Some(w("x")));
        assert!(!bad && it.sub == 0);
        let (it, _) = ms.item_from_flags(0, 0x1_0000_0007, 0, Some(w("x")));
        assert_eq!(it.id, 7);
    }

    #[test]
    fn state_words() {
        let mut ms = Menus::new();
        let p = ms.create(true);
        let s = ms.create(true);
        add(&mut ms, s, 0, 1, Some("a"));
        add(&mut ms, s, 0, 2, Some("b"));
        add(&mut ms, p, MF_SEPARATOR, 6, None);
        add(&mut ms, p, MF_POPUP | MF_GRAYED, u64::from(s), Some("pop"));
        add(&mut ms, p, MF_CHECKED | MF_MENUBREAK, 9, Some("x"));
        assert_eq!(ms.state(p, Sel::Pos(0)), Ok(0x803));
        assert_eq!(ms.state(p, Sel::Cmd(6)), Ok(0x803));
        assert_eq!(ms.state(p, Sel::Pos(1)), Ok(0x211));
        assert_eq!(ms.state(p, Sel::Pos(2)), Ok(0x48));
        assert_eq!(ms.state(p, Sel::Pos(9)), Ok(u32::MAX));
        assert_eq!(ms.state(999, Sel::Pos(0)), Err(ERROR_INVALID_MENU_HANDLE));
    }

    #[test]
    fn check_and_enable_return_previous() {
        let mut ms = Menus::new();
        let p = ms.create(true);
        add(&mut ms, p, 0, 1, Some("a"));
        assert_eq!(ms.check_item(p, Sel::Cmd(1), true), Ok(0));
        assert_eq!(ms.check_item(p, Sel::Cmd(1), true), Ok(8));
        assert_eq!(ms.check_item(p, Sel::Cmd(2), true), Ok(u32::MAX));
        assert_eq!(ms.enable_item(p, Sel::Cmd(1), MF_GRAYED), Ok(0));
        assert_eq!(ms.enable_item(p, Sel::Cmd(1), MF_DISABLED), Ok(1));
        assert_eq!(ms.enable_item(p, Sel::Cmd(1), MF_ENABLED), Ok(2));
        assert_eq!(ms.item(p, Sel::Pos(0)).unwrap().state, 8);
    }

    #[test]
    fn radio_groups() {
        let mut ms = Menus::new();
        let p = ms.create(true);
        for i in 0..3 {
            add(&mut ms, p, 0, 10 + i, Some("g"));
        }
        assert_eq!(ms.check_radio(p, 10, 12, 11, MF_BYCOMMAND), Ok(true));
        assert_eq!(ms.item(p, Sel::Pos(1)).unwrap().state, 8);
        assert_eq!(ms.item(p, Sel::Pos(1)).unwrap().ftype, MFT_RADIOCHECK);
        assert_eq!(ms.item(p, Sel::Pos(0)).unwrap().ftype, 0);
        assert_eq!(ms.check_radio(p, 10, 12, 99, MF_BYCOMMAND), Err(ERROR_MENU_ITEM_NOT_FOUND));
        assert_eq!(ms.check_radio(p, 10, 11, 12, MF_BYCOMMAND), Ok(false));
        assert_eq!(ms.item(p, Sel::Pos(1)).unwrap().state, 0);
        assert_eq!(ms.check_radio(p, 5, 6, 5, MF_BYPOSITION), Err(ERROR_MENU_ITEM_NOT_FOUND));
        assert_eq!(ms.check_radio(p, 0, 99, 2, MF_BYPOSITION), Ok(true));
    }

    #[test]
    fn modify_keeps_a_popup_the_flags_do_not_replace() {
        let mut ms = Menus::new();
        let p = ms.create(true);
        let s = ms.create(true);
        let t = ms.create(true);
        add(&mut ms, p, MF_POPUP, u64::from(s), Some("sub"));
        let (it, _) = ms.item_from_flags(0, 9, 0, Some(w("str")));
        ms.modify_item(p, Sel::Pos(0), 0, it).unwrap();
        // The item is still a popup item (observed on Windows), with the new text.
        assert_eq!(ms.sub_menu(p, 0), Ok(s));
        assert_eq!(ms.item_id(p, 0), Ok(u32::MAX));
        assert_eq!(ms.item(p, Sel::Pos(0)).unwrap().text.as_deref(), Some(&w("str")[..]));
        // Owner draw keeps the text.
        let (it, _) = ms.item_from_flags(MF_OWNERDRAW, 7, 0x99, None);
        ms.modify_item(p, Sel::Pos(0), MF_OWNERDRAW, it).unwrap();
        let k = ms.item(p, Sel::Pos(0)).unwrap();
        assert_eq!((k.sub, k.ftype, k.data), (s, MFT_OWNERDRAW, 0x99));
        assert_eq!(k.text.as_deref(), Some(&w("str")[..]));
        // A new popup replaces it; the old one stays alive until the menu goes.
        let (it, _) = ms.item_from_flags(MF_POPUP, u64::from(t), 0, Some(w("t")));
        ms.modify_item(p, Sel::Pos(0), MF_POPUP, it).unwrap();
        assert_eq!(ms.sub_menu(p, 0), Ok(t));
        assert!(ms.is_menu(s));
        assert!(ms.destroy(p));
        assert!(!ms.is_menu(s) && !ms.is_menu(t));
    }

    #[test]
    fn set_item_info_replaces_a_popup_and_destroys_the_old_one() {
        let mut ms = Menus::new();
        let p = ms.create(true);
        let a = ms.create(true);
        let b = ms.create(true);
        add(&mut ms, p, 0, 7, Some("plain"));
        let info = InfoIn { mask: MIIM_SUBMENU, sub: a, ..InfoIn::default() };
        ms.set_item_info(p, Sel::Pos(0), &info).unwrap();
        let info = InfoIn { mask: MIIM_SUBMENU, sub: b, ..InfoIn::default() };
        ms.set_item_info(p, Sel::Pos(0), &info).unwrap();
        assert!(!ms.is_menu(a) && ms.is_menu(b));
        let info = InfoIn { mask: MIIM_ID, id: 99, ..InfoIn::default() };
        ms.set_item_info(p, Sel::Pos(0), &info).unwrap();
        assert_eq!(ms.item_id(p, 0), Ok(u32::MAX));
        let bad = InfoIn { mask: MIIM_SUBMENU, sub: 12345, ..InfoIn::default() };
        assert_eq!(ms.set_item_info(p, Sel::Pos(0), &bad), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn item_info_without_text_is_a_separator() {
        let ms = Menus::new();
        let it = ms.item_from_info(&InfoIn { mask: MIIM_ID, id: 12, ..InfoIn::default() }).unwrap();
        assert!(it.is_separator() && it.state == 3 && it.id == 12);
        let it = ms.item_from_info(&InfoIn { mask: MIIM_STRING, text: Some(w("x")), ..InfoIn::default() }).unwrap();
        assert!(!it.is_separator() && it.id == 0);
        let it = ms
            .item_from_info(&InfoIn { mask: MIIM_FTYPE | MIIM_ID, ftype: MFT_SEPARATOR, id: 13, ..InfoIn::default() })
            .unwrap();
        assert_eq!((it.ftype, it.state, it.id), (MFT_SEPARATOR, 3, 13));
    }

    #[test]
    fn menu_info_applies_to_submenus() {
        let mut ms = Menus::new();
        let p = ms.create(true);
        let s = ms.create(true);
        add(&mut ms, p, MF_POPUP, u64::from(s), Some("s"));
        ms.set_info(p, MIM_MAXHEIGHT | MIM_HELPID, &Info { cy_max: 77, help_id: 5, ..Info::default() }).unwrap();
        ms.set_info(p, MIM_STYLE | MIM_APPLYTOSUBMENUS, &Info { style: 0x4000_0000, ..Info::default() }).unwrap();
        assert_eq!(ms.info(p).unwrap(), Info { style: 0x4000_0000, cy_max: 77, help_id: 5, ..Info::default() });
        assert_eq!(ms.info(s).unwrap(), Info { style: 0x4000_0000, ..Info::default() });
    }

    #[test]
    fn old_format_template() {
        // Version 0, offset 0; "&File" popup { "Open" id 301; separator; "Exit" id 305 (end) }; "Help" id 9 (end, help).
        let mut b: Vec<u8> = vec![0, 0, 0, 0];
        let put = |b: &mut Vec<u8>, opt: u16, id: Option<u16>, s: &str| {
            b.extend(opt.to_le_bytes());
            if let Some(id) = id {
                b.extend(id.to_le_bytes());
            }
            for c in s.encode_utf16() {
                b.extend(c.to_le_bytes());
            }
            b.extend([0, 0]);
        };
        put(&mut b, MF_POPUP as u16, None, "&File");
        put(&mut b, 0, Some(301), "&Open\tCtrl+O");
        put(&mut b, 0, Some(0), "");
        put(&mut b, (MF_END | MF_GRAYED) as u16, Some(305), "E&xit");
        put(&mut b, (MF_END | MF_HELP) as u16, Some(9), "&Help");
        let t = parse_template(&b).unwrap();
        assert_eq!(t.len(), 2);
        let sub = t[0].sub.as_ref().unwrap();
        assert_eq!(sub.len(), 3);
        assert_eq!(sub[1].ftype, MFT_SEPARATOR);
        assert_eq!(sub[1].state, 3);
        assert_eq!(sub[2].state, 1);
        assert_eq!(t[1].ftype, MFT_RIGHTJUSTIFY);
        let mut ms = Menus::new();
        let h = ms.load_template(&b).unwrap();
        assert_eq!(ms.item_count(h), Some(2));
        assert_ne!(ms.sub_menu(h, 0), Ok(0));
    }

    #[test]
    fn extended_template() {
        let mut b: Vec<u8> = vec![1, 0, 4, 0, 0, 0, 0, 0];
        let put = |b: &mut Vec<u8>, ty: u32, st: u32, id: u32, res: u16, s: &str| {
            b.extend(ty.to_le_bytes());
            b.extend(st.to_le_bytes());
            b.extend(id.to_le_bytes());
            b.extend(res.to_le_bytes());
            for c in s.encode_utf16() {
                b.extend(c.to_le_bytes());
            }
            b.extend([0, 0]);
            while !b.len().is_multiple_of(4) {
                b.push(0);
            }
        };
        put(&mut b, 0, 0, 500, 0, "Plain");
        put(&mut b, MFT_RADIOCHECK, MFS_CHECKED, 501, 0, "Radio");
        put(&mut b, 0, 0, 0, 1, "Menu");
        b.extend(0u32.to_le_bytes());
        put(&mut b, 0, 0, 510, 0, "Sub 1");
        put(&mut b, MFT_MENUBREAK, 0, 511, 0x80, "Sub 2");
        put(&mut b, MFT_RIGHTJUSTIFY, 0, 505, 0x80, "Right");
        let t = parse_template(&b).unwrap();
        assert_eq!(t.len(), 4);
        assert_eq!(t[1].ftype, MFT_RADIOCHECK);
        assert_eq!(t[2].sub.as_ref().map(|s| s.len()), Some(2));
        assert_eq!(t[3].ftype, MFT_RIGHTJUSTIFY);
    }

    #[test]
    fn mnemonics() {
        assert_eq!(mnemonic(&w("&File")), Some(u16::from(b'F')));
        assert_eq!(mnemonic(&w("Sa&ve")), Some(u16::from(b'V')));
        assert_eq!(mnemonic(&w("A && B")), None);
        assert_eq!(strip_amp(&w("A && &B")), w("A & B"));
        assert!(names_equal(&w("appmenu"), &w("APPMENU")));
    }

    #[test]
    fn layouts() {
        let items: Vec<Item> =
            ["File", "Edit", "Help"].iter().map(|t| Item { text: Some(w(t)), ..Item::default() }).collect();
        let (cells, rows) = bar_layout(&items, 20, 8, 300, &|t| 7 * t.len() as i32);
        assert_eq!(rows, 1);
        assert_eq!(cells[0].left, 0);
        assert_eq!(cells[1].left, cells[0].right);
        let (cells, rows) = bar_layout(&items, 20, 8, 60, &|t| 7 * t.len() as i32);
        assert!(rows > 1 && cells[2].top > 0);
        let (wd, ht, cells) = popup_layout(&items, &Metrics::POLY, &|t| 7 * t.len() as i32);
        assert!(wd > 60 && ht > 3 * 24);
        assert_eq!(cells.len(), 3);
    }
}
