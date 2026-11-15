//! `SysListView32`: items and columns, selection, notifications, hit testing, keyboard and
//! mouse, and painting of the report view (and, simply, the list and icon views)
//! (`specs/windows/comctl32.dll/SysListView32.yaml`; Learn, "List-View Control Reference").
//!
//! State lives in a table behind a spin lock that is never held while a message is sent: every
//! handler works on the state in short steps and sends its notifications in between.

use crate::cc::*;
use crate::header;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use poly_win32_gui::SpinLock;

// ---- styles, messages and notifications (`commctrl.h`)

const LVS_TYPEMASK: u32 = 3;
const LVS_REPORT: u32 = 1;
const LVS_SMALLICON: u32 = 2;
const LVS_LIST: u32 = 3;
const LVS_SINGLESEL: u32 = 4;
const LVS_SHOWSELALWAYS: u32 = 8;
const LVS_OWNERDATA: u32 = 0x1000;
const LVS_NOCOLUMNHEADER: u32 = 0x4000;
const LVS_EX_GRIDLINES: u32 = 1;
const LVS_EX_FULLROWSELECT: u32 = 0x20;
const LVS_EX_CHECKBOXES: u32 = 4;

const LVIF_TEXT: u32 = 1;
const LVIF_IMAGE: u32 = 2;
const LVIF_PARAM: u32 = 4;
const LVIF_STATE: u32 = 8;
const LVIF_INDENT: u32 = 0x10;
const LVIS_FOCUSED: u32 = 1;
const LVIS_SELECTED: u32 = 2;
const LVIS_CUT: u32 = 4;
const LVIS_DROPHILITED: u32 = 8;
const LVIS_STATEIMAGEMASK: u32 = 0xF000;

const LVCF_FMT: u32 = 1;
const LVCF_WIDTH: u32 = 2;
const LVCF_TEXT: u32 = 4;
const LVCF_SUBITEM: u32 = 8;
const LVCF_IMAGE: u32 = 0x10;
const LVCF_ORDER: u32 = 0x20;

const LVIR_BOUNDS: i32 = 0;
const LVIR_ICON: i32 = 1;
const LVIR_LABEL: i32 = 2;
const LVIR_SELECTBOUNDS: i32 = 3;

const LVNI_ALL: u32 = 0;
const LVNI_FOCUSED: u32 = 1;
const LVNI_SELECTED: u32 = 2;
const LVNI_CUT: u32 = 4;
const LVNI_DROPHILITED: u32 = 8;
const LVNI_ABOVE: u32 = 0x100;
const LVNI_BELOW: u32 = 0x200;
const LVNI_TOLEFT: u32 = 0x400;
const LVNI_TORIGHT: u32 = 0x800;

const LVHT_NOWHERE: u32 = 1;
const LVHT_ONITEMICON: u32 = 2;
const LVHT_ONITEMLABEL: u32 = 4;
const LVHT_ONITEMSTATEICON: u32 = 8;
const LVHT_ABOVE: u32 = 8;
const LVHT_BELOW: u32 = 0x10;
const LVHT_TORIGHT: u32 = 0x20;
const LVHT_TOLEFT: u32 = 0x40;

const LVN_FIRST: i32 = -100;
const LVN_ITEMCHANGING: i32 = LVN_FIRST;
const LVN_ITEMCHANGED: i32 = LVN_FIRST - 1;
const LVN_INSERTITEM: i32 = LVN_FIRST - 2;
const LVN_DELETEITEM: i32 = LVN_FIRST - 3;
const LVN_DELETEALLITEMS: i32 = LVN_FIRST - 4;
const LVN_COLUMNCLICK: i32 = LVN_FIRST - 8;
const LVN_ODCACHEHINT: i32 = LVN_FIRST - 13;
const LVN_ITEMACTIVATE: i32 = LVN_FIRST - 14;
const LVN_GETDISPINFOW: i32 = LVN_FIRST - 77;
const LVN_KEYDOWN: i32 = LVN_FIRST - 55;
const HDN_FIRST: i32 = -300;
const HDN_ITEMCHANGINGW: i32 = HDN_FIRST - 20;
const HDN_ITEMCHANGEDW: i32 = HDN_FIRST - 21;
const HDN_ITEMCLICKW: i32 = HDN_FIRST - 22;
const CDDS_PREPAINT: u32 = 1;
const CDDS_ITEM: u32 = 0x10000;
const CDDS_ITEMPREPAINT: u32 = CDDS_ITEM | CDDS_PREPAINT;
const CDRF_NOTIFYITEMDRAW: isize = 0x20;
const CDRF_NEWFONT: isize = 2;
const CDRF_SKIPDEFAULT: isize = 4;

const LPSTR_TEXTCALLBACK: usize = usize::MAX;
const I_IMAGECALLBACK: i32 = -1;

/// `LVITEMW` (v5.82 members).
#[repr(C)]
#[derive(Clone, Copy)]
struct LvItem {
    mask: u32,
    item: i32,
    sub_item: i32,
    state: u32,
    state_mask: u32,
    text: usize,
    text_max: i32,
    image: i32,
    param: isize,
    indent: i32,
    group_id: i32,
    columns: u32,
    pu_columns: usize,
}

/// `LVCOLUMNW`.
#[repr(C)]
#[derive(Clone, Copy)]
struct LvColumn {
    mask: u32,
    fmt: i32,
    cx: i32,
    text: usize,
    text_max: i32,
    sub_item: i32,
    image: i32,
    order: i32,
}

/// `NMLISTVIEW`.
#[repr(C)]
struct NmListView {
    hdr: NmHdr,
    item: i32,
    sub_item: i32,
    new_state: u32,
    old_state: u32,
    changed: u32,
    pt: [i32; 2],
    param: isize,
}

/// `NMLVDISPINFOW`.
#[repr(C)]
struct NmLvDispInfo {
    hdr: NmHdr,
    item: LvItem,
}

/// `NMLVKEYDOWN` (packed: the key is a word, the flags follow).
#[repr(C)]
struct NmLvKeyDown {
    hdr: NmHdr,
    vkey: u16,
    flags: u32,
}

/// `NMLVCACHEHINT`.
#[repr(C)]
struct NmLvCacheHint {
    hdr: NmHdr,
    from: i32,
    to: i32,
}

/// `NMCUSTOMDRAW`.
#[repr(C)]
struct NmCustomDraw {
    hdr: NmHdr,
    stage: u32,
    hdc: usize,
    rc: Rc,
    item_spec: usize,
    item_state: u32,
    item_param: isize,
}

/// `NMLVCUSTOMDRAW`.
#[repr(C)]
struct NmLvCustomDraw {
    nmcd: NmCustomDraw,
    text: u32,
    text_bk: u32,
    sub_item: i32,
    item_type: u32,
    pt: [i32; 2],
    align: u32,
}

#[repr(C)]
struct NmHeader {
    hdr: NmHdr,
    item: i32,
    button: i32,
    pitem: usize,
}

// ---- state

#[derive(Clone, Debug)]
enum Text {
    Str(Vec<u16>),
    Callback,
}

#[derive(Clone, Debug)]
struct Item {
    texts: Vec<Option<Text>>,
    image: i32,
    param: isize,
    state: u32,
    indent: i32,
}

#[derive(Clone, Debug, Default)]
struct Col {
    text: Vec<u16>,
    width: i32,
    fmt: i32,
    sub_item: i32,
    image: i32,
    order: i32,
}

struct Lv {
    cols: Vec<Col>,
    items: Vec<Item>,
    /// An owner data list: the count, the selection and the focus.
    owner: bool,
    count: i32,
    selected: BTreeSet<i32>,
    focus: i32,
    ex: u32,
    bk: u32,
    text: u32,
    text_bk: u32,
    images: [usize; 3],
    callback_mask: u32,
    mark: i32,
    header: usize,
    font: usize,
    top: i32,
    redraw: bool,
    hot: i32,
}

static STATES: SpinLock<BTreeMap<usize, Lv>> = SpinLock::new(BTreeMap::new());

fn with<R>(h: usize, f: impl FnOnce(&mut Lv) -> R) -> Option<R> {
    STATES.with(|m| m.get_mut(&h).map(f))
}

pub fn on_create(hwnd: usize) {
    let style = style_of(hwnd);
    STATES.with(|m| {
        m.insert(
            hwnd,
            Lv {
                cols: Vec::new(),
                items: Vec::new(),
                owner: style & LVS_OWNERDATA != 0,
                count: 0,
                selected: BTreeSet::new(),
                focus: -1,
                ex: 0,
                bk: sys_color(COLOR_WINDOW),
                text: CLR_DEFAULT,
                text_bk: CLR_DEFAULT,
                images: [0; 3],
                callback_mask: 0,
                mark: -1,
                header: 0,
                font: 0,
                top: 0,
                redraw: true,
                hot: -1,
            },
        )
    });
}

/// `WM_CREATE`: a report view has its header from the start.
pub fn on_created(hwnd: usize) {
    if style_of(hwnd) & LVS_TYPEMASK == LVS_REPORT {
        make_header(hwnd);
    }
}

