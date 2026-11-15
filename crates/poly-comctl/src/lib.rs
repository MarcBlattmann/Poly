//! The state of the common controls of comctl32 5.82 and the rules their basic messages follow
//! (`specs/windows/comctl32.dll/InitCommonControlsEx.yaml`, `messages_5_82`; Learn, "Common
//! Controls"): the status bar's parts and texts, the progress bar, the trackbar, the up-down
//! control, the tab control, the list view's columns and items, the tree view's items and the
//! counts of the header, toolbar and rebar. No window handling is here: `comctl32.dll` decodes
//! the messages (their numbers and structures are from `commctrl.h`) and calls these.

#![no_std]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

/// The class names of the controls the DLL registers, with what a window of each class starts as.
pub mod class {
    /// What creating a window of a class changes.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Make {
        /// Style bits added (`CCS_TOP` for the toolbar, `WS_CLIPSIBLINGS` for the tab control, ...).
        pub add_style: u32,
        /// Style bits removed.
        pub drop_style: u32,
        /// The window text the control keeps: `None`: what it was made with.
        pub text: Option<&'static str>,
        /// The size the control gives itself instead of the one asked for.
        pub size: Size,
    }

    /// How a control sizes itself at creation.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Size {
        /// As asked.
        Asked,
        /// The parent's client width and this height (toolbar, rebar), at the top.
        TopBar(i32),
        /// The parent's client width and this height at the bottom (status bar).
        BottomBar(i32),
        /// The width of a scroll bar button and the height asked (the up-down control).
        ScrollWidth,
        /// The width asked and this height (the combo box of `ComboBoxEx32`).
        Height(i32),
    }

    pub const CCS_TOP: u32 = 0x0001;
    pub const CCS_BOTTOM: u32 = 0x0003;
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const WS_BORDER: u32 = 0x0080_0000;
    pub const WS_CHILD: u32 = 0x4000_0000;
    pub const WS_CLIPSIBLINGS: u32 = 0x0400_0000;

    const fn plain() -> Make {
        Make { add_style: 0, drop_style: 0, text: None, size: Size::Asked }
    }

    /// The common control classes of version 5.82, by name.
    pub const NAMES: &[&str] = &[
        "SysListView32",
        "SysTreeView32",
        "ToolbarWindow32",
        "msctls_statusbar32",
        "SysTabControl32",
        "msctls_progress32",
        "msctls_trackbar32",
        "msctls_updown32",
        "ReBarWindow32",
        "SysHeader32",
        "tooltips_class32",
        "SysAnimate32",
        "msctls_hotkey32",
        "SysDateTimePick32",
        "SysMonthCal32",
        "ComboBoxEx32",
        "SysIPAddress32",
        "SysPager",
        "NativeFontCtl",
    ];

    /// What a window of class `name` (any case) is made as; `None`: not a common control class here.
    pub fn make(name: &str) -> Option<Make> {
        let n = NAMES.iter().find(|c| c.eq_ignore_ascii_case(name))?;
        Some(match *n {
            "ToolbarWindow32" => Make { add_style: CCS_TOP, text: Some(""), size: Size::TopBar(28), ..plain() },
            "ReBarWindow32" => Make { add_style: CCS_TOP, text: Some(""), size: Size::TopBar(0), ..plain() },
            "msctls_statusbar32" => Make { size: Size::BottomBar(23), ..plain() },
            "SysTabControl32" => Make { add_style: WS_CLIPSIBLINGS, ..plain() },
            "msctls_updown32" => Make { size: Size::ScrollWidth, ..plain() },
            "tooltips_class32" => {
                Make { add_style: WS_POPUP | WS_BORDER, drop_style: WS_CHILD, text: Some(""), ..plain() }
            }
            "msctls_hotkey32" => Make { text: Some(""), ..plain() },
            "ComboBoxEx32" => Make { text: Some(""), size: Size::Height(24), ..plain() },
            "SysIPAddress32" => Make { text: Some("0.0.0.0"), ..plain() },
            _ => plain(),
        })
    }
}

