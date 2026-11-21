//! The window under a still pointer (`WM_MOUSEMOVE.yaml`, `WM_MOUSELEAVE.yaml`,
//! `WM_NCMOUSELEAVE.yaml`, `WM_MOUSEHOVER.yaml`, `WM_CAPTURECHANGED.yaml`,
//! `tooltips_class32.yaml`).
//!
//! Windows tells the window under the pointer about it again whenever the window
//! structure changed, although the pointer did not move: a window shown, hidden, moved,
//! resized, raised or destroyed, or the capture given to another window. The window that
//! is now under the pointer gets `WM_SETCURSOR` and a `WM_MOUSEMOVE` (keys and client
//! coordinates as for a real move), and tracked windows the pointer is no longer over get
//! their `WM_MOUSELEAVE` / `WM_NCMOUSELEAVE` after it. The oracle showed the move also when
//! the change was elsewhere on the screen (a tooltip shown and hidden away from the
//! pointer, a window shown behind the one under it), and none when a window was only
//! disabled, and none for a capture change alone (`SetCapture.yaml` `sc_basic`: `SetCapture(B)`
//! with the pointer elsewhere gives B no move). So Poly keeps a fingerprint of the window
//! structure the pointer's target follows (the visible windows' places, sizes, order and
//! click-through styles) and synthesizes one move when it changed. The move goes where the
//! pointer's messages go when it is made: to a window that took the capture in the meantime
//! (`WM_MOUSELEAVE.yaml` `capture`: B shown and given the capture gets the move).
//!
//! The check runs when a thread next looks at its queue (`Peek`, `QueueStatus`) after a
//! request that may change the window structure (`structural`): the steps of one
//! `SetWindowPos` or `ShowWindow` give one move, and calls made right after it (a
//! `SetCapture`) count, as on the oracle, whose move came within about 60 ms
//! (`WM_MOUSELEAVE.yaml` `capture`: `ShowWindow` and `SetCapture` of B give B the move, not
//! the window under the pointer). Only the interactive
//! session does this: the tests recorded in session 0 (no pointer on a screen) were
//! recorded without it.
//!
//! Hit testing passes through a top-level window with both `WS_EX_LAYERED` and
//! `WS_EX_TRANSPARENT` (the style alone, or a translucent layered window, takes the
//! pointer), and through a window that answered `WM_NCHITTEST` with `HTTRANSPARENT`
//! (`user32` reports that answer: `TRACK_TRANSPARENT`) until the pointer really moves.

use super::*;

/// Private `TrackMouse` flag bits (no `TME_*` value uses them; `user32`'s
/// `TrackMouseEvent` takes them out of a program's flags).
///
/// `DefWindowProc` handled `WM_NCMOUSEMOVE` of the window: its non-client area is
/// tracked until the pointer leaves it (`WM_NCMOUSELEAVE`, without a request of the program).
pub const TRACK_NC_IMPLICIT: u32 = 0x0100_0000;
/// The pointer moved into the client area of the window whose non-client area was tracked:
/// the tracking ends with `WM_NCMOUSELEAVE` (after the move's messages).
pub const TRACK_NC_END: u32 = 0x0200_0000;
/// The window answered `WM_NCHITTEST` with `HTTRANSPARENT` at the pointer: the pointer's
/// messages go to the window below it, and the move being retrieved is made again for that window.
pub const TRACK_TRANSPARENT: u32 = 0x0400_0000;
/// The bits above.
pub const TRACK_PRIVATE: u32 = TRACK_NC_IMPLICIT | TRACK_NC_END | TRACK_TRANSPARENT;

/// `TME_LEAVE`, `TME_NONCLIENT` (winuser.h).
pub(super) const TME_LEAVE: u32 = 0x2;
pub(super) const TME_NONCLIENT: u32 = 0x10;

/// `SPI_GETMOUSEHOVERWIDTH` / `HEIGHT` (`WM_MOUSEHOVER.yaml`: 4 and 4): a move that stays
/// within this rectangle around the place the hover started does not restart it.
pub(super) const HOVER_RECT: (i32, i32) = (4, 4);

/// What the window under the pointer depends on: the pointer's messages are synthesized again
/// when it changed.
#[derive(Debug, Default)]
pub(super) struct PointerState {
    /// A request that may have changed the window structure ran since the last check.
    pub pending: bool,
    /// The fingerprint at the last check.
    pub seen: u64,
    /// Windows that answered `HTTRANSPARENT` at the pointer's place, until it moves.
    pub transparent: Vec<u32>,
    /// The desktop's pointer left the Windows surfaces (`FromCompositor::Leave`): it is over
    /// none of the session's windows until it comes back.
    pub away: bool,
}