pub fn on_destroy(hwnd: usize) {
    let (items, owner) = with(hwnd, |lv| {
        let n = if lv.owner { lv.count } else { lv.items.len() as i32 };
        (n, lv.owner)
    })
    .unwrap_or((0, false));
    let _ = owner;
    // Every item still there is deleted (its notification first), the last one first.
    for i in (0..items).rev() {
        notify_item(hwnd, LVN_DELETEITEM, i, 0, 0, 0, 0);
    }
    let h = with(hwnd, |lv| lv.header).unwrap_or(0);
    if h != 0 {
        // SAFETY: our child.
        unsafe { DestroyWindow(h) };
    }
    STATES.with(|m| m.remove(&hwnd));
}

fn make_header(hwnd: usize) -> usize {
    let have = with(hwnd, |lv| lv.header).unwrap_or(0);
    if have != 0 {
        return have;
    }
    let class: Vec<u16> = "SysHeader32\0".encode_utf16().collect();
    let hidden = style_of(hwnd) & LVS_NOCOLUMNHEADER != 0;
    // SAFETY: a child window of the list view of a registered class.
    let h = unsafe {
        CreateWindowExW(0, class.as_ptr(), core::ptr::null(), WS_CHILD | if hidden { 0 } else { WS_VISIBLE } | header::HDS_BUTTONS, 0, 0, 0, 0, hwnd, 0, 0, 0)
    };
    with(hwnd, |lv| lv.header = h);
    sync_header(hwnd);
    layout(hwnd);
    h
}

fn sync_header(hwnd: usize) {
    let Some((h, items, font)) = with(hwnd, |lv| {
        (
            lv.header,
            lv.cols.iter().map(|c| header::Item { text: c.text.clone(), width: c.width, fmt: c.fmt, image: c.image, param: 0 }).collect::<Vec<_>>(),
            lv.font,
        )
    }) else {
        return;
    };
    if h != 0 {
        header::set_font(h, font);
        header::sync(h, items);
    }
}

// ---- geometry

fn font_height(hwnd: usize) -> i32 {
    let font = with(hwnd, |lv| lv.font).unwrap_or(0);
    let m = Measure::new(hwnd, font);
    font_metrics(m.hdc).0
}

fn header_height(hwnd: usize) -> i32 {
    if style_of(hwnd) & LVS_NOCOLUMNHEADER != 0 || style_of(hwnd) & LVS_TYPEMASK != LVS_REPORT {
        return 0;
    }
    font_height(hwnd) + 3
}

fn row_height(hwnd: usize) -> i32 {
    let small = with(hwnd, |lv| lv.images[1]).unwrap_or(0);
    let mut h = font_height(hwnd) + 2;
    if small != 0 {
        let (mut cx, mut cy) = (0, 0);
        // SAFETY: our image list functions.
        unsafe { crate::imagelist::ImageList_GetIconSize(small, &mut cx, &mut cy) };
        h = h.max(cy + 1);
    }
    h.max(17)
}

fn layout(hwnd: usize) {
    let h = with(hwnd, |lv| lv.header).unwrap_or(0);
    if h == 0 {
        return;
    }
    let c = client_rect(hwnd);
    let total: i32 = with(hwnd, |lv| lv.cols.iter().map(|c| c.width).sum()).unwrap_or(0);
    // SAFETY: our child window.
    unsafe { SetWindowPos(h, 0, 0, 0, rc_w(&c).max(total), header_height(hwnd).max(1), 0x14) };
}

fn item_count(hwnd: usize) -> i32 {
    with(hwnd, |lv| if lv.owner { lv.count } else { lv.items.len() as i32 }).unwrap_or(0)
}

fn total_width(hwnd: usize) -> i32 {
    with(hwnd, |lv| lv.cols.iter().map(|c| c.width).sum()).unwrap_or(0)
}

/// The rectangle of a row of the report view (`LVIR_BOUNDS`).
fn row_rect(hwnd: usize, i: i32) -> Rc {
    let rh = row_height(hwnd);
    let top = with(hwnd, |lv| lv.top).unwrap_or(0);
    let y = header_height(hwnd) + (i - top) * rh;
    let w = total_width(hwnd).max(rc_w(&client_rect(hwnd)));
    [0, y, w.max(total_width(hwnd)), y + rh]
}

/// The width of the icon the item has room for (the small image list's).
fn icon_width(hwnd: usize) -> i32 {
    let small = with(hwnd, |lv| lv.images[1]).unwrap_or(0);
    if small == 0 {
        return 0;
    }
    let (mut cx, mut cy) = (0, 0);
    // SAFETY: our image list functions.
    unsafe { crate::imagelist::ImageList_GetIconSize(small, &mut cx, &mut cy) };
    cx + 2
}

fn column_left(hwnd: usize, sub: usize) -> i32 {
    with(hwnd, |lv| lv.cols[..sub.min(lv.cols.len())].iter().map(|c| c.width).sum()).unwrap_or(0)
}

fn visible_rows(hwnd: usize) -> i32 {
    let c = client_rect(hwnd);
    ((rc_h(&c) - header_height(hwnd)) / row_height(hwnd).max(1)).max(1)
}

// ---- text and display information

fn dispinfo(hwnd: usize, item: i32, sub: i32, mask: u32, cap: usize) -> (Vec<u16>, i32, u32) {
    let mut buf = alloc::vec![0u16; cap.max(1)];
    let mut nm = NmLvDispInfo {
        hdr: NmHdr::default(),
        item: LvItem {
            mask,
            item,
            sub_item: sub,
            state: 0,
            state_mask: 0,
            text: if mask & LVIF_TEXT != 0 { buf.as_mut_ptr() as usize } else { 0 },
            text_max: if mask & LVIF_TEXT != 0 { cap as i32 } else { 0 },
            image: I_IMAGECALLBACK,
            param: 0,
            indent: 0,
            group_id: 0,
            columns: 0,
            pu_columns: 0,
        },
    };
    let own = buf.as_ptr() as usize;
    notify(hwnd, LVN_GETDISPINFOW, &mut nm);
    let mut text = Vec::new();
    if mask & LVIF_TEXT != 0 && nm.item.text != 0 {
        let p = nm.item.text;
        if p == own {
            text = buf.iter().take_while(|&&c| c != 0).copied().collect();
        } else {
            // The parent gave its own string.
            // SAFETY: a NUL-terminated string of the parent's.
            text = unsafe { wide_at(p, -1) };
            text.truncate(cap.saturating_sub(1));
        }
    }
    (text, nm.item.image, nm.item.state)
}

/// The text of a cell: stored text, or what the parent says (`LPSTR_TEXTCALLBACK`, no text set).
fn cell_text(hwnd: usize, item: i32, sub: i32, cap: usize) -> Vec<u16> {
    let stored = with(hwnd, |lv| {
        if lv.owner {
            return None;
        }
        lv.items.get(item as usize).map(|it| match it.texts.get(sub as usize) {
            Some(Some(Text::Str(s))) => Some(s.clone()),
            _ => None,
        })
    })
    .flatten()
    .flatten();
    if let Some(s) = stored {
        return s;
    }
    dispinfo(hwnd, item, sub, LVIF_TEXT, cap).0
}

fn cell_image(hwnd: usize, item: i32) -> i32 {
    let img = with(hwnd, |lv| if lv.owner { I_IMAGECALLBACK } else { lv.items.get(item as usize).map_or(-1, |i| i.image) }).unwrap_or(-1);
    if img == I_IMAGECALLBACK {
        let (_, image, _) = dispinfo(hwnd, item, 0, LVIF_IMAGE, 0);
        return image;
    }
    img
}

// ---- notifications

fn notify_item(hwnd: usize, code: i32, item: i32, sub: i32, new_state: u32, old_state: u32, changed: u32) -> isize {
    let param = with(hwnd, |lv| lv.items.get(item as usize).map_or(0, |i| i.param)).unwrap_or(0);
    let mut nm = NmListView { hdr: NmHdr::default(), item, sub_item: sub, new_state, old_state, changed, pt: [0; 2], param };
    notify(hwnd, code, &mut nm)
}

fn item_state(hwnd: usize, i: i32) -> u32 {
    with(hwnd, |lv| {
        let mut s = 0;
        if lv.owner {
            if lv.selected.contains(&i) {
                s |= LVIS_SELECTED;
            }
            if lv.focus == i {
                s |= LVIS_FOCUSED;
            }
        } else if let Some(it) = lv.items.get(i as usize) {
            s = it.state;
        }
        s
    })
    .unwrap_or(0)
}