/// The progress bar (`msctls_progress32`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub lo: i32,
    pub hi: i32,
    pub pos: i32,
    pub step: i32,
}

impl Default for Progress {
    fn default() -> Self {
        Progress { lo: 0, hi: 100, pos: 0, step: 10 }
    }
}

impl Progress {
    /// `PBM_SETRANGE` / `PBM_SETRANGE32`: the old range (low in the low word, high in the high word).
    pub fn set_range(&mut self, lo: i32, hi: i32) -> u32 {
        let old = (self.lo as u32 & 0xFFFF) | (self.hi as u32) << 16;
        self.lo = lo;
        self.hi = hi;
        self.pos = self.pos.clamp(lo.min(hi), hi.max(lo));
        old
    }

    /// `PBM_SETPOS`: the old position; the new one stays inside the range.
    pub fn set_pos(&mut self, pos: i32) -> i32 {
        let old = self.pos;
        self.pos = pos.clamp(self.lo.min(self.hi), self.hi.max(self.lo));
        old
    }

    /// `PBM_DELTAPOS`: the old position.
    pub fn delta_pos(&mut self, d: i32) -> i32 {
        self.set_pos(self.pos.saturating_add(d))
    }

    /// `PBM_SETSTEP`: the old step.
    pub fn set_step(&mut self, step: i32) -> i32 {
        core::mem::replace(&mut self.step, step)
    }

    /// `PBM_STEPIT`: the old position; past the top the bar starts again at the bottom.
    pub fn step_it(&mut self) -> i32 {
        let old = self.pos;
        let next = self.pos.saturating_add(self.step);
        self.pos = if next > self.hi { self.lo } else { next.max(self.lo) };
        old
    }
}

/// The trackbar (`msctls_trackbar32`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trackbar {
    pub lo: i32,
    pub hi: i32,
    pub pos: i32,
    pub page: i32,
    pub line: i32,
}

impl Default for Trackbar {
    fn default() -> Self {
        Trackbar { lo: 0, hi: 100, pos: 0, page: 20, line: 1 }
    }
}

impl Trackbar {
    pub fn set_range(&mut self, lo: i32, hi: i32) {
        self.lo = lo;
        self.hi = hi;
        self.pos = self.pos.clamp(lo.min(hi), hi.max(lo));
    }

    pub fn set_pos(&mut self, pos: i32) {
        self.pos = pos.clamp(self.lo.min(self.hi), self.hi.max(self.lo));
    }
}

/// The up-down control (`msctls_updown32`): `upper` is the value the up arrow goes towards (it may be
/// below `lower`), 100 and 0 until a range is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpDown {
    pub upper: i32,
    pub lower: i32,
    pub pos: i32,
    pub base: i32,
}

impl Default for UpDown {
    fn default() -> Self {
        UpDown { upper: 100, lower: 0, pos: 0, base: 10 }
    }
}

impl UpDown {
    fn bounds(&self) -> (i32, i32) {
        (self.upper.min(self.lower), self.upper.max(self.lower))
    }

    /// `UDM_SETRANGE` / `UDM_SETRANGE32`.
    pub fn set_range(&mut self, upper: i32, lower: i32) {
        self.upper = upper;
        self.lower = lower;
        let (lo, hi) = self.bounds();
        self.pos = self.pos.clamp(lo, hi);
    }

    /// `UDM_SETPOS`: the old position.
    pub fn set_pos(&mut self, pos: i32) -> i32 {
        let (lo, hi) = self.bounds();
        core::mem::replace(&mut self.pos, pos.clamp(lo, hi))
    }
}

/// One tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tab {
    pub text: Vec<u16>,
    pub image: i32,
    pub param: isize,
}

/// The tab control (`SysTabControl32`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tabs {
    pub items: Vec<Tab>,
    pub cur: i32,
}

impl Default for Tabs {
    fn default() -> Self {
        Tabs { items: Vec::new(), cur: -1 }
    }
}

