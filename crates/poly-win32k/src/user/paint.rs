//! Painting in `user32` (ADR 0131, slice B): BeginPaint / EndPaint, the
//! update region functions, UpdateWindow, RedrawWindow and the erase and
//! frame messages, over win32k's update regions (`crate::wm::paint`).
//! Everything follows the slice B sheets (BeginPaint.yaml, InvalidateRect.yaml,
//! ValidateRect.yaml, UpdateWindow.yaml, GetUpdateRect.yaml,
//! GetUpdateRgn.yaml, RedrawWindow.yaml).

use super::*;
use crate::wm::paint::*;

/// `WM_PAINT`, `WM_NCPAINT` (winuser.h).
pub const WM_PAINT: u32 = 0x000F;
pub const WM_NCPAINT: u32 = 0x0085;

/// What win32k has pending for a window's painting.
#[derive(Clone, Debug, Default)]
pub struct Pending {
    pub rects: Vec<Rect>,
    pub erase: bool,
    pub frame: bool,
    pub frame_whole: bool,
    pub internal: bool,
    pub ferase: bool,
    pub visible: bool,
}

impl Pending {
    /// The update region's bounding box, client coordinates.
    pub fn bounds(&self) -> Rect {
        crate::rgn::Rgn::from_rects(&self.rects).bounds()
    }
}

/// `PAINTSTRUCT` (winuser.h): 72 bytes on x64.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct PaintStruct {
    pub hdc: u64,
    pub erase: i32,
    pub paint: CRect,
    pub restore: i32,
    pub inc_update: i32,
    pub reserved: [u8; 32],
}

pub fn pending<E: Env + ?Sized>(env: &E, hwnd: u32) -> Option<Pending> {
    match env.call(Request::Update { hwnd }) {
        Reply::Update { rects, erase, frame, frame_whole, internal, ferase, visible } => {
            Some(Pending { rects, erase, frame, frame_whole, internal, ferase, visible })
        }
        _ => None,
    }
}

fn painted<E: Env + ?Sized>(env: &E, hwnd: u32, what: u32, ferase: u8) {
    env.call(Request::Painted { hwnd, what, ferase });
}

fn redraw<E: Env + ?Sized>(env: &E, hwnd: u32, rects: Option<&[Rect]>, flags: u32) -> bool {
    let (rects, whole) = match rects {
        Some(r) => (r.to_vec(), false),
        None => (Vec::new(), true),
    };
    matches!(env.call(Request::Redraw { hwnd, rects, whole, flags }), Reply::Ok {})
}

/// `WM_NCPAINT` for a pending frame: wParam 1 for all of the window, else
/// a region (screen coordinates) the procedure may read.
fn send_frame<E: Env + ?Sized>(env: &E, i: &WinInfo, p: &Pending) {
    if p.frame_whole {
        send(env, u64::from(i.hwnd), WM_NCPAINT, 1, 0);
    } else {
        let w = i.window;
        let rgn = env.make_region(&[w]);
        send(env, u64::from(i.hwnd), WM_NCPAINT, rgn, 0);
        env.delete_object(rgn);
    }
    painted(env, i.hwnd, PAINTED_FRAME, 0);
}

/// Send what is pending of the frame and the erase now (GetUpdateRect with
/// erase, RDW_ERASENOW, a window being shown): the update region stays.
pub fn erase_now<E: Env + ?Sized>(env: &E, hwnd: u32) {
    let Some(i) = info(env, u64::from(hwnd)) else { return };
    let Some(p) = pending(env, hwnd) else { return };
    if !p.visible {
        return;
    }
    if p.frame {
        send_frame(env, &i, &p);
    }
    if p.erase {
        let dc = env.paint_dc(hwnd, Some(&p.rects));
        let r = send(env, u64::from(hwnd), WM_ERASEBKGND, dc, 0);
        env.release_dc(hwnd, dc);
        painted(env, hwnd, PAINTED_ERASE, if r == 0 { 1 } else { 2 });
    }
}