/// Change the bits of `mask` of an item's state to `bits`; sends `LVN_ITEMCHANGING` (always, even
/// when nothing changes: the sheet's `LVM_SETITEMSTATE` of the same state) and, if the state
/// changed, `LVN_ITEMCHANGED` after the later changes of the same call (`defer`).
fn set_state(hwnd: usize, i: i32, bits: u32, mask: u32, defer: &mut Vec<(i32, u32, u32)>) -> bool {
    let old = item_state(hwnd, i);
    let new = (old & !mask) | (bits & mask);
    let refused = notify_item(hwnd, LVN_ITEMCHANGING, i, 0, new, old, LVIF_STATE) != 0;
    if refused {
        return false;
    }
    if new != old {
        with(hwnd, |lv| {
            if lv.owner {
                if mask & LVIS_SELECTED != 0 {
                    if new & LVIS_SELECTED != 0 {
                        lv.selected.insert(i);
                    } else {
                        lv.selected.remove(&i);
                    }
                }
                if mask & LVIS_FOCUSED != 0 {
                    lv.focus = if new & LVIS_FOCUSED != 0 { i } else if lv.focus == i { -1 } else { lv.focus };
                }
            } else if let Some(it) = lv.items.get_mut(i as usize) {
                it.state = new;
            }
        });
        defer.push((i, new, old));
    }
    true
}

fn flush(hwnd: usize, defer: Vec<(i32, u32, u32)>) {
    for (i, new, old) in defer.into_iter().rev() {
        notify_item(hwnd, LVN_ITEMCHANGED, i, 0, new, old, LVIF_STATE);
    }
}

/// `LVM_SETITEMSTATE` for one item: focus is exclusive, and in a single selection list so is
/// selection (the old one is deselected first).
fn set_item_state(hwnd: usize, i: i32, bits: u32, mask: u32) -> bool {
    let single = style_of(hwnd) & LVS_SINGLESEL != 0;
    let n = item_count(hwnd);
    if i < 0 || i >= n {
        return false;
    }
    let mut defer = Vec::new();
    let mut ok = true;
    if mask & LVIS_SELECTED != 0 && bits & LVIS_SELECTED != 0 && single {
        let others: Vec<i32> = (0..n).filter(|&k| k != i && item_state(hwnd, k) & LVIS_SELECTED != 0).collect();
        for k in others {
            let mut d = Vec::new();
            set_state(hwnd, k, 0, LVIS_SELECTED, &mut d);
            flush(hwnd, d);
        }
    }
    if mask & LVIS_FOCUSED != 0 && bits & LVIS_FOCUSED != 0 {
        let others: Vec<i32> = (0..n).filter(|&k| k != i && item_state(hwnd, k) & LVIS_FOCUSED != 0).collect();
        ok &= set_state(hwnd, i, bits, mask, &mut defer);
        for k in others {
            set_state(hwnd, k, 0, LVIS_FOCUSED, &mut defer);
        }
        // The first item that gets the focus is the selection mark.
        with(hwnd, |lv| {
            if lv.mark < 0 {
                lv.mark = i;
            }
        });
    } else {
        ok &= set_state(hwnd, i, bits, mask, &mut defer);
    }
    flush(hwnd, defer);
    invalidate(hwnd);
    ok
}

// ---- items

fn read_item(p: usize) -> LvItem {
    // SAFETY: the caller's LVITEMW (at least up to `lParam`: the smaller structures of earlier
    // versions end there; the rest reads as zero).
    unsafe { rd::<LvItem>(p, 0) }
}

fn insert_item(hwnd: usize, p: usize) -> isize {
    let it = read_item(p);
    let n = item_count(hwnd);
    if it.item < 0 {
        return -1;
    }
    let at = it.item.min(n);
    let ncols = with(hwnd, |lv| lv.cols.len()).unwrap_or(0).max(1);
    let mut texts: Vec<Option<Text>> = alloc::vec![None; ncols];
    if it.mask & LVIF_TEXT != 0 {
        texts[0] = Some(if it.text == LPSTR_TEXTCALLBACK { Text::Callback } else { Text::Str(unsafe { wide_at(it.text, -1) }) });
    }
    let item = Item {
        texts,
        image: if it.mask & LVIF_IMAGE != 0 { it.image } else { 0 },
        param: if it.mask & LVIF_PARAM != 0 { it.param } else { 0 },
        state: if it.mask & LVIF_STATE != 0 { it.state & it.state_mask } else { 0 },
        indent: if it.mask & LVIF_INDENT != 0 { it.indent } else { 0 },
    };
    with(hwnd, |lv| {
        if lv.owner {
            lv.count += 1;
            // Items at or after the new one move down.
            lv.selected = lv.selected.iter().map(|&k| if k >= at { k + 1 } else { k }).collect();
        } else {
            lv.items.insert(at as usize, item);
        }
    });
    notify_item(hwnd, LVN_INSERTITEM, at, 0, 0, 0, 0);
    invalidate(hwnd);
    at as isize
}

fn delete_item(hwnd: usize, i: i32) -> bool {
    let n = item_count(hwnd);
    if i < 0 || i >= n {
        return false;
    }
    let state = item_state(hwnd, i);
    notify_item(hwnd, LVN_ITEMCHANGING, i, 0, 0, state, LVIF_STATE);
    notify_item(hwnd, LVN_DELETEITEM, i, 0, 0, 0, 0);
    with(hwnd, |lv| {
        if lv.owner {
            lv.count -= 1;
            lv.selected = lv.selected.iter().filter(|&&k| k != i).map(|&k| if k > i { k - 1 } else { k }).collect();
        } else {
            lv.items.remove(i as usize);
        }
        if lv.focus == i {
            lv.focus = -1;
        } else if lv.focus > i {
            lv.focus -= 1;
        }
        if lv.mark == i {
            lv.mark = -1;
        }
    });
    invalidate(hwnd);
    true
}

fn delete_all(hwnd: usize) -> bool {
    let keep = notify_plain(hwnd, LVN_DELETEALLITEMS) != 0;
    let n = item_count(hwnd);
    if !keep {
        for i in (0..n).rev() {
            notify_item(hwnd, LVN_DELETEITEM, i, 0, 0, 0, 0);
        }
    }
    with(hwnd, |lv| {
        lv.items.clear();
        lv.count = 0;
        lv.selected.clear();
        lv.focus = -1;
        lv.mark = -1;
        lv.top = 0;
    });
    invalidate(hwnd);
    true
}

/// `LVM_GETITEMW`: only the fields of the mask; text and image from the parent when they are callbacks.
fn get_item(hwnd: usize, p: usize) -> bool {
    let mut it = read_item(p);
    let n = item_count(hwnd);
    if it.item < 0 || it.item >= n {
        return false;
    }
    if it.mask & LVIF_TEXT != 0 {
        let cap = it.text_max.max(0) as usize;
        let t = cell_text(hwnd, it.item, it.sub_item, cap.max(1));
        // SAFETY: the caller's buffer of `text_max` units.
        unsafe { put_text(it.text, cap, &t) };
    }
    if it.mask & LVIF_IMAGE != 0 {
        it.image = cell_image(hwnd, it.item);
    }
    if it.mask & LVIF_PARAM != 0 {
        it.param = with(hwnd, |lv| lv.items.get(it.item as usize).map_or(0, |x| x.param)).unwrap_or(0);
    }
    if it.mask & LVIF_STATE != 0 {
        it.state = item_state(hwnd, it.item) & it.state_mask;
    }
    if it.mask & LVIF_INDENT != 0 {
        it.indent = with(hwnd, |lv| lv.items.get(it.item as usize).map_or(0, |x| x.indent)).unwrap_or(0);
    }
    // SAFETY: the caller's LVITEMW; only the members of the mask changed, the text buffer was written above.
    unsafe {
        wr(p, core::mem::offset_of!(LvItem, state), it.state);
        wr(p, core::mem::offset_of!(LvItem, image), it.image);
        wr(p, core::mem::offset_of!(LvItem, param), it.param);
        wr(p, core::mem::offset_of!(LvItem, indent), it.indent);
    }
    true
}

fn set_item(hwnd: usize, p: usize) -> bool {
    let it = read_item(p);
    let n = item_count(hwnd);
    if it.item < 0 || it.item >= n {
        return false;
    }
    let old_state = item_state(hwnd, it.item);
    let new_state = if it.mask & LVIF_STATE != 0 { (old_state & !it.state_mask) | (it.state & it.state_mask) } else { old_state };
    if notify_item(hwnd, LVN_ITEMCHANGING, it.item, it.sub_item, new_state, old_state, it.mask) != 0 {
        return false;
    }
    let text = if it.mask & LVIF_TEXT != 0 {
        Some(if it.text == LPSTR_TEXTCALLBACK { Text::Callback } else { Text::Str(unsafe { wide_at(it.text, -1) }) })
    } else {
        None
    };
    let ncols = with(hwnd, |lv| lv.cols.len()).unwrap_or(0).max(1);
    with(hwnd, |lv| {
        if let Some(x) = lv.items.get_mut(it.item as usize) {
            if let Some(t) = text {
                let sub = it.sub_item.max(0) as usize;
                if sub < ncols.max(x.texts.len()) {
                    if x.texts.len() <= sub {
                        x.texts.resize(sub + 1, None);
                    }
                    x.texts[sub] = Some(t);
                }
            }
            if it.mask & LVIF_IMAGE != 0 {
                x.image = it.image;
            }
            if it.mask & LVIF_PARAM != 0 {
                x.param = it.param;
            }
            if it.mask & LVIF_INDENT != 0 {
                x.indent = it.indent;
            }
            if it.mask & LVIF_STATE != 0 {
                x.state = new_state;
            }
        }
    });
    notify_item(hwnd, LVN_ITEMCHANGED, it.item, it.sub_item, new_state, old_state, it.mask);
    invalidate(hwnd);
    true
}