impl Tabs {
    /// `TCM_INSERTITEM`: the index; one past the end (or more) appends. The first tab is selected.
    pub fn insert(&mut self, index: i32, tab: Tab) -> i32 {
        if index < 0 {
            return -1;
        }
        let at = (index as usize).min(self.items.len());
        self.items.insert(at, tab);
        if self.items.len() == 1 {
            self.cur = 0;
        } else if self.cur >= at as i32 {
            self.cur += 1;
        }
        at as i32
    }

    /// `TCM_SETCURSEL`: the old selection (-1: none); an index that is no tab leaves it and gives -1.
    pub fn set_cur_sel(&mut self, index: i32) -> i32 {
        if index < -1 || index >= self.items.len() as i32 {
            return -1;
        }
        core::mem::replace(&mut self.cur, index)
    }

    /// `TCM_DELETEITEM`.
    pub fn delete(&mut self, index: i32) -> bool {
        if index < 0 || index as usize >= self.items.len() {
            return false;
        }
        self.items.remove(index as usize);
        if self.items.is_empty() {
            self.cur = -1;
        } else if self.cur > index {
            self.cur -= 1;
        } else if self.cur == index {
            self.cur = index.min(self.items.len() as i32 - 1);
        }
        true
    }

    /// `TCM_DELETEALLITEMS`.
    pub fn delete_all(&mut self) {
        self.items.clear();
        self.cur = -1;
    }
}

/// The status bar (`msctls_statusbar32`): the right edge of each part (-1: to the end) and its text and type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusBar {
    pub edges: Vec<i32>,
    pub texts: Vec<(Vec<u16>, u32)>,
    pub simple: bool,
}

impl Default for StatusBar {
    fn default() -> Self {
        StatusBar { edges: vec![-1], texts: vec![(Vec::new(), 0)], simple: false }
    }
}

impl StatusBar {
    /// `SB_SETPARTS`: up to 256 parts.
    pub fn set_parts(&mut self, edges: &[i32]) -> bool {
        if edges.is_empty() || edges.len() > 256 {
            return false;
        }
        self.edges = edges.to_vec();
        self.texts.resize(edges.len(), (Vec::new(), 0));
        true
    }

    /// `SB_SETTEXT`: `part` and `kind` as the message packs them (`SBT_*` in the high byte).
    pub fn set_text(&mut self, part: usize, kind: u32, text: &[u16]) -> bool {
        match self.texts.get_mut(part) {
            Some(t) => {
                *t = (text.to_vec(), kind & !0xFF);
                true
            }
            None => false,
        }
    }

    /// `SB_GETTEXT`: the text and its type.
    pub fn text(&self, part: usize) -> Option<&(Vec<u16>, u32)> {
        self.texts.get(part)
    }
}

/// A list view column.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    pub text: Vec<u16>,
    pub width: i32,
}

/// The list view (`SysListView32`): columns and, per item, the text of each sub item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListView {
    pub columns: Vec<Column>,
    pub items: Vec<Vec<Vec<u16>>>,
}

impl ListView {
    /// `LVM_INSERTCOLUMN`: the index (one past the end appends), -1 for a negative one.
    pub fn insert_column(&mut self, index: i32, col: Column) -> i32 {
        if index < 0 {
            return -1;
        }
        let at = (index as usize).min(self.columns.len());
        self.columns.insert(at, col);
        at as i32
    }

    /// `LVM_INSERTITEM`: an item at `index` (pushing the others down; past the end appends).
    pub fn insert_item(&mut self, index: i32, text: &[u16]) -> i32 {
        if index < 0 {
            return -1;
        }
        let at = (index as usize).min(self.items.len());
        self.items.insert(at, vec![text.to_vec()]);
        at as i32
    }

