//! `SysTreeView32`: items and their order, expansion, selection, notifications, hit testing,
//! keyboard and mouse, and painting with lines and +/- buttons
//! (`specs/windows/comctl32.dll/SysTreeView32.yaml`; Learn, "Tree-View Control Reference").
//!
//! State lives in a table behind a spin lock that is never held while a message is sent.

use crate::cc::*;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_win32_gui::SpinLock;

const TVS_HASBUTTONS: u32 = 1;
const TVS_HASLINES: u32 = 2;
const TVS_LINESATROOT: u32 = 4;
const TVS_SHOWSELALWAYS: u32 = 0x20;
const TVS_NONEVENHEIGHT: u32 = 0x4000;

const TVIF_TEXT: u32 = 1;
const TVIF_IMAGE: u32 = 2;
const TVIF_PARAM: u32 = 4;
const TVIF_STATE: u32 = 8;
const TVIF_HANDLE: u32 = 0x10;
const TVIF_SELECTEDIMAGE: u32 = 0x20;
const TVIF_CHILDREN: u32 = 0x40;

const TVIS_SELECTED: u32 = 2;
const TVIS_DROPHILITED: u32 = 8;
const TVIS_BOLD: u32 = 0x10;
const TVIS_EXPANDED: u32 = 0x20;
const TVIS_EXPANDEDONCE: u32 = 0x40;

const TVE_COLLAPSE: u32 = 1;
const TVE_EXPAND: u32 = 2;
const TVE_TOGGLE: u32 = 3;
const TVE_COLLAPSERESET: u32 = 0x8000;

const TVGN_ROOT: u32 = 0;
const TVGN_NEXT: u32 = 1;
const TVGN_PREVIOUS: u32 = 2;
const TVGN_PARENT: u32 = 3;
const TVGN_CHILD: u32 = 4;
const TVGN_FIRSTVISIBLE: u32 = 5;
const TVGN_NEXTVISIBLE: u32 = 6;
const TVGN_PREVIOUSVISIBLE: u32 = 7;
const TVGN_DROPHILITE: u32 = 8;
const TVGN_CARET: u32 = 9;
const TVGN_LASTVISIBLE: u32 = 0xA;

const TVHT_NOWHERE: u32 = 1;
const TVHT_ONITEMICON: u32 = 2;
const TVHT_ONITEMLABEL: u32 = 4;
const TVHT_ONITEMINDENT: u32 = 8;
const TVHT_ONITEMBUTTON: u32 = 0x10;
const TVHT_ONITEMRIGHT: u32 = 0x20;
const TVHT_BELOW: u32 = 0x200;

const TVC_BYMOUSE: u32 = 1;
const TVC_BYKEYBOARD: u32 = 2;
/// Not a `TVC_` value: `TVM_EXPAND` with `TVE_TOGGLE` does not tell the parent (the sheet's test).
const QUIET: u32 = 0x100;

const TVN_FIRST: i32 = -400;
const TVN_SELCHANGINGW: i32 = TVN_FIRST - 50;
const TVN_SELCHANGEDW: i32 = TVN_FIRST - 51;
const TVN_GETDISPINFOW: i32 = TVN_FIRST - 52;
const TVN_ITEMEXPANDINGW: i32 = TVN_FIRST - 54;
const TVN_ITEMEXPANDEDW: i32 = TVN_FIRST - 55;
const TVN_DELETEITEMW: i32 = TVN_FIRST - 58;
const TVN_KEYDOWN: i32 = TVN_FIRST - 12;

const TVI_ROOT: usize = (-0x10000isize) as usize;
const TVI_FIRST: usize = (-0x0FFFFisize) as usize;
const TVI_LAST: usize = (-0x0FFFEisize) as usize;
const TVI_SORT: usize = (-0x0FFFDisize) as usize;

const LPSTR_TEXTCALLBACK: usize = usize::MAX;

/// `TVITEMW` (the members of `TVITEMEXW` after `lParam` are not used).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TvItem {
    mask: u32,
    item: usize,
    state: u32,
    state_mask: u32,
    text: usize,
    text_max: i32,
    image: i32,
    selected_image: i32,
    children: i32,
    param: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TvInsert {
    parent: usize,
    after: usize,
    item: TvItem,
}

#[repr(C)]
struct NmTreeView {
    hdr: NmHdr,
    action: u32,
    old: TvItem,
    new: TvItem,
    pt: [i32; 2],
}

#[repr(C)]
struct NmTvKeyDown {
    hdr: NmHdr,
    vkey: u16,
    flags: u32,
}

#[repr(C)]
struct NmTvDispInfo {
    hdr: NmHdr,
    item: TvItem,
}

#[derive(Clone, Debug)]
struct Node {
    parent: u32,
    kids: Vec<u32>,
    text: Vec<u16>,
    callback: bool,
    image: i32,
    selected_image: i32,
    param: isize,
    state: u32,
    children_flag: i32,
}

struct Tv {
    nodes: BTreeMap<u32, Node>,
    roots: Vec<u32>,
    next: u32,
    caret: u32,
    drop: u32,
    first_visible: u32,
    indent: i32,
    item_height: i32,
    bk: u32,
    text: u32,
    font: usize,
    images: [usize; 2],
}

static STATES: SpinLock<BTreeMap<usize, Tv>> = SpinLock::new(BTreeMap::new());

fn with<R>(h: usize, f: impl FnOnce(&mut Tv) -> R) -> Option<R> {
    STATES.with(|m| m.get_mut(&h).map(f))
}

const MIN_INDENT: i32 = 17;
const DEFAULT_INDENT: i32 = 19;

pub fn on_create(hwnd: usize) {
    STATES.with(|m| {
        m.insert(
            hwnd,
            Tv {
                nodes: BTreeMap::new(),
                roots: Vec::new(),
                next: 0,
                caret: 0,
                drop: 0,
                first_visible: 0,
                indent: DEFAULT_INDENT,
                item_height: 0,
                bk: CLR_NONE,
                text: CLR_NONE,
                font: 0,
                images: [0; 2],
            },
        )
    });
}