// ---- columns

fn read_column(p: usize) -> LvColumn {
    // SAFETY: the caller's LVCOLUMNW.
    unsafe { rd::<LvColumn>(p, 0) }
}

fn col_from(c: &LvColumn) -> Col {
    Col {
        text: if c.mask & LVCF_TEXT != 0 { unsafe { wide_at(c.text, -1) } } else { Vec::new() },
        width: if c.mask & LVCF_WIDTH != 0 { c.cx } else { 0 },
        fmt: if c.mask & LVCF_FMT != 0 { c.fmt } else { 0 },
        sub_item: if c.mask & LVCF_SUBITEM != 0 { c.sub_item } else { 0 },
        image: if c.mask & LVCF_IMAGE != 0 { c.image } else { -1 },
        order: if c.mask & LVCF_ORDER != 0 { c.order } else { 0 },
    }
}

const COL_FMT_FLAG: i32 = 0x4000;

fn after_columns(hwnd: usize) {
    sync_header(hwnd);
    layout(hwnd);
    invalidate(hwnd);
}

// ---- hit testing and rectangles

fn item_at(hwnd: usize, x: i32, y: i32) -> (i32, u32) {
    let n = item_count(hwnd);
    let hh = header_height(hwnd);
    let rh = row_height(hwnd);
    let top = with(hwnd, |lv| lv.top).unwrap_or(0);
    if y < hh {
        return (-1, LVHT_ABOVE);
    }
    let i = top + (y - hh) / rh;
    let c = client_rect(hwnd);
    if i >= n || y >= rc_h(&c) {
        return (-1, if y >= rc_h(&c) { LVHT_BELOW } else { LVHT_NOWHERE | LVHT_BELOW });
    }
    let ex = with(hwnd, |lv| lv.ex).unwrap_or(0);
    let icon = icon_width(hwnd);
    let check = if ex & LVS_EX_CHECKBOXES != 0 { 16 } else { 0 };
    let tw = {
        let t = cell_text(hwnd, i, 0, 260);
        let font = with(hwnd, |lv| lv.font).unwrap_or(0);
        let m = Measure::new(hwnd, font);
        text_width(m.hdc, &t)
    };
    let flags = if x < check {
        LVHT_ONITEMSTATEICON
    } else if x < check + icon {
        LVHT_ONITEMICON
    } else if x < check + icon + 2 + tw + 4 || ex & LVS_EX_FULLROWSELECT != 0 {
        LVHT_ONITEMLABEL
    } else {
        LVHT_NOWHERE
    };
    (if flags == LVHT_NOWHERE { -1 } else { i }, flags)
}

fn text_extent_of(hwnd: usize, i: i32) -> i32 {
    let t = cell_text(hwnd, i, 0, 260);
    let font = with(hwnd, |lv| lv.font).unwrap_or(0);
    let m = Measure::new(hwnd, font);
    text_width(m.hdc, &t)
}

fn get_item_rect(hwnd: usize, i: i32, code: i32, out: usize) -> bool {
    if i < 0 || i >= item_count(hwnd) || out == 0 {
        return false;
    }
    let ex = with(hwnd, |lv| lv.ex).unwrap_or(0);
    let row = row_rect(hwnd, i);
    let icon = icon_width(hwnd);
    let first = with(hwnd, |lv| lv.cols.first().map_or(row[2], |c| c.width)).unwrap_or(row[2]);
    let r = match code {
        LVIR_BOUNDS => row,
        LVIR_ICON => [row[0], row[1], row[0] + icon, row[3]],
        LVIR_LABEL => [row[0] + icon + 2, row[1], (row[0] + first).min(row[2]), row[3]],
        _ => {
            if ex & LVS_EX_FULLROWSELECT != 0 {
                row
            } else {
                [row[0] + 2, row[1], row[0] + icon + 2 + text_extent_of(hwnd, i) + 2, row[3]]
            }
        }
    };
    for (k, v) in r.iter().enumerate() {
        // SAFETY: the caller's RECT.
        unsafe { wr(out, 4 * k, *v) };
    }
    true
}

fn ensure_visible(hwnd: usize, i: i32) {
    if i < 0 || i >= item_count(hwnd) {
        return;
    }
    let rows = visible_rows(hwnd);
    with(hwnd, |lv| {
        if i < lv.top {
            lv.top = i;
        } else if i >= lv.top + rows {
            lv.top = i - rows + 1;
        }
    });
    invalidate(hwnd);
}

fn next_item(hwnd: usize, from: i32, flags: u32) -> i32 {
    let n = item_count(hwnd);
    let want = |k: i32| -> bool {
        let st = item_state(hwnd, k);
        (flags & LVNI_FOCUSED == 0 || st & LVIS_FOCUSED != 0)
            && (flags & LVNI_SELECTED == 0 || st & LVIS_SELECTED != 0)
            && (flags & LVNI_CUT == 0 || st & LVIS_CUT != 0)
            && (flags & LVNI_DROPHILITED == 0 || st & LVIS_DROPHILITED != 0)
    };
    if flags & LVNI_ABOVE != 0 {
        let start = if from < 0 { n - 1 } else { from - 1 };
        return (0..=start).rev().find(|&k| k < n && want(k)).unwrap_or(-1);
    }
    if flags & (LVNI_BELOW | LVNI_TORIGHT) != 0 && flags & LVNI_ABOVE == 0 {
        return ((from + 1).max(0)..n).find(|&k| want(k)).unwrap_or(-1);
    }
    if flags & LVNI_TOLEFT != 0 {
        return (0..from.max(0)).rev().find(|&k| want(k)).unwrap_or(-1);
    }
    ((from + 1).max(0)..n).find(|&k| want(k)).unwrap_or(-1)
}

// ---- painting