    /// `LVM_SETITEMTEXT`: the text of a sub item (a column beyond the last one fails).
    pub fn set_text(&mut self, item: i32, sub: i32, text: &[u16]) -> bool {
        let (Ok(i), Ok(s)) = (usize::try_from(item), usize::try_from(sub)) else { return false };
        let Some(row) = self.items.get_mut(i) else { return false };
        if s > 255 {
            return false;
        }
        if row.len() <= s {
            row.resize(s + 1, Vec::new());
        }
        row[s] = text.to_vec();
        true
    }

    /// `LVM_GETITEMTEXT`: the text of a sub item (empty for one never set); `None` for no such item.
    pub fn text(&self, item: i32, sub: i32) -> Option<&[u16]> {
        let row = self.items.get(usize::try_from(item).ok()?)?;
        let s = usize::try_from(sub).ok()?;
        Some(row.get(s).map_or(&[][..], |t| t.as_slice()))
    }

    /// `LVM_DELETEITEM`.
    pub fn delete(&mut self, item: i32) -> bool {
        match usize::try_from(item) {
            Ok(i) if i < self.items.len() => {
                self.items.remove(i);
                true
            }
            _ => false,
        }
    }
}

/// A tree view item's handle (never 0).
pub type TreeItem = u32;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Node {
    id: TreeItem,
    parent: TreeItem,
    text: Vec<u16>,
}

/// `TVGN_*` (`commctrl.h`).
pub const TVGN_ROOT: u32 = 0;
pub const TVGN_NEXT: u32 = 1;
pub const TVGN_PREVIOUS: u32 = 2;
pub const TVGN_PARENT: u32 = 3;
pub const TVGN_CHILD: u32 = 4;
pub const TVGN_CARET: u32 = 9;
/// `TVI_ROOT` (`(HTREEITEM)-0x10000`) and the insert positions `TVI_FIRST`, `TVI_LAST`, `TVI_SORT`.
pub const TVI_ROOT: u64 = 0xFFFF_FFFF_FFFF_0000;
pub const TVI_FIRST: u64 = 0xFFFF_FFFF_FFFF_0001;
pub const TVI_LAST: u64 = 0xFFFF_FFFF_FFFF_0002;

/// The tree view (`SysTreeView32`): items in display order, each with its parent; a handle is
/// made once and never reused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeView {
    nodes: Vec<Node>,
    next: TreeItem,
    caret: TreeItem,
}

impl TreeView {
    fn siblings(&self, parent: TreeItem) -> Vec<TreeItem> {
        self.nodes.iter().filter(|n| n.parent == parent).map(|n| n.id).collect()
    }

    fn position(&self, id: TreeItem) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// `TVM_INSERTITEM`: under `parent` (0: the root) after `after` (`TVI_FIRST`, `TVI_LAST`, a sibling).
    pub fn insert(&mut self, parent: TreeItem, after: u64, text: &[u16]) -> TreeItem {
        if parent != 0 && self.position(parent).is_none() {
            return 0;
        }
        self.next += 1;
        let id = self.next;
        let sibs = self.siblings(parent);
        // Where among the siblings.
        let before: Option<TreeItem> = match after {
            TVI_FIRST => sibs.first().copied(),
            TVI_LAST => None,
            a => match sibs.iter().position(|&s| u64::from(s) == a) {
                Some(p) => sibs.get(p + 1).copied(),
                None => None,
            },
        };
        let at = match before {
            Some(b) => self.position(b).unwrap_or(self.nodes.len()),
            None => {
                // After the last sibling and all its descendants.
                match sibs.last() {
                    Some(&last) => self.end_of(last),
                    None => match parent {
                        0 => self.nodes.len(),
                        p => self.end_of(p),
                    },
                }
            }
        };
        self.nodes.insert(at, Node { id, parent, text: text.to_vec() });
        if self.caret == 0 && self.nodes.len() == 1 {
            // The caret stays unset until something selects.
        }
        id
    }