/// `BeginPaint` (BeginPaint.yaml).
pub fn begin_paint<E: Env + ?Sized>(env: &E, hwnd: u64, ps: &mut PaintStruct) -> u64 {
    *ps = PaintStruct::default();
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    let h = i.hwnd;
    let Some(p) = pending(env, h) else { return 0 };
    if !p.visible {
        // A hidden window: a DC with nothing to paint.
        let dc = env.paint_dc(h, Some(&[]));
        ps.hdc = dc;
        return dc;
    }
    if p.frame {
        send_frame(env, &i, &p);
    }
    let dc = env.paint_dc(h, Some(&p.rects));
    let erase = if p.erase {
        let r = send(env, hwnd, WM_ERASEBKGND, dc, 0);
        r == 0
    } else {
        p.ferase
    };
    painted(env, h, PAINTED_VALIDATE, if p.erase { if erase { 1 } else { 2 } } else { 0 });
    ps.hdc = dc;
    ps.erase = i32::from(erase);
    ps.paint = CRect::from(p.bounds());
    dc
}

/// `EndPaint` (EndPaint.yaml): the paint DC goes.
pub fn end_paint<E: Env + ?Sized>(env: &E, hwnd: u64, ps: &PaintStruct) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    if ps.hdc != 0 {
        env.release_dc(i.hwnd, ps.hdc);
    }
    true
}

/// `InvalidateRect` / `InvalidateRgn` (rects in client coordinates, `None`
/// for all of the client area): 1 also for a bad handle, which sets
/// ERROR_INVALID_WINDOW_HANDLE (InvalidateRect.yaml).
pub fn invalidate<E: Env + ?Sized>(env: &E, hwnd: u64, rects: Option<&[Rect]>, erase: bool) -> bool {
    if hwnd == 0 {
        return true;
    }
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return true;
    };
    let rects: Option<Vec<Rect>> = rects.map(|r| r.iter().map(|&x| normalized(x)).collect());
    redraw(env, i.hwnd, rects.as_deref(), RDW_INVALIDATE | if erase { RDW_ERASE } else { 0 });
    true
}

/// `ValidateRect` / `ValidateRgn`: children too, as InvalidateRect reaches
/// them (observed in vr_basic: a validated parent's child has nothing
/// invalid).
pub fn validate<E: Env + ?Sized>(env: &E, hwnd: u64, rects: Option<&[Rect]>) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    redraw(env, i.hwnd, rects, RDW_VALIDATE);
    true
}

fn normalized(r: Rect) -> Rect {
    if r.right < r.left || r.bottom < r.top {
        Rect::new(r.left.min(r.right), r.top.min(r.bottom), r.left.max(r.right), r.top.max(r.bottom))
    } else {
        r
    }
}

/// `UpdateWindow` (UpdateWindow.yaml): one WM_PAINT, sent, to the window
/// and to each of its invalid descendants.
pub fn update_window<E: Env + ?Sized>(env: &E, hwnd: u64) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    let order = match env.call(Request::PaintOrder { root: i.hwnd }) {
        Reply::Windows { hwnds } => hwnds,
        _ => Vec::new(),
    };
    for h in order {
        send(env, u64::from(h), WM_PAINT, 0, 0);
    }
    true
}

/// `RedrawWindow` (RedrawWindow.yaml): `rects` the region or rectangle given
/// (client coordinates), `None` for all of the client area.
pub fn redraw_window<E: Env + ?Sized>(env: &E, hwnd: u64, rects: Option<&[Rect]>, flags: u32) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    let region_flags = flags & !(RDW_UPDATENOW | RDW_ERASENOW);
    if region_flags != 0 {
        redraw(env, i.hwnd, rects, region_flags);
    }
    if flags & RDW_UPDATENOW != 0 {
        update_window(env, hwnd);
    } else if flags & RDW_ERASENOW != 0 {
        erase_now(env, i.hwnd);
    }
    true
}

/// `GetUpdateRect` / `GetUpdateRgn`'s common part: what is invalid after
/// sending the pending erase when asked (`None` for a bad handle).
pub fn update_area<E: Env + ?Sized>(env: &E, hwnd: u64, erase: bool) -> Option<(Pending, WinInfo)> {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return None;
    };
    let mut p = pending(env, i.hwnd)?;
    if erase && (p.erase || p.frame) {
        erase_now(env, i.hwnd);
        p = pending(env, i.hwnd)?;
    }
    Some((p, i))
}