fn draw(hwnd: usize, hdc: usize) {
    let c = client_rect(hwnd);
    let Some((bk, text_clr, tbk, font, ex, small, top, focus_sel_always)) = with(hwnd, |lv| {
        (lv.bk, lv.text, lv.text_bk, lv.font, lv.ex, lv.images[1], lv.top, true)
    }) else {
        return;
    };
    let _ = focus_sel_always;
    let style = style_of(hwnd);
    fill(hdc, &c, if bk == CLR_NONE { sys_color(COLOR_WINDOW) } else { bk });
    // The custom draw round: one notification before anything is drawn.
    let mut cd = NmLvCustomDraw {
        nmcd: NmCustomDraw { hdr: NmHdr::default(), stage: CDDS_PREPAINT, hdc, rc: c, item_spec: 0, item_state: 0, item_param: 0 },
        text: CLR_DEFAULT,
        text_bk: CLR_DEFAULT,
        sub_item: 0,
        item_type: 0,
        pt: [0; 2],
        align: 0,
    };
    let want_items = notify(hwnd, NM_CUSTOMDRAW, &mut cd) & CDRF_NOTIFYITEMDRAW != 0;
    let n = item_count(hwnd);
    let rows = visible_rows(hwnd);
    let last = (top + rows).min(n);
    if with(hwnd, |lv| lv.owner).unwrap_or(false) && n > 0 {
        let mut hint = NmLvCacheHint { hdr: NmHdr::default(), from: top, to: (last - 1).max(top) };
        notify(hwnd, LVN_ODCACHEHINT, &mut hint);
    }
    // SAFETY: the DC and a font.
    let old = unsafe { SelectObject(hdc, if font != 0 { font } else { GetStockObject(DEFAULT_GUI_FONT) }) };
    // SAFETY: the DC.
    unsafe { SetBkMode(hdc, 1) };
    let focused = unsafe { GetFocus() } == hwnd;
    let ncols = with(hwnd, |lv| lv.cols.len()).unwrap_or(0).max(1);
    let widths: Vec<(i32, i32)> = with(hwnd, |lv| lv.cols.iter().map(|c| (c.width, c.fmt)).collect()).unwrap_or_default();
    let report = style & LVS_TYPEMASK == LVS_REPORT;
    let rh = row_height(hwnd);
    let icon = icon_width(hwnd);
    for i in top..last {
        let row = row_rect(hwnd, i);
        if row[1] >= rc_h(&c) {
            break;
        }
        let state = item_state(hwnd, i);
        let sel = state & LVIS_SELECTED != 0 && (focused || style & LVS_SHOWSELALWAYS != 0);
        let mut text_color = if text_clr == CLR_DEFAULT { sys_color(COLOR_WINDOWTEXT) } else { text_clr };
        let mut row_bk = None;
        if want_items {
            let mut icd = NmLvCustomDraw {
                nmcd: NmCustomDraw { hdr: NmHdr::default(), stage: CDDS_ITEMPREPAINT, hdc, rc: row, item_spec: i as usize, item_state: 0, item_param: 0 },
                text: CLR_DEFAULT,
                text_bk: CLR_DEFAULT,
                sub_item: 0,
                item_type: 0,
                pt: [0; 2],
                align: 0,
            };
            let r = notify(hwnd, NM_CUSTOMDRAW, &mut icd);
            if r & CDRF_SKIPDEFAULT != 0 {
                continue;
            }
            if r & CDRF_NEWFONT != 0 || icd.text != CLR_DEFAULT {
                if icd.text != CLR_DEFAULT {
                    text_color = icd.text;
                }
                if icd.text_bk != CLR_DEFAULT {
                    row_bk = Some(icd.text_bk);
                }
            }
        }
        let _ = tbk;
        let full = ex & LVS_EX_FULLROWSELECT != 0;
        let label_w = text_extent_of(hwnd, i);
        let sel_rect = if full || !report { row } else { [row[0] + 2, row[1], row[0] + icon + 2 + label_w + 2, row[3]] };
        if let Some(b) = row_bk {
            fill(hdc, &row, b);
        }
        if sel {
            let color = if focused { sys_color(COLOR_HIGHLIGHT) } else { sys_color(COLOR_BTNFACE) };
            fill(hdc, &sel_rect, color);
            if focused {
                text_color = sys_color(COLOR_HIGHLIGHTTEXT);
            }
        }
        let mut x = row[0];
        for sub in 0..ncols {
            let w = widths.get(sub).map_or(rc_w(&row), |c| c.0);
            let cell = [x, row[1], x + w, row[3]];
            if cell[0] >= rc_w(&c) {
                break;
            }
            let mut tx = cell[0] + 3;
            if sub == 0 {
                let img = cell_image(hwnd, i);
                if small != 0 && img >= 0 {
                    // SAFETY: our image list of the program.
                    unsafe { crate::imagelist::ImageList_Draw(small, img, hdc, cell[0] + 1, row[1] + (rh - icon.max(16)) / 2 + 1, 1) };
                }
                tx = cell[0] + icon + 3;
            }
            let t = cell_text(hwnd, i, sub as i32, 260);
            // SAFETY: the DC.
            unsafe { SetTextColor(hdc, text_color) };
            let fmt = widths.get(sub).map_or(0, |c| c.1) & 3;
            let align = match fmt {
                1 => DT_RIGHT,
                2 => DT_CENTER,
                _ => DT_LEFT,
            };
            draw_text(hdc, &t, &[tx, cell[1], cell[2] - 4, cell[3]], DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS | align);
            if ex & LVS_EX_GRIDLINES != 0 {
                fill(hdc, &[cell[2] - 1, cell[1], cell[2], cell[3]], sys_color(COLOR_BTNFACE));
            }
            x += w;
        }
        if ex & LVS_EX_GRIDLINES != 0 {
            fill(hdc, &[row[0], row[3] - 1, row[2], row[3]], sys_color(COLOR_BTNFACE));
        }
        if focused && state & LVIS_FOCUSED != 0 {
            focus_rect(hdc, &sel_rect, sys_color(COLOR_WINDOWTEXT));
        }
    }
    // SAFETY: the font taken above.
    unsafe { SelectObject(hdc, old) };
}

fn paint_event(hwnd: usize) {
    let mut ps = [0u8; 72];
    // SAFETY: a PAINTSTRUCT of at most 72 bytes.
    unsafe {
        let hdc = BeginPaint(hwnd, ps.as_mut_ptr());
        draw(hwnd, hdc);
        EndPaint(hwnd, ps.as_ptr());
    }
}

// ---- keyboard and mouse

fn navigate(hwnd: usize, vk: u32) {
    let n = item_count(hwnd);
    if n == 0 {
        return;
    }
    let focus = with(hwnd, |lv| {
        if lv.owner {
            lv.focus
        } else {
            lv.items.iter().position(|i| i.state & LVIS_FOCUSED != 0).map_or(-1, |p| p as i32)
        }
    })
    .unwrap_or(-1);
    let page = visible_rows(hwnd);
    let target = match vk {
        VK_DOWN | VK_RIGHT => {
            if focus < 0 { 0 } else { (focus + 1).min(n - 1) }
        }
        VK_UP | VK_LEFT => {
            if focus < 0 { 0 } else { (focus - 1).max(0) }
        }
        VK_HOME => 0,
        VK_END => n - 1,
        VK_NEXT => if focus < 0 { 0 } else { (focus + page).min(n - 1) },
        VK_PRIOR => if focus < 0 { 0 } else { (focus - page).max(0) },
        _ => return,
    };
    let single = style_of(hwnd) & LVS_SINGLESEL != 0;
    let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
    // Take the focus and selection away from where they were, then put them on the target.
    let old_focus = focus;
    if old_focus >= 0 && old_focus != target {
        let mut d = Vec::new();
        set_state(hwnd, old_focus, 0, LVIS_FOCUSED, &mut d);
        flush(hwnd, d);
    }
    if !ctrl || single {
        let selected: Vec<i32> = (0..n).filter(|&k| k != target && item_state(hwnd, k) & LVIS_SELECTED != 0).collect();
        for k in selected {
            let mut d = Vec::new();
            set_state(hwnd, k, 0, LVIS_SELECTED, &mut d);
            flush(hwnd, d);
        }
    }
    let mut d = Vec::new();
    set_state(hwnd, target, if ctrl && !single { LVIS_FOCUSED } else { LVIS_FOCUSED | LVIS_SELECTED }, LVIS_FOCUSED | LVIS_SELECTED, &mut d);
    flush(hwnd, d);
    with(hwnd, |lv| lv.mark = target);
    ensure_visible(hwnd, target);
}

fn click(hwnd: usize, x: i32, y: i32, double: bool) {
    // SAFETY: a plain call.
    unsafe { SetFocus(hwnd) };
    let (i, flags) = item_at(hwnd, x, y);
    let ctrl = unsafe { GetKeyState(VK_CONTROL) } < 0;
    let shift = unsafe { GetKeyState(VK_SHIFT) } < 0;
    if i >= 0 {
        let single = style_of(hwnd) & LVS_SINGLESEL != 0;
        if !(ctrl || shift) || single {
            let n = item_count(hwnd);
            let selected: Vec<i32> = (0..n).filter(|&k| k != i && item_state(hwnd, k) & LVIS_SELECTED != 0).collect();
            for k in selected {
                let mut d = Vec::new();
                set_state(hwnd, k, 0, LVIS_SELECTED, &mut d);
                flush(hwnd, d);
            }
        }
        if shift && !single {
            let mark = with(hwnd, |lv| lv.mark).unwrap_or(-1).max(0);
            for k in mark.min(i)..=mark.max(i) {
                let mut d = Vec::new();
                set_state(hwnd, k, LVIS_SELECTED, LVIS_SELECTED, &mut d);
                flush(hwnd, d);
            }
        } else if ctrl && !single {
            let cur = item_state(hwnd, i) & LVIS_SELECTED;
            let mut d = Vec::new();
            set_state(hwnd, i, cur ^ LVIS_SELECTED, LVIS_SELECTED, &mut d);
            flush(hwnd, d);
        } else {
            let mut d = Vec::new();
            set_state(hwnd, i, LVIS_SELECTED, LVIS_SELECTED, &mut d);
            flush(hwnd, d);
        }
        // The item has the focus.
        let old = with(hwnd, |lv| if lv.owner { lv.focus } else { lv.items.iter().position(|x| x.state & LVIS_FOCUSED != 0).map_or(-1, |p| p as i32) }).unwrap_or(-1);
        if old != i {
            let mut d = Vec::new();
            if old >= 0 {
                set_state(hwnd, old, 0, LVIS_FOCUSED, &mut d);
            }
            set_state(hwnd, i, LVIS_FOCUSED, LVIS_FOCUSED, &mut d);
            flush(hwnd, d);
        }
        if !shift {
            with(hwnd, |lv| lv.mark = i);
        }
    } else if !(ctrl || shift) && flags & (LVHT_NOWHERE | LVHT_BELOW) != 0 {
        let n = item_count(hwnd);
        let selected: Vec<i32> = (0..n).filter(|&k| item_state(hwnd, k) & LVIS_SELECTED != 0).collect();
        for k in selected {
            let mut d = Vec::new();
            set_state(hwnd, k, 0, LVIS_SELECTED, &mut d);
            flush(hwnd, d);
        }
    }
    let mut nm = NmListView { hdr: NmHdr::default(), item: i, sub_item: 0, new_state: 0, old_state: 0, changed: 0, pt: [x, y], param: 0 };
    notify(hwnd, if double { NM_DBLCLK } else { NM_CLICK }, &mut nm);
    if double && i >= 0 {
        let mut nm = NmListView { hdr: NmHdr::default(), item: i, sub_item: 0, new_state: 0, old_state: 0, changed: 0, pt: [x, y], param: 0 };
        notify(hwnd, LVN_ITEMACTIVATE, &mut nm);
    }
    invalidate(hwnd);
}