pub fn on_destroy(hwnd: usize) {
    let all: Vec<u32> = with(hwnd, |t| t.nodes.keys().copied().collect()).unwrap_or_default();
    for id in all {
        send_delete(hwnd, id);
    }
    STATES.with(|m| m.remove(&hwnd));
}

// ---- model helpers

fn kids_of(t: &Tv, parent: u32) -> &Vec<u32> {
    if parent == 0 { &t.roots } else { &t.nodes[&parent].kids }
}

fn level_of(t: &Tv, mut id: u32) -> i32 {
    let mut n = 0;
    while let Some(p) = t.nodes.get(&id).map(|x| x.parent) {
        if p == 0 {
            break;
        }
        n += 1;
        id = p;
    }
    n
}

/// The items shown, in order (those under collapsed parents are not).
fn visible(t: &Tv) -> Vec<u32> {
    fn walk(t: &Tv, list: &[u32], out: &mut Vec<u32>) {
        for &id in list {
            out.push(id);
            let n = &t.nodes[&id];
            if n.state & TVIS_EXPANDED != 0 && !n.kids.is_empty() {
                walk(t, &n.kids, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(t, &t.roots, &mut out);
    out
}

fn is_ancestor(t: &Tv, anc: u32, mut id: u32) -> bool {
    while let Some(n) = t.nodes.get(&id) {
        if n.parent == anc {
            return true;
        }
        if n.parent == 0 {
            return false;
        }
        id = n.parent;
    }
    false
}

fn tv_item_of(t: &Tv, id: u32, mask: u32) -> TvItem {
    match t.nodes.get(&id) {
        Some(n) => TvItem {
            mask,
            item: id as usize,
            state: if mask & TVIF_STATE != 0 {
                n.state
                    | if t.caret == id { TVIS_SELECTED } else { 0 }
                    | if t.drop == id { TVIS_DROPHILITED } else { 0 }
            } else {
                0
            },
            state_mask: 0,
            text: 0,
            text_max: 0,
            image: n.image,
            selected_image: n.selected_image,
            children: if n.children_flag != 0 { n.children_flag } else { i32::from(!n.kids.is_empty()) },
            param: n.param,
        },
        None => TvItem { mask, ..TvItem::default() },
    }
}

fn state_of(t: &Tv, id: u32) -> u32 {
    tv_item_of(t, id, TVIF_STATE).state
}

fn node_notify(hwnd: usize, code: i32, action: u32, old: Option<(u32, u32)>, new: Option<(u32, u32)>) -> isize {
    // (item, mask) pairs; the structures are filled from the model.
    let (oi, ni) = with(hwnd, |t| {
        (
            old.map_or(TvItem::default(), |(id, m)| tv_item_of(t, id, m)),
            new.map_or(TvItem::default(), |(id, m)| tv_item_of(t, id, m)),
        )
    })
    .unwrap_or((TvItem::default(), TvItem::default()));
    let mut nm = NmTreeView { hdr: NmHdr::default(), action, old: oi, new: ni, pt: [0; 2] };
    notify(hwnd, code, &mut nm)
}

fn send_delete(hwnd: usize, id: u32) {
    node_notify(hwnd, TVN_DELETEITEMW, 0, Some((id, TVIF_HANDLE | TVIF_PARAM)), None);
}

fn free_subtree(t: &mut Tv, id: u32, out: &mut Vec<u32>) {
    out.push(id);
    let kids = t.nodes.get(&id).map(|n| n.kids.clone()).unwrap_or_default();
    for k in kids {
        free_subtree(t, k, out);
    }
}

// ---- insertion, deletion

fn insert(hwnd: usize, p: usize, wide: bool) -> usize {
    // SAFETY: the caller's TVINSERTSTRUCT.
    let ins: TvInsert = unsafe { rd(p, 0) };
    let it = ins.item;
    let text = if it.mask & TVIF_TEXT != 0 && it.text != LPSTR_TEXTCALLBACK {
        if wide {
            // SAFETY: a NUL-terminated string.
            unsafe { wide_at(it.text, -1) }
        } else {
            ansi(it.text)
        }
    } else {
        Vec::new()
    };
    let callback = it.mask & TVIF_TEXT != 0 && it.text == LPSTR_TEXTCALLBACK;
    let parent = if ins.parent == TVI_ROOT || ins.parent == 0 { 0 } else { ins.parent as u32 };
    let r = with(hwnd, |t| {
        if parent != 0 && !t.nodes.contains_key(&parent) {
            return 0;
        }
        t.next += 1;
        let id = t.next;
        let node = Node {
            parent,
            kids: Vec::new(),
            text: text.clone(),
            callback,
            image: if it.mask & TVIF_IMAGE != 0 { it.image } else { 0 },
            selected_image: if it.mask & TVIF_SELECTEDIMAGE != 0 { it.selected_image } else { 0 },
            param: if it.mask & TVIF_PARAM != 0 { it.param } else { 0 },
            state: if it.mask & TVIF_STATE != 0 { it.state & it.state_mask & !TVIS_SELECTED } else { 0 },
            children_flag: if it.mask & TVIF_CHILDREN != 0 { it.children } else { 0 },
        };
        t.nodes.insert(id, node);
        let siblings: Vec<u32> = kids_of(t, parent).clone();
        let at = match ins.after {
            TVI_FIRST => 0,
            TVI_LAST => siblings.len(),
            TVI_SORT => {
                let lower = |s: &[u16]| -> Vec<u16> {
                    s.iter().map(|&c| if (0x41..=0x5A).contains(&c) { c + 32 } else { c }).collect()
                };
                let mine = lower(&text);
                siblings.iter().position(|s| lower(&t.nodes[s].text) > mine).unwrap_or(siblings.len())
            }
            a => siblings.iter().position(|&s| s as usize == a).map_or(siblings.len(), |i| i + 1),
        };
        if parent == 0 {
            t.roots.insert(at, id);
        } else {
            t.nodes.get_mut(&parent).unwrap().kids.insert(at, id);
        }
        id as usize
    })
    .unwrap_or(0);
    invalidate(hwnd);
    r
}

fn ansi(p: usize) -> Vec<u16> {
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

fn delete(hwnd: usize, item: usize) -> bool {
    let ids = with(hwnd, |t| {
        let mut ids = Vec::new();
        if item == TVI_ROOT {
            for r in t.roots.clone() {
                free_subtree(t, r, &mut ids);
            }
        } else if t.nodes.contains_key(&(item as u32)) && item <= u32::MAX as usize {
            free_subtree(t, item as u32, &mut ids);
        } else {
            return None;
        }
        Some(ids)
    })
    .flatten();
    let Some(ids) = ids else { return false };
    let caret = with(hwnd, |t| t.caret).unwrap_or(0);
    let replacement = if ids.contains(&caret) {
        // The next item takes the selection (or the previous one, or the parent).
        with(hwnd, |t| {
            let top = ids[0];
            let parent = t.nodes[&top].parent;
            let sibs = kids_of(t, parent).clone();
            let pos = sibs.iter().position(|&s| s == top).unwrap_or(0);
            sibs.get(pos + 1).or_else(|| pos.checked_sub(1).and_then(|p| sibs.get(p))).copied().unwrap_or(parent)
        })
        .unwrap_or(0)
    } else {
        0
    };
    for &id in &ids {
        send_delete(hwnd, id);
    }
    with(hwnd, |t| {
        for &id in &ids {
            if let Some(n) = t.nodes.remove(&id) {
                if n.parent == 0 {
                    t.roots.retain(|&r| r != id);
                } else if let Some(p) = t.nodes.get_mut(&n.parent) {
                    p.kids.retain(|&k| k != id);
                }
            }
        }
        if ids.contains(&t.caret) {
            t.caret = 0;
        }
        if ids.contains(&t.drop) {
            t.drop = 0;
        }
        if ids.contains(&t.first_visible) {
            t.first_visible = 0;
        }
    });
    if replacement != 0 && !ids.contains(&replacement) {
        select(hwnd, replacement, 0);
    }
    invalidate(hwnd);
    true
}

// ---- expansion and selection

fn expand(hwnd: usize, id: u32, flags: u32, action_hint: u32) -> bool {
    let Some((has_kids, expanded)) = with(hwnd, |t| {
        t.nodes.get(&id).map(|n| (!n.kids.is_empty() || n.children_flag != 0, n.state & TVIS_EXPANDED != 0))
    })
    .flatten() else {
        return false;
    };
    // Only 1, 2 and 3 (with the two flags above them) are actions.
    let code = flags & !0xC000;
    let want_expand = match code {
        TVE_EXPAND => true,
        TVE_COLLAPSE => false,
        TVE_TOGGLE => !expanded,
        _ => return false,
    };
    let quiet = action_hint & QUIET != 0;
    if want_expand == expanded {
        if !want_expand && flags & TVE_COLLAPSERESET != 0 {
            // (The item is collapsed already; the children go all the same.)
            // COLLAPSERESET removes the children.
            let kids = with(hwnd, |t| t.nodes[&id].kids.clone()).unwrap_or_default();
            for k in kids {
                delete(hwnd, k as usize);
            }
            with(hwnd, |t| {
                if let Some(n) = t.nodes.get_mut(&id) {
                    n.state &= !(TVIS_EXPANDEDONCE);
                }
            });
            return true;
        }
        // Expanding what is expanded works, collapsing what is collapsed does not.
        return want_expand;
    }
    if !has_kids && want_expand {
        // Nothing to show; the parent is still asked (TVN_ITEMEXPANDING) when it says the item has children.
        return false;
    }
    let action = if want_expand { TVE_EXPAND } else { TVE_COLLAPSE };
    // The parent hears of an expansion, not of a collapse (`TVM_EXPAND` in the sheet's test).
    if want_expand && !quiet && node_notify(hwnd, TVN_ITEMEXPANDINGW, action, None, Some((id, 0x3E))) != 0 {
        return false;
    }
    with(hwnd, |t| {
        if let Some(n) = t.nodes.get_mut(&id) {
            if want_expand {
                n.state |= TVIS_EXPANDED;
            } else {
                n.state &= !TVIS_EXPANDED;
            }
        }
        // A selected item below a collapsed one moves the selection up to it.
    });
    if want_expand {
        if !quiet {
            node_notify(hwnd, TVN_ITEMEXPANDEDW, action, None, Some((id, 0x3E)));
        }
        with(hwnd, |t| {
            if let Some(n) = t.nodes.get_mut(&id) {
                n.state |= TVIS_EXPANDEDONCE;
            }
        });
    }
    if !want_expand {
        let caret = with(hwnd, |t| t.caret).unwrap_or(0);
        let below = with(hwnd, |t| is_ancestor(t, id, caret)).unwrap_or(false);
        if below {
            select(hwnd, id, 0);
        }
        if flags & TVE_COLLAPSERESET != 0 {
            let kids = with(hwnd, |t| t.nodes[&id].kids.clone()).unwrap_or_default();
            for k in kids {
                delete(hwnd, k as usize);
            }
        }
    }
    invalidate(hwnd);
    true
}

/// Make `id` (0: none) the selected item: the parent may refuse, hidden items are shown first.
fn select(hwnd: usize, id: u32, action: u32) -> bool {
    let (old, exists) = with(hwnd, |t| (t.caret, id == 0 || t.nodes.contains_key(&id))).unwrap_or((0, false));
    if !exists {
        return false;
    }
    if old == id {
        return true;
    }
    let (new_mask, old_mask) = (0x1C, 0x1C);
    if node_notify(hwnd, TVN_SELCHANGINGW, action, Some((old, old_mask)), Some((id, new_mask))) != 0 {
        return false;
    }
    // Show the item: its collapsed ancestors expand.
    let ancestors: Vec<u32> = with(hwnd, |t| {
        let mut v = Vec::new();
        let mut cur = id;
        while let Some(n) = t.nodes.get(&cur) {
            if n.parent == 0 {
                break;
            }
            v.push(n.parent);
            cur = n.parent;
        }
        v.reverse();
        v.into_iter().filter(|a| t.nodes[a].state & TVIS_EXPANDED == 0).collect()
    })
    .unwrap_or_default();
    for a in ancestors {
        expand(hwnd, a, TVE_EXPAND, 0);
    }
    with(hwnd, |t| t.caret = id);
    node_notify(hwnd, TVN_SELCHANGEDW, action, Some((old, old_mask)), Some((id, new_mask)));
    ensure_visible(hwnd, id);
    invalidate(hwnd);
    true
}

// ---- geometry

fn font_height(hwnd: usize) -> i32 {
    let font = with(hwnd, |t| t.font).unwrap_or(0);
    let m = Measure::new(hwnd, font);
    font_metrics(m.hdc).0
}

fn row_height(hwnd: usize) -> i32 {
    let set = with(hwnd, |t| t.item_height).unwrap_or(0);
    if set > 0 {
        return set;
    }
    let mut h = font_height(hwnd) + 4;
    if style_of(hwnd) & TVS_NONEVENHEIGHT == 0 {
        h &= !1;
    }
    h.max(16)
}

fn indent(hwnd: usize) -> i32 {
    with(hwnd, |t| t.indent).unwrap_or(DEFAULT_INDENT)
}

/// The left edge of the content (button and lines) of items at level 0: with `TVS_LINESATROOT` the
/// roots have room for their button.
fn base_x(hwnd: usize) -> i32 {
    let s = style_of(hwnd);
    if s & (TVS_HASBUTTONS | TVS_HASLINES) != 0 && s & TVS_LINESATROOT != 0 { indent(hwnd) } else { 0 }
}

fn image_width(hwnd: usize) -> i32 {
    let il = with(hwnd, |t| t.images[0]).unwrap_or(0);
    if il == 0 {
        return 0;
    }
    let (mut cx, mut cy) = (0, 0);
    // SAFETY: our image list functions.
    unsafe { crate::imagelist::ImageList_GetIconSize(il, &mut cx, &mut cy) };
    cx + 2
}

fn text_of(hwnd: usize, id: u32) -> Vec<u16> {
    let stored = with(hwnd, |t| t.nodes.get(&id).map(|n| (n.callback, n.text.clone()))).flatten();
    match stored {
        Some((false, t)) => t,
        Some((true, _)) => {
            let mut buf = alloc::vec![0u16; 261];
            let mut nm = NmTvDispInfo {
                hdr: NmHdr::default(),
                item: TvItem {
                    mask: TVIF_TEXT,
                    item: id as usize,
                    text: buf.as_mut_ptr() as usize,
                    text_max: 260,
                    ..TvItem::default()
                },
            };
            let own = buf.as_ptr() as usize;
            notify(hwnd, TVN_GETDISPINFOW, &mut nm);
            if nm.item.text == own {
                buf.iter().take_while(|&&c| c != 0).copied().collect()
            } else {
                unsafe { wide_at(nm.item.text, -1) }
            }
        }
        None => Vec::new(),
    }
}

fn text_extent(hwnd: usize, id: u32) -> i32 {
    let t = text_of(hwnd, id);
    let bold = with(hwnd, |tv| tv.nodes.get(&id).is_some_and(|n| n.state & TVIS_BOLD != 0)).unwrap_or(false);
    let font = with(hwnd, |tv| tv.font).unwrap_or(0);
    let m = Measure::new(hwnd, font);
    text_width(m.hdc, &t) + if bold { 1 } else { 0 }
}

/// The row of `id` and its text rectangle, if it is shown.
fn item_rects(hwnd: usize, id: u32) -> Option<(Rc, Rc)> {
    let rh = row_height(hwnd);
    let base = base_x(hwnd);
    let ind = indent(hwnd);
    let (vis, first, level) = with(hwnd, |t| {
        let vis = visible(t);
        let first = if t.first_visible != 0 && vis.contains(&t.first_visible) {
            t.first_visible
        } else {
            vis.first().copied().unwrap_or(0)
        };
        (vis, first, level_of(t, id))
    })?;
    let pos = vis.iter().position(|&v| v == id)?;
    let fpos = vis.iter().position(|&v| v == first).unwrap_or(0);
    let y = (pos as i32 - fpos as i32) * rh;
    let c = client_rect(hwnd);
    let text_left = base + level * ind + image_width(hwnd) + 2;
    let text_w = text_extent(hwnd, id);
    let row = [0, y, rc_w(&c), y + rh];
    Some((row, [text_left, y, text_left + text_w + 4, y + rh]))
}

fn ensure_visible(hwnd: usize, id: u32) {
    if id == 0 {
        return;
    }
    let rh = row_height(hwnd).max(1);
    let rows = (rc_h(&client_rect(hwnd)) / rh).max(1);
    with(hwnd, |t| {
        let vis = visible(t);
        let Some(pos) = vis.iter().position(|&v| v == id) else { return };
        let first = if t.first_visible != 0 { vis.iter().position(|&v| v == t.first_visible).unwrap_or(0) } else { 0 };
        let want = if pos < first {
            pos
        } else if pos >= first + rows as usize {
            pos + 1 - rows as usize
        } else {
            first
        };
        t.first_visible = vis.get(want).copied().unwrap_or(0);
    });
}

fn hit_test(hwnd: usize, x: i32, y: i32) -> (u32, u32) {
    let c = client_rect(hwnd);
    if x < 0 || y < 0 || x >= rc_w(&c) {
        return (0, TVHT_NOWHERE);
    }
    let vis: Vec<u32> = with(hwnd, |t| visible(t)).unwrap_or_default();
    for id in vis {
        let Some((row, text)) = item_rects(hwnd, id) else { continue };
        if y < row[1] || y >= row[3] {
            continue;
        }
        let ind = indent(hwnd);
        let level = with(hwnd, |t| level_of(t, id)).unwrap_or(0);
        let btn_x = base_x(hwnd) + level * ind - ind;
        let img = image_width(hwnd);
        let has_kids = with(hwnd, |t| t.nodes.get(&id).is_some_and(|n| !n.kids.is_empty() || n.children_flag != 0))
            .unwrap_or(false);
        if style_of(hwnd) & TVS_HASBUTTONS != 0 && has_kids && x >= btn_x && x < btn_x + ind {
            return (id, TVHT_ONITEMBUTTON);
        }
        if x >= text[0] - img && x < text[0] && img > 0 {
            return (id, TVHT_ONITEMICON);
        }
        if x >= text[0] && x < text[2] {
            return (id, TVHT_ONITEMLABEL);
        }
        if x < text[0] {
            return (id, TVHT_ONITEMINDENT);
        }
        return (id, TVHT_ONITEMRIGHT);
    }
    (0, if y >= 0 { TVHT_BELOW } else { TVHT_NOWHERE })
}

// ---- painting

fn draw(hwnd: usize, hdc: usize) {
    let c = client_rect(hwnd);
    let Some((bk, tcolor, font, images)) = with(hwnd, |t| (t.bk, t.text, t.font, t.images)) else { return };
    let style = style_of(hwnd);
    fill(hdc, &c, if bk == CLR_NONE { sys_color(COLOR_WINDOW) } else { bk });
    // SAFETY: the DC and a font.
    let old = unsafe { SelectObject(hdc, if font != 0 { font } else { GetStockObject(DEFAULT_GUI_FONT) }) };
    // SAFETY: the DC.
    unsafe { SetBkMode(hdc, 1) };
    let focused = unsafe { GetFocus() } == hwnd;
    let vis: Vec<u32> = with(hwnd, |t| visible(t)).unwrap_or_default();
    let ind = indent(hwnd);
    let rh = row_height(hwnd);
    let base = base_x(hwnd);
    let line_color = sys_color(COLOR_BTNSHADOW);
    let always = style & TVS_SHOWSELALWAYS != 0;
    for &id in &vis {
        let Some((row, text)) = item_rects(hwnd, id) else { continue };
        if row[1] >= rc_h(&c) {
            break;
        }
        if row[3] <= 0 {
            continue;
        }
        let (level, has_kids, expanded, state, img, simg, is_caret, is_drop, last_sibling) = with(hwnd, |t| {
            let n = &t.nodes[&id];
            let sibs = kids_of(t, n.parent);
            (
                level_of(t, id),
                !n.kids.is_empty() || n.children_flag != 0,
                n.state & TVIS_EXPANDED != 0,
                n.state,
                n.image,
                n.selected_image,
                t.caret == id,
                t.drop == id,
                sibs.last() == Some(&id),
            )
        })
        .unwrap_or((0, false, false, 0, 0, 0, false, false, false));
        let mid = (row[1] + row[3]) / 2;
        // Lines and the button.
        let x0 = base + level * ind;
        if style & TVS_HASLINES != 0 {
            let lx = x0 - ind / 2;
            if level > 0 || style & TVS_LINESATROOT != 0 {
                fill(hdc, &[lx, row[1], lx + 1, if last_sibling { mid } else { row[3] }], line_color);
                fill(hdc, &[lx, mid, x0 - 1, mid + 1], line_color);
            }
        }
        if style & TVS_HASBUTTONS != 0 && has_kids && (level > 0 || style & TVS_LINESATROOT != 0) {
            let bx = x0 - ind / 2;
            let b = [bx - 4, mid - 4, bx + 5, mid + 5];
            fill(hdc, &b, sys_color(COLOR_WINDOW));
            frame(hdc, &b, sys_color(COLOR_BTNSHADOW));
            fill(hdc, &[bx - 2, mid, bx + 3, mid + 1], sys_color(COLOR_WINDOWTEXT));
            if !expanded {
                fill(hdc, &[bx, mid - 2, bx + 1, mid + 3], sys_color(COLOR_WINDOWTEXT));
            }
        }
        let il = images[0];
        if il != 0 {
            let which = if is_caret { simg } else { img };
            // SAFETY: our image list of the program.
            crate::imagelist::ImageList_Draw(il, which, hdc, x0 + 1, row[1] + (rh - 16) / 2, 1);
        }
        let selected = is_caret && (focused || always);
        let mut color = if tcolor == CLR_NONE { sys_color(COLOR_WINDOWTEXT) } else { tcolor };
        if selected {
            fill(hdc, &text, if focused { sys_color(COLOR_HIGHLIGHT) } else { sys_color(COLOR_BTNFACE) });
            if focused {
                color = sys_color(COLOR_HIGHLIGHTTEXT);
            }
        } else if is_drop {
            fill(hdc, &text, sys_color(COLOR_HIGHLIGHT));
            color = sys_color(COLOR_HIGHLIGHTTEXT);
        }
        let _ = state;
        // SAFETY: the DC.
        unsafe { SetTextColor(hdc, color) };
        draw_text(
            hdc,
            &text_of(hwnd, id),
            &[text[0] + 2, text[1], text[2], text[3]],
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        if is_caret && focused {
            focus_rect(hdc, &text, sys_color(COLOR_WINDOWTEXT));
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

// ---- navigation

fn next_visible(t: &Tv, id: u32, forward: bool) -> u32 {
    let vis = visible(t);
    let Some(p) = vis.iter().position(|&v| v == id) else { return 0 };
    if forward { vis.get(p + 1).copied().unwrap_or(0) } else { p.checked_sub(1).map_or(0, |q| vis[q]) }
}

fn get_next(hwnd: usize, kind: u32, item: usize) -> usize {
    let id = if item == TVI_ROOT { 0 } else { item as u32 };
    with(hwnd, |t| match kind {
        TVGN_ROOT => t.roots.first().copied().unwrap_or(0),
        TVGN_CHILD => {
            if id == 0 && item != TVI_ROOT && item != 0 {
                return 0;
            }
            if id != 0 && !t.nodes.contains_key(&id) {
                return 0;
            }
            kids_of(t, id).first().copied().unwrap_or(0)
        }
        TVGN_NEXT | TVGN_PREVIOUS => {
            let Some(n) = t.nodes.get(&id) else { return 0 };
            let sibs = kids_of(t, n.parent);
            let p = sibs.iter().position(|&s| s == id).unwrap_or(0);
            if kind == TVGN_NEXT {
                sibs.get(p + 1).copied().unwrap_or(0)
            } else {
                p.checked_sub(1).map_or(0, |q| sibs[q])
            }
        }
        TVGN_PARENT => t.nodes.get(&id).map_or(0, |n| n.parent),
        TVGN_FIRSTVISIBLE => {
            let vis = visible(t);
            if t.first_visible != 0 && vis.contains(&t.first_visible) {
                t.first_visible
            } else {
                vis.first().copied().unwrap_or(0)
            }
        }
        TVGN_NEXTVISIBLE => next_visible(t, id, true),
        TVGN_PREVIOUSVISIBLE => next_visible(t, id, false),
        TVGN_LASTVISIBLE => visible(t).last().copied().unwrap_or(0),
        TVGN_DROPHILITE => t.drop,
        TVGN_CARET => t.caret,
        _ => 0,
    })
    .unwrap_or(0) as usize
}

fn key(hwnd: usize, vk: u32) {
    let mut nm = NmTvKeyDown { hdr: NmHdr::default(), vkey: vk as u16, flags: 0 };
    notify(hwnd, TVN_KEYDOWN, &mut nm);
    let Some((caret, vis)) = with(hwnd, |t| (t.caret, visible(t))) else { return };
    if vis.is_empty() {
        return;
    }
    let pos = vis.iter().position(|&v| v == caret);
    let rows = (rc_h(&client_rect(hwnd)) / row_height(hwnd).max(1)).max(1) as usize;
    let target = match vk {
        VK_DOWN => pos.map_or(vis[0], |p| vis[(p + 1).min(vis.len() - 1)]),
        VK_UP => pos.map_or(vis[0], |p| vis[p.saturating_sub(1)]),
        VK_HOME => vis[0],
        VK_END => *vis.last().unwrap(),
        VK_NEXT => pos.map_or(vis[0], |p| vis[(p + rows).min(vis.len() - 1)]),
        VK_PRIOR => pos.map_or(vis[0], |p| vis[p.saturating_sub(rows)]),
        VK_RIGHT | VK_ADD => {
            if caret != 0 {
                let (exp, kids) =
                    with(hwnd, |t| (t.nodes[&caret].state & TVIS_EXPANDED != 0, t.nodes[&caret].kids.clone()))
                        .unwrap_or((false, Vec::new()));
                if !exp && (!kids.is_empty() || with(hwnd, |t| t.nodes[&caret].children_flag != 0).unwrap_or(false)) {
                    expand(hwnd, caret, TVE_EXPAND, TVC_BYKEYBOARD);
                    return;
                }
                if exp && vk == VK_RIGHT {
                    kids.first().copied().unwrap_or(caret)
                } else {
                    return;
                }
            } else {
                vis[0]
            }
        }
        VK_LEFT | VK_SUBTRACT => {
            if caret != 0 {
                let (exp, parent) =
                    with(hwnd, |t| (t.nodes[&caret].state & TVIS_EXPANDED != 0, t.nodes[&caret].parent))
                        .unwrap_or((false, 0));
                if exp {
                    expand(hwnd, caret, TVE_COLLAPSE, TVC_BYKEYBOARD);
                    return;
                }
                if vk == VK_LEFT && parent != 0 {
                    parent
                } else {
                    return;
                }
            } else {
                vis[0]
            }
        }
        _ => return,
    };
    select(hwnd, target, TVC_BYKEYBOARD);
}

fn click(hwnd: usize, x: i32, y: i32, double: bool) {
    // SAFETY: a plain call.
    unsafe { SetFocus(hwnd) };
    let (id, flags) = hit_test(hwnd, x, y);
    if id != 0 {
        if flags & TVHT_ONITEMBUTTON != 0 {
            expand(hwnd, id, TVE_TOGGLE, TVC_BYMOUSE);
        } else if flags & (TVHT_ONITEMLABEL | TVHT_ONITEMICON | TVHT_ONITEMINDENT | TVHT_ONITEMRIGHT) != 0 {
            select(hwnd, id, TVC_BYMOUSE);
            if double {
                expand(hwnd, id, TVE_TOGGLE, TVC_BYMOUSE);
            }
        }
    }
    notify_plain(hwnd, if double { NM_DBLCLK } else { NM_CLICK });
    invalidate(hwnd);
}

// ---- the window procedure

const TVM_INSERTITEMA: u32 = 0x1100;
const TVM_DELETEITEM: u32 = 0x1101;
const TVM_EXPAND: u32 = 0x1102;
const TVM_GETITEMRECT: u32 = 0x1104;
const TVM_GETCOUNT: u32 = 0x1105;
const TVM_GETINDENT: u32 = 0x1106;
const TVM_SETINDENT: u32 = 0x1107;
const TVM_GETIMAGELIST: u32 = 0x1108;
const TVM_SETIMAGELIST: u32 = 0x1109;
const TVM_GETNEXTITEM: u32 = 0x110A;
const TVM_SELECTITEM: u32 = 0x110B;
const TVM_GETITEMA: u32 = 0x110C;
const TVM_SETITEMA: u32 = 0x110D;
const TVM_GETVISIBLECOUNT: u32 = 0x1110;
const TVM_HITTEST: u32 = 0x1111;
const TVM_SORTCHILDREN: u32 = 0x1113;
const TVM_ENSUREVISIBLE: u32 = 0x1114;
const TVM_SETITEMHEIGHT: u32 = 0x111B;
const TVM_GETITEMHEIGHT: u32 = 0x111C;
const TVM_SETBKCOLOR: u32 = 0x111D;
const TVM_SETTEXTCOLOR: u32 = 0x111E;
const TVM_GETBKCOLOR: u32 = 0x111F;
const TVM_GETTEXTCOLOR: u32 = 0x1120;
const TVM_GETITEMSTATE: u32 = 0x1127;
const TVM_INSERTITEMW: u32 = 0x1132;
const TVM_GETITEMW: u32 = 0x113E;
const TVM_SETITEMW: u32 = 0x113F;
const TVM_SETEXTENDEDSTYLE: u32 = 0x112C;
const TVM_GETEXTENDEDSTYLE: u32 = 0x112D;
const CCM_SETUNICODEFORMAT: u32 = 0x2005;
const CCM_GETUNICODEFORMAT: u32 = 0x2006;

fn get_item(hwnd: usize, l: usize, wide: bool) -> bool {
    // SAFETY: the caller's TVITEM.
    let mut it: TvItem = unsafe { rd(l, 0) };
    let id = it.item as u32;
    if it.item > u32::MAX as usize || !with(hwnd, |t| t.nodes.contains_key(&id)).unwrap_or(false) {
        return false;
    }
    let full = with(hwnd, |t| tv_item_of(t, id, it.mask)).unwrap_or_default();
    if it.mask & TVIF_TEXT != 0 {
        let t = text_of(hwnd, id);
        let cap = it.text_max.max(0) as usize;
        if wide {
            // SAFETY: the caller's buffer.
            unsafe { put_text(it.text, cap, &t) };
        } else {
            let a = poly_kernelbase::text::to_ansi(&t);
            if cap > 0 && it.text != 0 {
                let n = a.len().min(cap - 1);
                // SAFETY: the caller's buffer.
                unsafe {
                    core::ptr::copy_nonoverlapping(a.as_ptr(), it.text as *mut u8, n);
                    *(it.text as *mut u8).add(n) = 0;
                }
            }
        }
    }
    if it.mask & TVIF_IMAGE != 0 {
        it.image = full.image;
    }
    if it.mask & TVIF_SELECTEDIMAGE != 0 {
        it.selected_image = full.selected_image;
    }
    if it.mask & TVIF_PARAM != 0 {
        it.param = full.param;
    }
    if it.mask & TVIF_CHILDREN != 0 {
        it.children = full.children;
        if full.children == -1 {
            // I_CHILDRENCALLBACK: the parent says whether the item has children.
            let mut nm = NmTvDispInfo { hdr: NmHdr::default(), item: TvItem { mask: TVIF_CHILDREN, item: it.item, ..TvItem::default() } };
            notify(hwnd, TVN_GETDISPINFOW, &mut nm);
            it.children = nm.item.children;
        }
    }
    if it.mask & TVIF_STATE != 0 {
        it.state = full.state & it.state_mask;
    }
    // SAFETY: the caller's TVITEM: only the members of the mask changed.
    unsafe {
        wr(l, core::mem::offset_of!(TvItem, state), it.state);
        wr(l, core::mem::offset_of!(TvItem, image), it.image);
        wr(l, core::mem::offset_of!(TvItem, selected_image), it.selected_image);
        wr(l, core::mem::offset_of!(TvItem, children), it.children);
        wr(l, core::mem::offset_of!(TvItem, param), it.param);
    }
    true
}

fn set_item(hwnd: usize, l: usize, wide: bool) -> bool {
    // SAFETY: the caller's TVITEM.
    let it: TvItem = unsafe { rd(l, 0) };
    let id = it.item as u32;
    if it.item > u32::MAX as usize {
        return false;
    }
    let text = if it.mask & TVIF_TEXT != 0 && it.text != LPSTR_TEXTCALLBACK {
        Some(if wide { unsafe { wide_at(it.text, -1) } } else { ansi(it.text) })
    } else {
        None
    };
    let ok = with(hwnd, |t| match t.nodes.get_mut(&id) {
        Some(n) => {
            if let Some(tx) = text {
                n.text = tx;
                n.callback = false;
            } else if it.mask & TVIF_TEXT != 0 {
                n.callback = true;
            }
            if it.mask & TVIF_IMAGE != 0 {
                n.image = it.image;
            }
            if it.mask & TVIF_SELECTEDIMAGE != 0 {
                n.selected_image = it.selected_image;
            }
            if it.mask & TVIF_PARAM != 0 {
                n.param = it.param;
            }
            if it.mask & TVIF_CHILDREN != 0 {
                n.children_flag = it.children;
            }
            if it.mask & TVIF_STATE != 0 {
                n.state = (n.state & !it.state_mask) | (it.state & it.state_mask & !TVIS_SELECTED);
            }
            true
        }
        None => false,
    })
    .unwrap_or(false);
    invalidate(hwnd);
    ok
}

fn sort_children(hwnd: usize, parent: usize, recurse: bool) {
    let id = if parent == TVI_ROOT { 0 } else { parent as u32 };
    let lower =
        |s: &[u16]| -> Vec<u16> { s.iter().map(|&c| if (0x41..=0x5A).contains(&c) { c + 32 } else { c }).collect() };
    fn go(t: &mut Tv, id: u32, recurse: bool, lower: &dyn Fn(&[u16]) -> Vec<u16>) {
        let mut ks = kids_of(t, id).clone();
        ks.sort_by_key(|k| lower(&t.nodes[k].text));
        if id == 0 {
            t.roots = ks.clone();
        } else {
            t.nodes.get_mut(&id).unwrap().kids = ks.clone();
        }
        if recurse {
            for k in ks {
                go(t, k, true, lower);
            }
        }
    }
    with(hwnd, |t| {
        if id == 0 || t.nodes.contains_key(&id) {
            go(t, id, recurse, &lower);
        }
    });
    invalidate(hwnd);
}

pub fn proc(hwnd: usize, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: the messages' pointers are the caller's structures (`commctrl.h`).
    unsafe {
        match msg {
            WM_PAINT => {
                paint_event(hwnd);
                0
            }
            WM_PRINT | WM_PRINTCLIENT => {
                draw(hwnd, w);
                0
            }
            WM_ERASEBKGND => 1,
            WM_SIZE => {
                invalidate(hwnd);
                0
            }
            WM_SETFONT => {
                with(hwnd, |t| t.font = w);
                invalidate(hwnd);
                0
            }
            WM_GETFONT => with(hwnd, |t| t.font).unwrap_or(0) as isize,
            WM_SETFOCUS | WM_KILLFOCUS => {
                notify_plain(hwnd, if msg == WM_SETFOCUS { NM_SETFOCUS } else { NM_KILLFOCUS });
                invalidate(hwnd);
                0
            }
            WM_GETDLGCODE => 0x0001 | 0x0004 | 0x0080,
            WM_KEYDOWN => {
                key(hwnd, w as u32);
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
                notify_plain(hwnd, NM_RCLICK);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = hi(w as isize) / 120;
                let first = with(hwnd, |t| {
                    let vis = visible(t);
                    let p = if t.first_visible != 0 {
                        vis.iter().position(|&v| v == t.first_visible).unwrap_or(0)
                    } else {
                        0
                    };
                    let q = (p as i32 - delta * 3).clamp(0, (vis.len() as i32 - 1).max(0)) as usize;
                    t.first_visible = vis.get(q).copied().unwrap_or(0);
                })
                .is_some();
                let _ = first;
                invalidate(hwnd);
                0
            }
            TVM_INSERTITEMA | TVM_INSERTITEMW => {
                if l == 0 {
                    return 0;
                }
                insert(hwnd, l as usize, msg == TVM_INSERTITEMW) as isize
            }
            TVM_DELETEITEM => isize::from(delete(hwnd, l as usize)),
            TVM_EXPAND => {
                let id = l as u32;
                if l as usize > u32::MAX as usize {
                    return 0;
                }
                isize::from(expand(hwnd, id, w as u32, if w as u32 & !0xC000 == TVE_TOGGLE { QUIET } else { 0 }))
            }
            TVM_GETITEMRECT => {
                if l == 0 {
                    return 0;
                }
                let id = rd::<usize>(l as usize, 0) as u32;
                match item_rects(hwnd, id) {
                    Some((row, text)) => {
                        let r = if w != 0 { text } else { [row[0], row[1], row[2], row[3]] };
                        for (k, v) in r.iter().enumerate() {
                            wr(l as usize, 4 * k, *v);
                        }
                        1
                    }
                    None => 0,
                }
            }
            TVM_GETCOUNT => with(hwnd, |t| t.nodes.len()).unwrap_or(0) as isize,
            TVM_GETINDENT => indent(hwnd) as isize,
            TVM_SETINDENT => {
                with(hwnd, |t| t.indent = (w as i32).max(MIN_INDENT));
                invalidate(hwnd);
                0
            }
            TVM_GETIMAGELIST => with(hwnd, |t| t.images.get(w).copied().unwrap_or(0)).unwrap_or(0) as isize,
            TVM_SETIMAGELIST => {
                let old = with(hwnd, |t| t.images.get_mut(w).map(|s| core::mem::replace(s, l as usize)).unwrap_or(0))
                    .unwrap_or(0);
                invalidate(hwnd);
                old as isize
            }
            TVM_GETNEXTITEM => get_next(hwnd, w as u32, l as usize) as isize,
            TVM_SELECTITEM => {
                let id = if l as usize > u32::MAX as usize { u32::MAX } else { l as u32 };
                match w as u32 {
                    TVGN_CARET => isize::from(select(hwnd, id, 0) || with(hwnd, |t| t.caret == id).unwrap_or(false)),
                    TVGN_DROPHILITE => {
                        let ok = with(hwnd, |t| {
                            if id == 0 || t.nodes.contains_key(&id) {
                                t.drop = id;
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                        invalidate(hwnd);
                        isize::from(ok)
                    }
                    TVGN_FIRSTVISIBLE => {
                        let ok = with(hwnd, |t| {
                            let vis = visible(t);
                            if vis.contains(&id) {
                                t.first_visible = id;
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                        invalidate(hwnd);
                        isize::from(ok)
                    }
                    _ => 0,
                }
            }
            TVM_GETITEMA | TVM_GETITEMW => isize::from(l != 0 && get_item(hwnd, l as usize, msg == TVM_GETITEMW)),
            TVM_SETITEMA | TVM_SETITEMW => isize::from(l != 0 && set_item(hwnd, l as usize, msg == TVM_SETITEMW)),
            TVM_GETVISIBLECOUNT => {
                let rows = rc_h(&client_rect(hwnd)) / row_height(hwnd).max(1);
                rows as isize
            }
            TVM_HITTEST => {
                if l == 0 {
                    return 0;
                }
                let (id, flags) = hit_test(hwnd, rd::<i32>(l as usize, 0), rd::<i32>(l as usize, 4));
                wr(l as usize, 8, flags);
                wr(l as usize, 8 + core::mem::size_of::<usize>(), id as usize);
                id as isize
            }
            TVM_SORTCHILDREN => {
                sort_children(hwnd, l as usize, w != 0);
                1
            }
            TVM_ENSUREVISIBLE => {
                let id = l as u32;
                let ancestors: Vec<u32> = with(hwnd, |t| {
                    let mut v = Vec::new();
                    let mut cur = id;
                    while let Some(n) = t.nodes.get(&cur) {
                        if n.parent == 0 {
                            break;
                        }
                        v.push(n.parent);
                        cur = n.parent;
                    }
                    v.reverse();
                    v.into_iter().filter(|a| t.nodes[a].state & TVIS_EXPANDED == 0).collect()
                })
                .unwrap_or_default();
                for a in ancestors {
                    expand(hwnd, a, TVE_EXPAND, 0);
                }
                ensure_visible(hwnd, id);
                0
            }
            TVM_SETITEMHEIGHT => {
                let old = row_height(hwnd);
                let mut h = w as i32;
                if h > 0 && style_of(hwnd) & TVS_NONEVENHEIGHT == 0 {
                    h &= !1;
                }
                with(hwnd, |t| t.item_height = if h < 0 { 0 } else { h });
                invalidate(hwnd);
                old as isize
            }
            TVM_GETITEMHEIGHT => row_height(hwnd) as isize,
            TVM_SETBKCOLOR => {
                invalidate(hwnd);
                with(hwnd, |t| core::mem::replace(&mut t.bk, l as u32)).unwrap_or(CLR_NONE) as isize
            }
            TVM_SETTEXTCOLOR => {
                invalidate(hwnd);
                with(hwnd, |t| core::mem::replace(&mut t.text, l as u32)).unwrap_or(CLR_NONE) as isize
            }
            TVM_GETBKCOLOR => with(hwnd, |t| t.bk).unwrap_or(CLR_NONE) as isize,
            TVM_GETTEXTCOLOR => with(hwnd, |t| t.text).unwrap_or(CLR_NONE) as isize,
            TVM_GETITEMSTATE => with(hwnd, |t| {
                if t.nodes.contains_key(&(w as u32)) { (state_of(t, w as u32) & l as u32) as isize } else { 0 }
            })
            .unwrap_or(0),
            TVM_SETEXTENDEDSTYLE | TVM_GETEXTENDEDSTYLE => 0,
            CCM_SETUNICODEFORMAT | CCM_GETUNICODEFORMAT => 1,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