/// `GetUpdateRect`: whether anything is invalid, and the bounding box.
pub fn get_update_rect<E: Env + ?Sized>(env: &E, hwnd: u64, erase: bool) -> Option<(bool, Rect)> {
    let (p, i) = update_area(env, hwnd, erase)?;
    let any = !p.rects.is_empty() || (i.minimized && p.internal);
    Some((any, if p.rects.is_empty() { Rect::default() } else { p.bounds() }))
}

/// The paint work of a window becoming visible in an interactive session
/// (UpdateWindow.yaml `show`, `creation`; GetUpdateRgn.yaml gurg_children):
/// a top-level window, or a child of a `WS_CLIPCHILDREN` parent, gets its
/// frame and erase at once; a child's parent repaints (and erases now) the
/// area under it.
pub fn shown_paint<E: Env + ?Sized>(env: &E, i: &WinInfo) {
    if !screen(env).interactive {
        return;
    }
    let child = i.style & WS_CHILD != 0;
    let parent = if child { info(env, u64::from(i.parent)) } else { None };
    if child && let Some(p) = &parent {
        expose_parent(env, p, &[i.window_rel]);
    }
    if !child || parent.is_some_and(|p| p.style & WS_CLIPCHILDREN != 0) {
        erase_now(env, i.hwnd);
    }
}

/// A child's area `rects` (its parent's client coordinates) became visible
/// or covered differently: the parent repaints it, erased now.
pub fn expose_parent<E: Env + ?Sized>(env: &E, parent: &WinInfo, rects: &[Rect]) {
    if !screen(env).interactive || !parent.visible || rects.iter().all(|r| r.right <= r.left || r.bottom <= r.top) {
        return;
    }
    redraw(env, parent.hwnd, Some(rects), RDW_INVALIDATE | RDW_ERASE | RDW_NOCHILDREN);
    erase_now(env, parent.hwnd);
}

/// A window's size changed in an interactive session (UpdateWindow.yaml
/// `resize_plain`, `resize_redraw`): what grew (or all of it with
/// CS_HREDRAW / CS_VREDRAW) is invalid and erased now, with the frame.
pub fn resized_paint<E: Env + ?Sized>(env: &E, hwnd: u32, old_client: Rect, new_client: Rect) {
    const CS_VREDRAW: u32 = 0x0001;
    const CS_HREDRAW: u32 = 0x0002;
    if !screen(env).interactive {
        return;
    }
    let Some(i) = info(env, u64::from(hwnd)) else { return };
    if !i.visible {
        return;
    }
    let (ow, oh) = (old_client.width().max(0), old_client.height().max(0));
    let (nw, nh) = (new_client.width().max(0), new_client.height().max(0));
    if i.class_style & (CS_HREDRAW | CS_VREDRAW) != 0 {
        redraw(env, hwnd, None, RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_NOCHILDREN);
    } else if nw > ow || nh > oh {
        let grown =
            crate::rgn::Rgn::rect(Rect::new(0, 0, nw, nh)).subtract(&crate::rgn::Rgn::rect(Rect::new(0, 0, ow, oh)));
        redraw(env, hwnd, Some(grown.rects()), RDW_INVALIDATE | RDW_ERASE | RDW_NOCHILDREN);
        // The frame moved: a part of the window to repaint (a region).
        redraw(env, hwnd, Some(&[]), RDW_INVALIDATE | RDW_FRAME | RDW_NOCHILDREN);
        mark_frame_partial(env, hwnd);
    } else {
        return;
    }
    erase_now(env, hwnd);
}

/// A frame change that redraws everything (`SetWindowRgn(NULL)`,
/// `SetWindowRgn.yaml`): the client invalid and erased at once, the frame
/// repainted as a region.
pub fn frame_redraw<E: Env + ?Sized>(env: &E, hwnd: u32) {
    redraw(env, hwnd, None, RDW_INVALIDATE | RDW_ERASE | RDW_NOCHILDREN);
    redraw(env, hwnd, Some(&[]), RDW_INVALIDATE | RDW_FRAME | RDW_NOCHILDREN);
    mark_frame_partial(env, hwnd);
    erase_now(env, hwnd);
}

/// The frame pending for a part of the window only (WM_NCPAINT with a
/// region rather than 1).
fn mark_frame_partial<E: Env + ?Sized>(env: &E, hwnd: u32) {
    // RDW_FRAME marks the whole frame; a resize repaints a part of it.
    env.call(Request::Painted { hwnd, what: PAINTED_FRAME_PARTIAL, ferase: 0 });
}