// ---- the window procedure

const LVM_GETBKCOLOR: u32 = 0x1000;
const LVM_SETBKCOLOR: u32 = 0x1001;
const LVM_GETIMAGELIST: u32 = 0x1002;
const LVM_SETIMAGELIST: u32 = 0x1003;
const LVM_GETITEMCOUNT: u32 = 0x1004;
const LVM_GETITEMA: u32 = 0x1005;
const LVM_SETITEMA: u32 = 0x1006;
const LVM_INSERTITEMA: u32 = 0x1007;
const LVM_DELETEITEM: u32 = 0x1008;
const LVM_DELETEALLITEMS: u32 = 0x1009;
const LVM_GETCALLBACKMASK: u32 = 0x100A;
const LVM_SETCALLBACKMASK: u32 = 0x100B;
const LVM_GETNEXTITEM: u32 = 0x100C;
const LVM_FINDITEMA: u32 = 0x100D;
const LVM_GETITEMRECT: u32 = 0x100E;
const LVM_HITTEST: u32 = 0x1012;
const LVM_ENSUREVISIBLE: u32 = 0x1013;
const LVM_SCROLL: u32 = 0x1014;
const LVM_REDRAWITEMS: u32 = 0x1015;
const LVM_ARRANGE: u32 = 0x1016;
const LVM_GETCOLUMNA: u32 = 0x1019;
const LVM_SETCOLUMNA: u32 = 0x101A;
const LVM_INSERTCOLUMNA: u32 = 0x101B;
const LVM_DELETECOLUMN: u32 = 0x101C;
const LVM_GETCOLUMNWIDTH: u32 = 0x101D;
const LVM_SETCOLUMNWIDTH: u32 = 0x101E;
const LVM_GETHEADER: u32 = 0x101F;
const LVM_GETVIEWRECT: u32 = 0x1022;
const LVM_GETTEXTCOLOR: u32 = 0x1023;
const LVM_SETTEXTCOLOR: u32 = 0x1024;
const LVM_GETTEXTBKCOLOR: u32 = 0x1025;
const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
const LVM_GETTOPINDEX: u32 = 0x1027;
const LVM_GETCOUNTPERPAGE: u32 = 0x1028;
const LVM_UPDATE: u32 = 0x102A;
const LVM_SETITEMSTATE: u32 = 0x102B;
const LVM_GETITEMSTATE: u32 = 0x102C;
const LVM_GETITEMTEXTA: u32 = 0x102D;
const LVM_SETITEMTEXTA: u32 = 0x102E;
const LVM_SETITEMCOUNT: u32 = 0x102F;
const LVM_SORTITEMS: u32 = 0x1030;
const LVM_GETSELECTEDCOUNT: u32 = 0x1032;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = 0x1037;
const LVM_GETSUBITEMRECT: u32 = 0x1038;
const LVM_SUBITEMHITTEST: u32 = 0x1039;
const LVM_GETSELECTIONMARK: u32 = 0x1042;
const LVM_SETSELECTIONMARK: u32 = 0x1043;
const LVM_GETITEMW: u32 = 0x104B;
const LVM_SETITEMW: u32 = 0x104C;
const LVM_INSERTITEMW: u32 = 0x104D;
const LVM_SORTITEMSEX: u32 = 0x1051;
const LVM_FINDITEMW: u32 = 0x1053;
const LVM_GETSTRINGWIDTHW: u32 = 0x1057;
const LVM_GETCOLUMNW: u32 = 0x105F;
const LVM_SETCOLUMNW: u32 = 0x1060;
const LVM_INSERTCOLUMNW: u32 = 0x1061;
const LVM_GETITEMTEXTW: u32 = 0x1073;
const LVM_SETITEMTEXTW: u32 = 0x1074;
const CCM_SETUNICODEFORMAT: u32 = 0x2005;
const CCM_GETUNICODEFORMAT: u32 = 0x2006;
const LVM_SETCOLUMNORDERARRAY: u32 = 0x103A;
const LVM_GETCOLUMNORDERARRAY: u32 = 0x103B;

fn ansi_to_wide(p: usize) -> Vec<u16> {
    let mut b = Vec::new();
    let mut q = p as *const u8;
    // SAFETY: a NUL-terminated string.
    unsafe {
        while !q.is_null() && *q != 0 {
            b.push(*q);
            q = q.add(1);
        }
    }
    poly_kernelbase::text::from_ansi(&b)
}

