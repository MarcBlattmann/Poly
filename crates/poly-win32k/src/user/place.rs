//! Window placement: `GetWindowPlacement`, `SetWindowPlacement` and the
//! rectangle `MonitorFromWindow` judges a window by (MonitorFromWindow.yaml
//! `state`, `state_rects`; Learn: WINDOWPLACEMENT).
//!
//! A window keeps its restored ("normal") rectangle while it is minimized
//! or maximized. For a top-level window that is not a tool window the
//! placement speaks workspace coordinates: relative to the work area's
//! top-left corner (Learn; observed: a window at 100,100,340,220 reports
//! 100,76,340,196 over the work area 0,24,1920,1080).

use super::*;

/// `WINDOWPLACEMENT` (winuser.h): 44 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowPlacement {
    pub length: u32,
    pub flags: u32,
    pub show_cmd: u32,
    pub min_position: CPoint,
    pub max_position: CPoint,
    pub normal_position: CRect,
}

/// `sizeof(WINDOWPLACEMENT)`.
pub const WINDOWPLACEMENT_SIZE: u32 = 44;
/// `WPF_SETMINPOSITION` (winuser.h): `ptMinPosition` is valid.
pub const WPF_SETMINPOSITION: u32 = 0x0001;
/// `WPF_RESTORETOMAXIMIZED` (winuser.h).
pub const WPF_RESTORETOMAXIMIZED: u32 = 0x0002;
/// winerror.h.
const ERROR_INVALID_PARAMETER: u32 = 87;

/// What workspace coordinates of window `i` count from.
pub(super) fn workspace_origin<E: Env + ?Sized>(env: &E, i: &WinInfo) -> (i32, i32) {
    if i.style & WS_CHILD == 0 && i.ex_style & WS_EX_TOOLWINDOW == 0 {
        let d = screen(env);
        (d.work.left, d.work.top)
    } else {
        (0, 0)
    }
}

/// The rectangle window `i` has when it is neither minimized nor maximized
/// (relative to its parent; the screen for a top-level window).
pub fn restored_rect<E: Env + ?Sized>(env: &E, i: &WinInfo) -> Rect {
    if i.minimized || i.maximized { normal_rect(env, i) } else { i.window_rel }
}

/// The rectangle (screen coordinates) `MonitorFromWindow` judges window `i`
/// by: its current one, but a minimized window's restored one, not the
/// small rectangle it has while minimized (MonitorFromWindow.yaml `state`).
pub fn monitor_rect<E: Env + ?Sized>(env: &E, i: &WinInfo) -> Rect {
    if !i.minimized {
        return i.window;
    }
    let n = normal_rect(env, i);
    // Parent coordinates: the screen's for a top-level window.
    let (dx, dy) = (i.window.left - i.window_rel.left, i.window.top - i.window_rel.top);
    Rect::new(n.left + dx, n.top + dy, n.right + dx, n.bottom + dy)
}

/// `GetWindowPlacement`: the show state (SW_SHOWMINIMIZED 2,
/// SW_SHOWMAXIMIZED 3, else SW_SHOWNORMAL 1), the restored rectangle, and
/// WPF_RESTORETOMAXIMIZED for a maximized window and for a minimized one
/// that goes back to maximized.
pub fn get_window_placement<E: Env + ?Sized>(env: &E, hwnd: u64, out: &mut WindowPlacement) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    let (ox, oy) = workspace_origin(env, &i);
    let r = restored_rect(env, &i);
    *out = WindowPlacement {
        length: WINDOWPLACEMENT_SIZE,
        flags: if (i.maximized && !i.minimized) || (i.minimized && i.restore_max) { WPF_RESTORETOMAXIMIZED } else { 0 },
        show_cmd: if i.minimized {
            SW_SHOWMINIMIZED as u32
        } else if i.maximized {
            SW_SHOWMAXIMIZED as u32
        } else {
            SW_SHOWNORMAL as u32
        },
        min_position: CPoint { x: i.min_x, y: i.min_y },
        max_position: CPoint { x: -1, y: -1 },
        normal_position: CRect::from(Rect::new(r.left - ox, r.top - oy, r.right - ox, r.bottom - oy)),
    };
    true
}