    /// The index after the item and all that hangs below it.
    fn end_of(&self, id: TreeItem) -> usize {
        let Some(p) = self.position(id) else { return self.nodes.len() };
        let mut end = p + 1;
        let mut family = vec![id];
        while end < self.nodes.len() && family.contains(&self.nodes[end].parent) {
            family.push(self.nodes[end].id);
            end += 1;
        }
        end
    }

    /// `TVM_GETCOUNT`.
    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    /// `TVM_GETNEXTITEM`: 0 for none.
    pub fn next_item(&self, kind: u32, item: TreeItem) -> TreeItem {
        match kind {
            TVGN_ROOT => self.siblings(0).first().copied().unwrap_or(0),
            TVGN_CHILD => {
                let p = if item as u64 == TVI_ROOT { 0 } else { item };
                self.siblings(p).first().copied().unwrap_or(0)
            }
            TVGN_PARENT => self.position(item).map_or(0, |i| self.nodes[i].parent),
            TVGN_NEXT | TVGN_PREVIOUS => {
                let Some(i) = self.position(item) else { return 0 };
                let sibs = self.siblings(self.nodes[i].parent);
                let at = sibs.iter().position(|&s| s == item).unwrap_or(0);
                if kind == TVGN_NEXT { sibs.get(at + 1).copied().unwrap_or(0) } else { at.checked_sub(1).map_or(0, |p| sibs[p]) }
            }
            TVGN_CARET => self.caret,
            _ => 0,
        }
    }

    /// `TVM_SELECTITEM` with `TVGN_CARET`.
    pub fn select(&mut self, item: TreeItem) -> bool {
        if item != 0 && self.position(item).is_none() {
            return false;
        }
        self.caret = item;
        true
    }

    /// `TVM_DELETEITEM`: the item and what hangs below it (`TVI_ROOT`: everything).
    pub fn delete(&mut self, item: u64) -> bool {
        if item == TVI_ROOT {
            self.nodes.clear();
            self.caret = 0;
            return true;
        }
        let Some(p) = u32::try_from(item).ok().and_then(|i| self.position(i)) else { return false };
        let end = self.end_of(self.nodes[p].id);
        let gone: Vec<TreeItem> = self.nodes[p..end].iter().map(|n| n.id).collect();
        self.nodes.drain(p..end);
        if gone.contains(&self.caret) {
            self.caret = 0;
        }
        true
    }

