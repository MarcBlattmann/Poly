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
/// `WPF_RESTORETOMAXIMIZED` (winuser.h).
pub const WPF_RESTORETOMAXIMIZED: u32 = 0x0002;
/// winerror.h.
const ERROR_INVALID_PARAMETER: u32 = 87;

/// What workspace coordinates of window `i` count from.
fn workspace_origin<E: Env + ?Sized>(env: &E, i: &WinInfo) -> (i32, i32) {
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
        min_position: CPoint { x: -1, y: -1 },
        max_position: CPoint { x: -1, y: -1 },
        normal_position: CRect::from(Rect::new(r.left - ox, r.top - oy, r.right - ox, r.bottom - oy)),
    };
    true
}

/// A restored rectangle in workspace coordinates that lies wholly outside
/// the work area (not merely touching it) is moved onto it by the shortest
/// way (observed: -3000,100,-2800,200 becomes 0,100,200,200; 300,3000,500,3100
/// becomes 300,956,500,1056; -200,100,0,200 stays).
fn onto_work_area(r: Rect, width: i32, height: i32) -> Rect {
    let (mut dx, mut dy) = (0, 0);
    if r.right < 0 {
        dx = -r.left;
    } else if r.left > width {
        dx = width - r.right;
    }
    if r.bottom < 0 {
        dy = -r.top;
    } else if r.top > height {
        dy = height - r.bottom;
    }
    Rect::new(r.left + dx, r.top + dy, r.right + dx, r.bottom + dy)
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
    let mut r = Rect::from(wp.normal_position);
    if i.style & WS_CHILD == 0 {
        let d = screen(env);
        r = onto_work_area(r, d.work.width(), d.work.height());
    }
    let normal = Rect::new(r.left + ox, r.top + oy, r.right + ox, r.bottom + oy);
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
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_wholly_off_the_work_area_is_moved_onto_it() {
        let (w, h) = (1920, 1056);
        assert_eq!(onto_work_area(Rect::new(-3000, 100, -2800, 200), w, h), Rect::new(0, 100, 200, 200));
        assert_eq!(onto_work_area(Rect::new(300, 3000, 500, 3100), w, h), Rect::new(300, 956, 500, 1056));
        assert_eq!(onto_work_area(Rect::new(4000, 100, 4200, 200), w, h), Rect::new(1720, 100, 1920, 200));
        // Partly on it, or touching it from outside: left alone.
        assert_eq!(onto_work_area(Rect::new(-150, 100, 50, 200), w, h), Rect::new(-150, 100, 50, 200));
        assert_eq!(onto_work_area(Rect::new(-200, 100, 0, 200), w, h), Rect::new(-200, 100, 0, 200));
        assert_eq!(onto_work_area(Rect::new(300, 300, 500, 400), w, h), Rect::new(300, 300, 500, 400));
    }
}