pub fn proc(hwnd: usize, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: the messages' pointers are the caller's structures (`commctrl.h`).
    unsafe {
        match msg {
            WM_SIZE => {
                layout(hwnd);
                0
            }
            WM_PAINT => {
                paint_event(hwnd);
                0
            }
            WM_PRINT | WM_PRINTCLIENT => {
                draw(hwnd, w);
                if msg == WM_PRINT && l & 0x10 != 0 {
                    // PRF_CHILDREN: the header.
                    let h = with(hwnd, |lv| lv.header).unwrap_or(0);
                    if h != 0 && header_height(hwnd) > 0 {
                        let r = client_rect(h);
                        let _ = r;
                        SendMessageW(h, WM_PRINTCLIENT, w, 0);
                    }
                }
                0
            }
            WM_ERASEBKGND => 1,
            WM_SETFONT => {
                with(hwnd, |lv| lv.font = w);
                sync_header(hwnd);
                layout(hwnd);
                invalidate(hwnd);
                0
            }
            WM_GETFONT => with(hwnd, |lv| lv.font).unwrap_or(0) as isize,
            WM_SETFOCUS | WM_KILLFOCUS => {
                notify_plain(hwnd, if msg == WM_SETFOCUS { NM_SETFOCUS } else { NM_KILLFOCUS });
                invalidate(hwnd);
                0
            }
            WM_GETDLGCODE => 0x0001 | 0x0004 | 0x0080,
            WM_KEYDOWN => {
                let mut nm = NmLvKeyDown { hdr: NmHdr::default(), vkey: w as u16, flags: 0 };
                notify(hwnd, LVN_KEYDOWN, &mut nm);
                navigate(hwnd, w as u32);
                0
            }
            WM_LBUTTONDOWN => {
                click(hwnd, lo(l), hi(l), false);
                0
            }
            WM_LBUTTONDBLCLK => {
                click(hwnd, lo(l), hi(l), true);
                0
            }
            WM_RBUTTONUP => {
                let (i, _) = item_at(hwnd, lo(l), hi(l));
                let mut nm = NmListView { hdr: NmHdr::default(), item: i, sub_item: 0, new_state: 0, old_state: 0, changed: 0, pt: [lo(l), hi(l)], param: 0 };
                notify(hwnd, NM_RCLICK, &mut nm);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = hi(w as isize);
                let n = item_count(hwnd);
                let rows = visible_rows(hwnd);
                with(hwnd, |lv| lv.top = (lv.top - (delta / 120) * 3).clamp(0, (n - rows).max(0)));
                invalidate(hwnd);
                0
            }
            WM_NOTIFY => {
                // From the header: a click on a heading is `LVN_COLUMNCLICK`, a changed width is passed on.
                let code = rd::<i32>(l as usize, 2 * core::mem::size_of::<usize>()) ;
                let from = rd::<usize>(l as usize, 0);
                let h = with(hwnd, |lv| lv.header).unwrap_or(0);
                if from == h && h != 0 && code == HDN_ITEMCLICKW {
                    let item = rd::<i32>(l as usize, core::mem::size_of::<NmHdr>());
                    let mut nm = NmListView { hdr: NmHdr::default(), item: -1, sub_item: item, new_state: 0, old_state: 0, changed: 0, pt: [0; 2], param: 0 };
                    notify(hwnd, LVN_COLUMNCLICK, &mut nm);
                }
                0
            }
            LVM_GETBKCOLOR => with(hwnd, |lv| lv.bk).unwrap_or(0) as isize,
            LVM_SETBKCOLOR => {
                with(hwnd, |lv| lv.bk = l as u32);
                invalidate(hwnd);
                1
            }
            LVM_GETTEXTCOLOR => with(hwnd, |lv| lv.text).unwrap_or(0) as isize,
            LVM_SETTEXTCOLOR => {
                with(hwnd, |lv| lv.text = l as u32);
                invalidate(hwnd);
                1
            }
            LVM_GETTEXTBKCOLOR => with(hwnd, |lv| lv.text_bk).unwrap_or(0) as isize,
            LVM_SETTEXTBKCOLOR => {
                with(hwnd, |lv| lv.text_bk = l as u32);
                invalidate(hwnd);
                1
            }
            LVM_GETIMAGELIST => with(hwnd, |lv| lv.images.get(w).copied().unwrap_or(0)).unwrap_or(0) as isize,
            LVM_SETIMAGELIST => {
                // LVSIL_NORMAL 0, LVSIL_SMALL 1, LVSIL_STATE 2.
                let old = with(hwnd, |lv| lv.images.get_mut(w).map(|s| core::mem::replace(s, l as usize)).unwrap_or(0)).unwrap_or(0);
                invalidate(hwnd);
                old as isize
            }
            LVM_GETITEMCOUNT => item_count(hwnd) as isize,
            LVM_GETITEMA | LVM_GETITEMW => isize::from(get_item(hwnd, l as usize)),
            LVM_SETITEMA | LVM_SETITEMW => isize::from(l != 0 && set_item(hwnd, l as usize)),
            LVM_INSERTITEMA | LVM_INSERTITEMW => {
                if l == 0 {
                    return -1;
                }
                insert_item(hwnd, l as usize)
            }
            LVM_DELETEITEM => isize::from(delete_item(hwnd, w as i32)),
            LVM_DELETEALLITEMS => isize::from(delete_all(hwnd)),
            LVM_GETCALLBACKMASK => with(hwnd, |lv| lv.callback_mask).unwrap_or(0) as isize,
            LVM_SETCALLBACKMASK => {
                with(hwnd, |lv| lv.callback_mask = w as u32);
                1
            }
            LVM_GETNEXTITEM => next_item(hwnd, w as i32, l as u32 & 0xFFFF) as isize,
            LVM_FINDITEMA | LVM_FINDITEMW => {
                // LVFINDINFOW: flags, psz, lParam.
                let flags: u32 = rd(l as usize, 0);
                let psz: usize = rd(l as usize, core::mem::size_of::<usize>());
                let param: isize = rd(l as usize, 2 * core::mem::size_of::<usize>());
                let n = item_count(hwnd);
                let wanted = if flags & 2 != 0 { Some(if msg == LVM_FINDITEMW { wide_at(psz, -1) } else { ansi_to_wide(psz) }) } else { None };
                let eq = |a: &[u16], b: &[u16], partial: bool| -> bool {
                    let f = |c: u16| if (0x41..=0x5A).contains(&c) { c + 32 } else { c };
                    let (a, b): (Vec<u16>, Vec<u16>) = (a.iter().map(|&c| f(c)).collect(), b.iter().map(|&c| f(c)).collect());
                    if partial { a.starts_with(&b) } else { a == b }
                };
                let start = (w as i32 + 1).max(0);
                for k in start..n {
                    let hit = if flags & 1 != 0 {
                        with(hwnd, |lv| lv.items.get(k as usize).is_some_and(|i| i.param == param)).unwrap_or(false)
                    } else if let Some(t) = &wanted {
                        eq(&cell_text(hwnd, k, 0, 260), t, flags & 8 != 0)
                    } else {
                        false
                    };
                    if hit {
                        return k as isize;
                    }
                }
                -1
            }
            LVM_GETITEMRECT => isize::from(get_item_rect(hwnd, w as i32, rd::<i32>(l as usize, 0), l as usize)),
            LVM_GETSUBITEMRECT => {
                let code = rd::<i32>(l as usize, 0);
                let sub = rd::<i32>(l as usize, 4);
                if w as i32 >= item_count(hwnd) || sub < 0 {
                    return 0;
                }
                if sub == 0 {
                    return isize::from(get_item_rect(hwnd, w as i32, code, l as usize));
                }
                let row = row_rect(hwnd, w as i32);
                let (left, width) = (column_left(hwnd, sub as usize), with(hwnd, |lv| lv.cols.get(sub as usize).map_or(0, |c| c.width)).unwrap_or(0));
                let r = [left, row[1], left + width, row[3]];
                for (k, v) in r.iter().enumerate() {
                    wr(l as usize, 4 * k, *v);
                }
                1
            }
            LVM_HITTEST | LVM_SUBITEMHITTEST => {
                let (x, y) = (rd::<i32>(l as usize, 0), rd::<i32>(l as usize, 4));
                let (i, flags) = item_at(hwnd, x, y);
                wr(l as usize, 8, flags);
                wr(l as usize, 12, i);
                if msg == LVM_SUBITEMHITTEST {
                    let mut sub = 0;
                    let mut left = 0;
                    let widths: Vec<i32> = with(hwnd, |lv| lv.cols.iter().map(|c| c.width).collect()).unwrap_or_default();
                    for (k, cw) in widths.iter().enumerate() {
                        if x >= left && x < left + cw {
                            sub = k as i32;
                        }
                        left += cw;
                    }
                    wr(l as usize, 16, sub);
                }
                i as isize
            }
            LVM_ENSUREVISIBLE => {
                ensure_visible(hwnd, w as i32);
                1
            }
            LVM_SCROLL => {
                let rh = row_height(hwnd).max(1);
                with(hwnd, |lv| lv.top = (lv.top + hi(l) / rh).max(0));
                invalidate(hwnd);
                1
            }
            LVM_REDRAWITEMS | LVM_UPDATE | LVM_ARRANGE => {
                invalidate(hwnd);
                1
            }
            LVM_GETTOPINDEX => with(hwnd, |lv| lv.top).unwrap_or(0) as isize,
            LVM_GETCOUNTPERPAGE => visible_rows(hwnd) as isize,
            LVM_GETVIEWRECT => {
                let n = item_count(hwnd);
                let r = [0, 0, total_width(hwnd), n * row_height(hwnd) + header_height(hwnd)];
                if l != 0 {
                    for (k, v) in r.iter().enumerate() {
                        wr(l as usize, 4 * k, *v);
                    }
                }
                1
            }
            LVM_SETITEMSTATE => {
                let (bits, mask) = if l != 0 { (rd::<u32>(l as usize, core::mem::offset_of!(LvItem, state)), rd::<u32>(l as usize, core::mem::offset_of!(LvItem, state_mask))) } else { (0, 0) };
                if (w as i32) < 0 {
                    // Every item.
                    let n = item_count(hwnd);
                    let single = style_of(hwnd) & LVS_SINGLESEL != 0;
                    if single && bits & LVIS_SELECTED != 0 {
                        return 0;
                    }
                    let mut defer = Vec::new();
                    for k in 0..n {
                        set_state(hwnd, k, bits, mask, &mut defer);
                        // Each item's change is reported as it happens.
                        flush(hwnd, core::mem::take(&mut defer));
                    }
                    invalidate(hwnd);
                    return 1;
                }
                isize::from(set_item_state(hwnd, w as i32, bits, mask))
            }
            LVM_GETITEMSTATE => {
                if (w as i32) < 0 || w as i32 >= item_count(hwnd) {
                    return 0;
                }
                (item_state(hwnd, w as i32) & l as u32) as isize
            }
            LVM_GETITEMTEXTA | LVM_GETITEMTEXTW => {
                if w as i32 >= item_count(hwnd) {
                    return 0;
                }
                let it = read_item(l as usize);
                let cap = it.text_max.max(0) as usize;
                let t = cell_text(hwnd, w as i32, it.sub_item, cap.max(1));
                if msg == LVM_GETITEMTEXTW {
                    put_text(it.text, cap, &t) as isize
                } else {
                    let a = poly_kernelbase::text::to_ansi(&t);
                    let n = a.len().min(cap.saturating_sub(1));
                    if cap > 0 {
                        core::ptr::copy_nonoverlapping(a.as_ptr(), it.text as *mut u8, n);
                        *(it.text as *mut u8).add(n) = 0;
                    }
                    n as isize
                }
            }
            LVM_SETITEMTEXTA | LVM_SETITEMTEXTW => {
                let it = read_item(l as usize);
                let sub = it.sub_item;
                let ncols = with(hwnd, |lv| lv.cols.len()).unwrap_or(0).max(1) as i32;
                if w as i32 >= item_count(hwnd) || sub < 0 || sub >= ncols {
                    return 0;
                }
                let t = if it.text == LPSTR_TEXTCALLBACK {
                    Text::Callback
                } else if msg == LVM_SETITEMTEXTW {
                    Text::Str(wide_at(it.text, -1))
                } else {
                    Text::Str(ansi_to_wide(it.text))
                };
                with(hwnd, |lv| {
                    if let Some(x) = lv.items.get_mut(w) {
                        if x.texts.len() <= sub as usize {
                            x.texts.resize(sub as usize + 1, None);
                        }
                        x.texts[sub as usize] = Some(t);
                    }
                });
                invalidate(hwnd);
                1
            }
            LVM_SETITEMCOUNT => {
                with(hwnd, |lv| {
                    if lv.owner {
                        lv.count = w as i32;
                        lv.selected.retain(|&k| k < w as i32);
                    }
                });
                invalidate(hwnd);
                1
            }
            LVM_GETSELECTEDCOUNT => {
                let n = item_count(hwnd);
                (0..n).filter(|&k| item_state(hwnd, k) & LVIS_SELECTED != 0).count() as isize
            }
            LVM_GETEXTENDEDLISTVIEWSTYLE => with(hwnd, |lv| lv.ex).unwrap_or(0) as isize,
            LVM_SETEXTENDEDLISTVIEWSTYLE => {
                let mask = if w == 0 { u32::MAX } else { w as u32 };
                let old = with(hwnd, |lv| {
                    let old = lv.ex;
                    lv.ex = (lv.ex & !mask) | (l as u32 & mask);
                    old
                })
                .unwrap_or(0);
                invalidate(hwnd);
                old as isize
            }
            LVM_GETSELECTIONMARK => with(hwnd, |lv| lv.mark).unwrap_or(-1) as isize,
            LVM_SETSELECTIONMARK => with(hwnd, |lv| core::mem::replace(&mut lv.mark, l as i32)).unwrap_or(-1) as isize,
            LVM_GETSTRINGWIDTHW => {
                let t = wide_at(l as usize, -1);
                let font = with(hwnd, |lv| lv.font).unwrap_or(0);
                let m = Measure::new(hwnd, font);
                (text_width(m.hdc, &t) + 12) as isize
            }
            LVM_GETHEADER => {
                if style_of(hwnd) & LVS_TYPEMASK == LVS_REPORT { make_header(hwnd) as isize } else { 0 }
            }
            LVM_INSERTCOLUMNA | LVM_INSERTCOLUMNW => {
                if l == 0 {
                    return -1;
                }
                let c = read_column(l as usize);
                let mut col = col_from(&c);
                if msg == LVM_INSERTCOLUMNA && c.mask & LVCF_TEXT != 0 {
                    col.text = ansi_to_wide(c.text);
                }
                let at = with(hwnd, |lv| {
                    let at = (w as i32).clamp(0, lv.cols.len() as i32) as usize;
                    lv.cols.insert(at, col);
                    // Every item gets room for the new column's text.
                    for it in &mut lv.items {
                        if it.texts.len() < lv.cols.len() {
                            it.texts.resize(lv.cols.len(), None);
                        }
                    }
                    at
                })
                .unwrap_or(0);
                if style_of(hwnd) & LVS_TYPEMASK == LVS_REPORT {
                    make_header(hwnd);
                }
                after_columns(hwnd);
                at as isize
            }
            LVM_GETCOLUMNA | LVM_GETCOLUMNW => {
                let Some(col) = with(hwnd, |lv| lv.cols.get(w).cloned()).flatten() else { return 0 };
                if l == 0 {
                    return 0;
                }
                let mut c = read_column(l as usize);
                if c.mask & LVCF_FMT != 0 {
                    c.fmt = col.fmt | COL_FMT_FLAG;
                }
                if c.mask & LVCF_WIDTH != 0 {
                    c.cx = col.width;
                }
                if c.mask & LVCF_SUBITEM != 0 {
                    c.sub_item = col.sub_item;
                }
                if c.mask & LVCF_IMAGE != 0 {
                    c.image = col.image;
                }
                if c.mask & LVCF_ORDER != 0 {
                    c.order = col.order;
                }
                if c.mask & LVCF_TEXT != 0 {
                    if msg == LVM_GETCOLUMNW {
                        put_text(c.text, c.text_max.max(0) as usize, &col.text);
                    }
                }
                wr(l as usize, 0, c);
                1
            }
            LVM_SETCOLUMNA | LVM_SETCOLUMNW => {
                if l == 0 || w >= with(hwnd, |lv| lv.cols.len()).unwrap_or(0) {
                    return 0;
                }
                let c = read_column(l as usize);
                with(hwnd, |lv| {
                    let col = &mut lv.cols[w];
                    if c.mask & LVCF_FMT != 0 {
                        col.fmt = c.fmt;
                    }
                    if c.mask & LVCF_WIDTH != 0 {
                        col.width = c.cx;
                    }
                    if c.mask & LVCF_TEXT != 0 {
                        col.text = if msg == LVM_SETCOLUMNA { ansi_to_wide(c.text) } else { wide_at(c.text, -1) };
                    }
                    if c.mask & LVCF_SUBITEM != 0 {
                        col.sub_item = c.sub_item;
                    }
                    if c.mask & LVCF_IMAGE != 0 {
                        col.image = c.image;
                    }
                    if c.mask & LVCF_ORDER != 0 {
                        col.order = c.order;
                    }
                });
                after_columns(hwnd);
                1
            }
            LVM_DELETECOLUMN => {
                let ok = with(hwnd, |lv| {
                    if w < lv.cols.len() {
                        lv.cols.remove(w);
                        true
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
                after_columns(hwnd);
                isize::from(ok)
            }
            LVM_GETCOLUMNWIDTH => with(hwnd, |lv| lv.cols.get(w).map_or(0, |c| c.width)).unwrap_or(0) as isize,
            LVM_SETCOLUMNWIDTH => {
                let ok = with(hwnd, |lv| lv.cols.get(w).is_some()).unwrap_or(false);
                if !ok {
                    return 0;
                }
                let width = lo(l);
                let width = if width < 0 { column_auto_width(hwnd, w, width) } else { width };
                // The header tells its parent about the change before and after it.
                let hdr = with(hwnd, |lv| lv.header).unwrap_or(0);
                let mut nm = NmHeader { hdr: NmHdr::default(), item: w as i32, button: 0, pitem: 0 };
                if hdr != 0 {
                    notify_from(hdr, hwnd, HDN_ITEMCHANGINGW, &mut nm);
                }
                with(hwnd, |lv| lv.cols[w].width = width);
                after_columns(hwnd);
                if hdr != 0 {
                    notify_from(hdr, hwnd, HDN_ITEMCHANGEDW, &mut nm);
                }
                1
            }
            LVM_SORTITEMS | LVM_SORTITEMSEX => {
                sort_items(hwnd, msg == LVM_SORTITEMSEX, w, l);
                1
            }
            LVM_GETCOLUMNORDERARRAY => {
                let n = with(hwnd, |lv| lv.cols.len()).unwrap_or(0);
                if l != 0 {
                    for i in 0..n.min(w) {
                        wr(l as usize, 4 * i, i as i32);
                    }
                }
                1
            }
            LVM_SETCOLUMNORDERARRAY => 1,
            CCM_SETUNICODEFORMAT => 1,
            CCM_GETUNICODEFORMAT => 1,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

/// A notification of the header, sent by the list view on its behalf (a message the header
/// sends its parent, which the list view passes to its own).
fn notify_from(hdr: usize, hwnd: usize, code: i32, nm: &mut NmHeader) -> isize {
    // The header's notification goes to the list view's parent as it is: from the header's id.
    nm.hdr.hwnd_from = hdr;
    // SAFETY: a plain call.
    nm.hdr.id_from = unsafe { GetDlgCtrlID(hdr) } as usize;
    nm.hdr.code = code as u32;
    // SAFETY: a plain call.
    let parent = unsafe { GetParent(hwnd) };
    if parent == 0 {
        return 0;
    }
    // SAFETY: the structure is the caller's.
    unsafe { SendMessageW(parent, WM_NOTIFY, nm.hdr.id_from, nm as *mut NmHeader as isize) }
}

/// `LVSCW_AUTOSIZE` (-1): the widest text of the column; `LVSCW_AUTOSIZE_USEHEADER` (-2): or the heading's.
fn column_auto_width(hwnd: usize, col: usize, mode: i32) -> i32 {
    let n = item_count(hwnd);
    let font = with(hwnd, |lv| lv.font).unwrap_or(0);
    let m = Measure::new(hwnd, font);
    let mut best = 0;
    for i in 0..n.min(2000) {
        best = best.max(text_width(m.hdc, &cell_text(hwnd, i, col as i32, 260)));
    }
    if mode == -2 {
        let h = with(hwnd, |lv| lv.cols.get(col).map(|c| text_width(m.hdc, &c.text))).flatten().unwrap_or(0);
        best = best.max(h);
    }
    best + 12 + if col == 0 { icon_width(hwnd) } else { 0 }
}

/// `LVM_SORTITEMS` / `LVM_SORTITEMSEX`: the program's comparison function orders the items (a stable
/// insertion sort of the items taken out of the control, so the callback may use the control).
fn sort_items(hwnd: usize, by_index: bool, sort_param: usize, f: isize) {
    if f == 0 {
        return;
    }
    type Cmp = unsafe extern "system" fn(isize, isize, isize) -> i32;
    // SAFETY: the caller's comparison function.
    let cmp: Cmp = unsafe { core::mem::transmute(f) };
    let mut items = with(hwnd, |lv| core::mem::take(&mut lv.items)).unwrap_or_default();
    if by_index {
        // The function is given the indices; put the items back so that it can ask for them.
        with(hwnd, |lv| lv.items = items.clone());
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    for i in 1..order.len() {
        let mut j = i;
        while j > 0 {
            let (a, b) = (order[j - 1], order[j]);
            let (pa, pb) = if by_index { (a as isize, b as isize) } else { (items[a].param, items[b].param) };
            // SAFETY: the function the program gave.
            let r = unsafe { cmp(pa, pb, sort_param as isize) };
            if r > 0 {
                order.swap(j - 1, j);
                j -= 1;
            } else {
                break;
            }
        }
    }
    let sorted: Vec<Item> = order.iter().map(|&k| items[k].clone()).collect();
    items.clear();
    with(hwnd, |lv| lv.items = sorted);
    invalidate(hwnd);
}