/// A restored rectangle in SCREEN coordinates that lies wholly outside the
/// screen (not merely touching it) is moved onto the screen by the shortest
/// way, keeping its size: wholly left to x = 0, wholly right to the right
/// edge, wholly below to the bottom edge, wholly above to y = 0, where it is
/// also cut at the work area's top (observed with the work area 0,24,1920,1080:
/// the workspace rectangle 200,-700,440,-560 becomes the window
/// 200,24,440,140, reported as 200,0,440,116; a rectangle that touches the
/// screen from outside or overlaps it stays).
fn onto_screen(r: Rect, width: i32, height: i32, work_top: i32) -> Rect {
    let (mut dx, mut dy) = (0, 0);
    if r.right < 0 {
        dx = -r.left;
    } else if r.left > width {
        dx = width - r.right;
    }
    let mut top_cut = None;
    if r.bottom < 0 {
        dy = -r.top;
        top_cut = Some(work_top);
    } else if r.top > height {
        dy = height - r.bottom;
    }
    let mut m = Rect::new(r.left + dx, r.top + dy, r.right + dx, r.bottom + dy);
    if let Some(t) = top_cut {
        // Cut, not moved: the bottom stays where the move put it.
        m.top = m.top.max(t).min(m.bottom);
    }
    m
}

/// `SetWindowPlacement` (Learn): the restored rectangle and the show state.
pub fn set_window_placement<E: Env + ?Sized>(env: &E, hwnd: u64, wp: &WindowPlacement) -> bool {
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    if wp.length != WINDOWPLACEMENT_SIZE {
        env.set_last_error(ERROR_INVALID_PARAMETER);
        return false;
    }
    let (ox, oy) = workspace_origin(env, &i);
    let r = Rect::from(wp.normal_position);
    // The rectangle in screen coordinates (a tool window's and a child's are
    // given in them already: nothing is adjusted for those).
    let mut normal = Rect::new(r.left + ox, r.top + oy, r.right + ox, r.bottom + oy);
    if i.style & WS_CHILD == 0 && i.ex_style & WS_EX_TOOLWINDOW == 0 {
        let d = screen(env);
        normal = onto_screen(normal, d.width, d.height, d.work.top);
    }
    let keep = |env: &E| {
        env.call(Request::SetState {
            hwnd: i.hwnd,
            visible: None,
            minimized: None,
            maximized: None,
            normal: Some(normal),
            size_move_pending: None,
        });
    };
    let cmd = wp.show_cmd as i32;
    let minimizes = matches!(cmd, SW_SHOWMINIMIZED | SW_MINIMIZE | SW_SHOWMINNOACTIVE);
    match cmd {
        SW_SHOWMINIMIZED | SW_MINIMIZE | SW_SHOWMINNOACTIVE | SW_SHOWMAXIMIZED => {
            // The state first (it keeps the rectangle the window has), then
            // the rectangle to restore to.
            show_window(env, hwnd, cmd);
            keep(env);
        }
        _ => {
            if i.minimized || i.maximized {
                keep(env);
            } else {
                set_window_pos(
                    env,
                    hwnd,
                    0,
                    normal.left,
                    normal.top,
                    normal.width(),
                    normal.height(),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            show_window(env, hwnd, cmd);
        }
    }
    // ptMinPosition (MonitorFromWindow.yaml `placement`): a window that is
    // minimized by the call gets the place it got (stored by the minimize
    // itself); one that was minimized before keeps its value; one that was
    // not stores the one it was given. WPF_SETMINPOSITION puts a window that
    // is minimized now at the point given (workspace coordinates).
    let set_pos = wp.flags & WPF_SETMINPOSITION != 0;
    if set_pos || (!i.minimized && !minimizes) {
        env.call(Request::SetMinPos { hwnd: i.hwnd, x: wp.min_position.x, y: wp.min_position.y });
    }
    if set_pos
        && let Some(now) = info(env, hwnd)
        && now.minimized
    {
        let (x, y) = (wp.min_position.x + ox, wp.min_position.y + oy);
        set_window_pos(env, hwnd, 0, x, y, 0, 0, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_wholly_off_the_screen_is_moved_onto_it() {
        let (w, h, top) = (1920, 1080, 24);
        // Workspace 200,-700,440,-560 is 200,-676,440,-536 on the screen.
        assert_eq!(onto_screen(Rect::new(200, -676, 440, -536), w, h, top), Rect::new(200, 24, 440, 140));
        assert_eq!(onto_screen(Rect::new(-3000, 124, -2800, 224), w, h, top), Rect::new(0, 124, 200, 224));
        assert_eq!(onto_screen(Rect::new(300, 3024, 500, 3124), w, h, top), Rect::new(300, 980, 500, 1080));
        assert_eq!(onto_screen(Rect::new(2500, 174, 2740, 314), w, h, top), Rect::new(1680, 174, 1920, 314));
        // Partly on it (also above the work area's top), or touching it from
        // outside: left alone.
        assert_eq!(onto_screen(Rect::new(-120, -36, 120, 104), w, h, top), Rect::new(-120, -36, 120, 104));
        assert_eq!(onto_screen(Rect::new(-240, 174, 0, 314), w, h, top), Rect::new(-240, 174, 0, 314));
        assert_eq!(onto_screen(Rect::new(300, 324, 500, 424), w, h, top), Rect::new(300, 324, 500, 424));
    }
}