/// Whether `req` may change where the pointer's messages go.
pub(super) fn structural(req: &Request) -> bool {
    matches!(
        req,
        Request::SetRects { .. }
            | Request::FinishCreate { .. }
            | Request::Free { .. }
            | Request::SetLong { .. }
            | Request::SetStyle { .. }
            | Request::ZOrder { apply: true, .. }
            | Request::SetParent { .. }
            | Request::SetState { .. }
            | Request::Layered { .. }
            | Request::WindowRgn { .. }
    )
}

/// FNV-1a over 64-bit words.
fn mix(h: u64, v: u64) -> u64 {
    let mut h = h;
    for b in v.to_le_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

fn rect_word(r: Rect) -> [u64; 2] {
    [
        u64::from(r.left as u32) | u64::from(r.top as u32) << 32,
        u64::from(r.right as u32) | u64::from(r.bottom as u32) << 32,
    ]
}

impl Wm {
    /// A top-level window the pointer passes through: `WS_EX_LAYERED | WS_EX_TRANSPARENT`
    /// (`WM_MOUSELEAVE.yaml` `windows_that_do_not_cover`), or one that answered `HTTRANSPARENT`.
    pub(super) fn click_through(&self, h: u32, w: &Window) -> bool {
        // `WS_EX_TRANSPARENT` (winuser.h).
        const BOTH: u32 = guic::WS_EX_LAYERED | 0x20;
        w.ex_style & BOTH == BOTH || self.pointer.transparent.contains(&h)
    }

    /// The fingerprint of what decides where the pointer's messages go.
    fn pointer_fingerprint(&self) -> u64 {
        let mut h = 0xCBF2_9CE4_8422_2325u64;
        let mut stack: Vec<u32> = self.get(u64::from(self.desktop)).map(|d| d.children.clone()).unwrap_or_default();
        stack.reverse();
        while let Some(t) = stack.pop() {
            let Some(w) = self.get(u64::from(t)) else { continue };
            if w.style & WS_VISIBLE == 0 {
                continue;
            }
            h = mix(h, u64::from(t) | u64::from(w.minimized) << 32 | u64::from(self.click_through(t, w)) << 33);
            for v in rect_word(w.window_rel).into_iter().chain(rect_word(w.client_rel)) {
                h = mix(h, v);
            }
            h = mix(h, w.region.as_ref().map_or(0, |r| r.rects().len() as u64 + 1));
            // The children, the topmost first, after their parent.
            for &c in w.children.iter().rev() {
                stack.push(c);
            }
        }
        h
    }

    /// A mouse move was queued: what it went to is known to the windows (the next check
    /// compares with this).
    pub(super) fn pointer_moved(&mut self) {
        if self.display().interactive {
            self.pointer.seen = self.pointer_fingerprint();
        }
    }

    /// Run the check a structural request asked for: one synthesized move when the
    /// fingerprint changed (interactive session only).
    pub(super) fn pointer_recheck(&mut self, ctx: Ctx, fx: &mut Effects) {
        self.pointer.pending = false;
        if !self.display().interactive || self.pointer.away {
            return;
        }
        let f = self.pointer_fingerprint();
        if f == self.pointer.seen {
            return;
        }
        self.pointer.seen = f;
        self.queue_mouse(ctx, raw::WM_MOUSEMOVE, 0, fx);
    }

    /// A window answered `HTTRANSPARENT` at the pointer: it passes the pointer on until the
    /// pointer moves, its unretrieved move is dropped and the move is made again for the window
    /// that is under the pointer now. Leave messages the first move queued for windows the
    /// pointer is over after all are taken back with their tracking.
    pub(super) fn pointer_transparent(&mut self, ctx: Ctx, hwnd: u32, fx: &mut Effects) {
        if !self.pointer.transparent.contains(&hwnd) {
            self.pointer.transparent.push(hwnd);
        }
        let (x, y) = self.cursor;
        let target = self.mouse_target(x, y);
        // Take back the leave of a tracked window the pointer is over after all.
        let over = |wm: &Wm, h: u32| h == target && wm.in_client(h, (x, y));
        let keys: Vec<Key> = self.queues.keys().copied().collect();
        for k in keys {
            let back: Vec<u32> = self
                .queues
                .get(&k)
                .map(|q| q.raw.iter().filter(|m| m.msg == WM_MOUSELEAVE_MSG).map(|m| m.hwnd).collect())
                .unwrap_or_default();
            for h in back {
                if over(self, h) {
                    if let Some(q) = self.queues.get_mut(&k) {
                        q.raw.retain(|m| !(m.msg == WM_MOUSELEAVE_MSG && m.hwnd == h));
                    }
                    if !self.tracking.iter().any(|t| t.0 == h && t.1 & TME_NONCLIENT == 0) {
                        self.tracking.push((h, TME_LEAVE));
                    }
                }
            }
        }
        self.queue_mouse(ctx, raw::WM_MOUSEMOVE, 0, fx);
    }
}

/// `WM_MOUSELEAVE` (winuser.h).
pub(super) const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