    /// The text of an item.
    pub fn text(&self, item: TreeItem) -> Option<&[u16]> {
        self.position(item).map(|i| self.nodes[i].text.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn classes_are_made_as_the_sheet_says() {
        assert_eq!(class::make("ToolbarWindow32").unwrap().add_style, 1);
        assert_eq!(class::make("tooltips_class32").unwrap().add_style, 0x8080_0000);
        assert_eq!(class::make("tooltips_class32").unwrap().drop_style, class::WS_CHILD);
        assert_eq!(class::make("SYSTABCONTROL32").unwrap().add_style, 0x0400_0000);
        assert_eq!(class::make("SysIPAddress32").unwrap().text, Some("0.0.0.0"));
        assert!(class::make("SysLink").is_none());
        assert!(class::make("Button").is_none());
    }

    #[test]
    fn progress_rules() {
        let mut p = Progress::default();
        assert_eq!((p.lo, p.hi, p.step), (0, 100, 10));
        assert_eq!(p.set_pos(150), 0);
        assert_eq!(p.pos, 100);
        assert_eq!(p.set_range(0, 50), 100 << 16);
        assert_eq!(p.pos, 50);
        assert_eq!(p.delta_pos(-20), 50);
        assert_eq!(p.pos, 30);
        assert_eq!(p.set_step(5), 10);
        assert_eq!(p.step_it(), 30);
        assert_eq!(p.pos, 35);
        p.set_pos(48);
        p.step_it();
        assert_eq!(p.pos, 0, "past the top the bar starts again");
    }

    #[test]
    fn trackbar_and_updown_clamp() {
        let mut t = Trackbar::default();
        t.set_pos(500);
        assert_eq!(t.pos, 100);
        t.set_range(10, 20);
        assert_eq!(t.pos, 20);
        let mut u = UpDown::default();
        assert_eq!((u.upper, u.lower), (100, 0));
        u.set_range(10, 0);
        assert_eq!(u.set_pos(99), 0);
        assert_eq!(u.pos, 10);
        assert_eq!(u.set_pos(-5), 10);
        assert_eq!(u.pos, 0);
    }

    #[test]
    fn tabs_select_and_delete() {
        let mut t = Tabs::default();
        assert_eq!(t.cur, -1);
        assert_eq!(t.insert(0, Tab { text: w("a"), ..Tab::default() }), 0);
        assert_eq!(t.cur, 0);
        assert_eq!(t.insert(99, Tab { text: w("b"), ..Tab::default() }), 1);
        assert_eq!(t.insert(0, Tab { text: w("c"), ..Tab::default() }), 0);
        assert_eq!(t.cur, 1, "the selection follows its tab");
        assert_eq!(t.set_cur_sel(2), 1);
        assert_eq!(t.set_cur_sel(7), -1);
        assert_eq!(t.cur, 2);
        assert!(t.delete(0));
        assert_eq!(t.cur, 1);
        assert!(!t.delete(5));
        t.delete_all();
        assert_eq!((t.items.len(), t.cur), (0, -1));
    }

    #[test]
    fn status_bar_parts_and_texts() {
        let mut s = StatusBar::default();
        assert_eq!(s.edges, [-1]);
        assert!(s.set_parts(&[50, 100, -1]));
        assert_eq!(s.texts.len(), 3);
        assert!(s.set_text(1, 0x100, &w("mid")));
        assert_eq!(s.text(1).unwrap().1, 0x100);
        assert!(!s.set_text(3, 0, &w("x")));
        assert!(!s.set_parts(&[]));
    }

    #[test]
    fn list_view_items_and_sub_items() {
        let mut l = ListView::default();
        assert_eq!(l.insert_column(0, Column { text: w("Name"), width: 100 }), 0);
        assert_eq!(l.insert_item(0, &w("one")), 0);
        assert_eq!(l.insert_item(0, &w("zero")), 0);
        assert_eq!(l.text(1, 0).unwrap(), w("one").as_slice());
        assert!(l.set_text(1, 1, &w("sub")));
        assert_eq!(l.text(1, 1).unwrap(), w("sub").as_slice());
        assert_eq!(l.text(0, 3).unwrap(), &[] as &[u16]);
        assert!(l.text(5, 0).is_none());
        assert!(l.delete(0));
        assert_eq!(l.items.len(), 1);
    }

    #[test]
    fn tree_view_navigation() {
        let mut t = TreeView::default();
        let a = t.insert(0, TVI_LAST, &w("a"));
        let b = t.insert(0, TVI_LAST, &w("b"));
        let a1 = t.insert(a, TVI_LAST, &w("a1"));
        let a2 = t.insert(a, TVI_LAST, &w("a2"));
        assert_eq!(t.count(), 4);
        assert_eq!(t.next_item(TVGN_ROOT, 0), a);
        assert_eq!(t.next_item(TVGN_NEXT, a), b);
        assert_eq!(t.next_item(TVGN_CHILD, a), a1);
        assert_eq!(t.next_item(TVGN_NEXT, a1), a2);
        assert_eq!(t.next_item(TVGN_PARENT, a2), a);
        assert_eq!(t.next_item(TVGN_PREVIOUS, b), a);
        assert!(t.select(a2));
        assert_eq!(t.next_item(TVGN_CARET, 0), a2);
        let first = t.insert(0, TVI_FIRST, &w("first"));
        assert_eq!(t.next_item(TVGN_ROOT, 0), first);
        assert!(t.delete(u64::from(a)));
        assert_eq!(t.count(), 2, "a root takes its children");
        assert_eq!(t.next_item(TVGN_CARET, 0), 0);
        assert!(!t.delete(u64::from(a)));
    }
}
