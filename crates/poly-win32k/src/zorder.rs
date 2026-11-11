//! The z order of top-level windows and who takes the activation (ADR 0176).
//!
//! One model for the three activation sheets (GetActiveWindow.yaml
//! `z_order_moves`, `next_active`, `minimize_inactive`; GetFocus.yaml
//! `ime_window`; GetForegroundWindow.yaml `handover_same_thread`,
//! `nobody_takes_over`, `minimize_foreground`, `foreground_right`). It works
//! on a snapshot of the desktop's top-level windows from the top down and
//! touches no window manager state, so every rule is tested on the host;
//! [`crate::wm`] keeps the list and applies what the functions here answer.
//!
//! **Bands.** The `WS_EX_TOPMOST` windows lead the list; a window moves
//! inside its band only. A new window and a raised one go to the top of
//! their band, a minimized one to the bottom.
//!
//! **Helpers.** Each thread has a hidden default IME window in the list.
//! The thread's active window owns it, and a window that is raised takes
//! the windows it owns along above it: that is why the first activation of
//! a thread's first window is a move (the helper was made below it) and why
//! a window that passes another window's helper has moved.
//!
//! **Other processes.** Windows of other processes are in the list too (in
//! the oracle's session the runner's keeper, a hidden window of its
//! process, and a console window). A thread that has lost the right to
//! take the foreground cannot put a window that is not topmost above the
//! foreground process's windows: its top is below the *barrier*.
//!
//! **Who takes the activation** when a window goes is decided by the
//! thread's activation history, not by the z order ([`successor`]).

use alloc::vec::Vec;

/// A thread: (process id, thread id), or the input queue it shares.
pub type Key = (u32, u32);

/// `WS_EX_TOPMOST`.
pub const TOPMOST: u8 = 1;
/// `WS_VISIBLE`.
pub const VISIBLE: u8 = 2;
/// `WS_DISABLED`.
pub const DISABLED: u8 = 4;
/// Minimized (`WS_MINIMIZE`).
pub const MINIMIZED: u8 = 8;
/// `WS_EX_NOACTIVATE`.
pub const NOACTIVATE: u8 = 16;
/// `WS_EX_TOOLWINDOW`.
pub const TOOL: u8 = 32;
/// A thread's default IME window.
pub const HELPER: u8 = 64;

/// One top-level window of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Top {
    pub hwnd: u32,
    /// The input queue of its thread.
    pub queue: Key,
    /// [`TOPMOST`] and the other bits.
    pub flags: u8,
    pub owner: u32,
}

impl Top {
    pub fn is(&self, bit: u8) -> bool {
        self.flags & bit != 0
    }

    /// Whether the window can be given the activation by its thread:
    /// visible, enabled, not minimized, not `WS_EX_NOACTIVATE`
    /// (GetForegroundWindow.yaml `handover_same_thread`, `nobody_takes_over`).
    pub fn can_take(&self) -> bool {
        self.is(VISIBLE) && !self.is(DISABLED | MINIMIZED | NOACTIVATE | HELPER)
    }

    /// Whether a minimize can hand the activation to the window without
    /// its having been active before: also not a tool window and not
    /// topmost (GetActiveWindow.yaml `next_active`, rule 2).
    pub fn plain_candidate(&self) -> bool {
        self.can_take() && !self.is(TOOL | TOPMOST)
    }
}

fn find(list: &[Top], h: u32) -> Option<&Top> {
    list.iter().find(|t| t.hwnd == h)
}

/// The handles of a list, in its order.
pub fn handles(list: &[Top]) -> Vec<u32> {
    list.iter().map(|t| t.hwnd).collect()
}

/// The index where the band of the topmost (`true`) or the other windows
/// starts in `order` (a list without the window being placed).
fn band_start(list: &[Top], order: &[u32], topmost: bool) -> usize {
    if topmost {
        return 0;
    }
    order.iter().take_while(|&&h| find(list, h).is_some_and(|t| t.is(TOPMOST))).count()
}

/// The index where the band of the topmost windows (or of the others) ends.
fn band_end(list: &[Top], order: &[u32], topmost: bool) -> usize {
    if topmost { band_start(list, order, false) } else { order.len() }
}

/// The top of a band for a window that may not pass `barrier` (a window of
/// the foreground process in the band of the windows that are not topmost).
fn top_slot(list: &[Top], order: &[u32], topmost: bool, barrier: Option<u32>) -> usize {
    let start = band_start(list, order, topmost);
    if topmost {
        return start;
    }
    match barrier.and_then(|b| order.iter().position(|&h| h == b)) {
        Some(i) if i >= start => i + 1,
        _ => start,
    }
}

/// `hwnd` raised to the top of its band, the windows it owns above it in
/// the order they had. The new order; the window moved when it differs
/// from the old one.
pub fn raise(list: &[Top], hwnd: u32, barrier: Option<u32>) -> Vec<u32> {
    let Some(w) = find(list, hwnd) else { return handles(list) };
    let owned: Vec<u32> = list.iter().filter(|t| t.owner == hwnd && t.hwnd != hwnd).map(|t| t.hwnd).collect();
    let mut order: Vec<u32> = list.iter().map(|t| t.hwnd).filter(|h| *h != hwnd && !owned.contains(h)).collect();
    let at = top_slot(list, &order, w.is(TOPMOST), barrier);
    order.insert(at, hwnd);
    for (i, &o) in owned.iter().enumerate() {
        order.insert(at + i, o);
    }
    order
}

/// `hwnd` put at the bottom of its band (a minimize, `HWND_BOTTOM`). Its
/// thread's helper, if the window owns it, goes down with it and stays
/// directly above it: the window that takes the activation from a
/// minimized window does not pass the helper (gaw_successor, scenario 4:
/// no WM_WINDOWPOSCHANGED for the topmost window that takes over).
pub fn sink(list: &[Top], hwnd: u32) -> Vec<u32> {
    let Some(w) = find(list, hwnd) else { return handles(list) };
    let helpers: Vec<u32> = list.iter().filter(|t| t.owner == hwnd && t.is(HELPER)).map(|t| t.hwnd).collect();
    let mut order: Vec<u32> = list.iter().map(|t| t.hwnd).filter(|h| *h != hwnd && !helpers.contains(h)).collect();
    let at = band_end(list, &order, w.is(TOPMOST));
    order.insert(at, hwnd);
    for (i, &o) in helpers.iter().enumerate() {
        order.insert(at + i, o);
    }
    order
}

/// Where a new window goes: the top of its band; a helper directly below
/// the window that owns it (GetFocus.yaml `ime_window`: "at first lies
/// BELOW that window"). The index in `list`, which does not have it yet.
pub fn new_slot(list: &[Top], new: &Top, barrier: Option<u32>) -> usize {
    let order = handles(list);
    if new.is(HELPER)
        && let Some(i) = order.iter().position(|&h| h == new.owner)
    {
        return i + 1;
    }
    top_slot(list, &order, new.is(TOPMOST), barrier)
}

/// The helper `helper` after the focus changed in its thread: on top of
/// the band of the active window `owner` (which owns it from now on, and
/// whose band it is in). `list` has the helper with the owner's
/// [`TOPMOST`] bit already.
pub fn helper_to_top(list: &[Top], helper: u32, barrier: Option<u32>) -> Vec<u32> {
    let Some(w) = find(list, helper) else { return handles(list) };
    let mut order: Vec<u32> = list.iter().map(|t| t.hwnd).filter(|h| *h != helper).collect();
    let at = top_slot(list, &order, w.is(TOPMOST), barrier);
    order.insert(at, helper);
    order
}

/// The helper after the activation moved to `owner` without a focus
/// change (a window activated minimized): it stays where it is unless it
/// lies below its new owner or outside the owner's band; then it goes
/// directly above the owner.
pub fn helper_above_owner(list: &[Top], helper: u32, owner: u32) -> Vec<u32> {
    let order = handles(list);
    let (Some(hi), Some(oi)) =
        (order.iter().position(|&h| h == helper), order.iter().position(|&h| h == owner))
    else {
        return order;
    };
    let topmost = find(list, owner).is_some_and(|t| t.is(TOPMOST));
    let start = band_start(list, &order, topmost);
    let end = band_end(list, &order, topmost);
    if hi < oi && hi >= start && hi < end {
        return order;
    }
    let mut order: Vec<u32> = order.into_iter().filter(|&h| h != helper).collect();
    let oi = order.iter().position(|&h| h == owner).unwrap_or(0);
    order.insert(oi, helper);
    order
}

/// Why a window gives the activation up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// `ShowWindow(SW_HIDE)` (or a hide by `SetWindowPos`).
    Hide,
    /// `DestroyWindow`.
    Destroy,
    /// `ShowWindow(SW_MINIMIZE)`, of the active window or of another one.
    Minimize,
}

/// What happens to the activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing changes.
    Stay,
    /// This window of the thread becomes its active window.
    Take(u32),
    /// The thread has no active window any more. `Some`: that window (of
    /// another thread or process) becomes the foreground window; `None`:
    /// the foreground returns to where the thread took it from.
    Away(Option<u32>),
}

/// The state [`successor`] decides on.
#[derive(Clone, Copy, Debug)]
pub struct Scene<'a> {
    /// The top-level windows, from the top of the z order down (the window
    /// that goes is still in it).
    pub list: &'a [Top],
    /// The windows that were the active window of the thread's queue, the
    /// latest first; ones destroyed since are still named.
    pub history: &'a [u32],
    /// The thread's input queue.
    pub queue: Key,
    /// The thread's active window (0: none).
    pub active: u32,
    /// The foreground window.
    pub foreground: u32,
}

/// Who takes the activation when window `going` of the thread is hidden,
/// destroyed or minimized (GetActiveWindow.yaml `next_active`,
/// `minimize_inactive`; GetForegroundWindow.yaml `handover_same_thread`,
/// `nobody_takes_over`, `minimize_foreground`).
///
/// A hide or destroy of a window that is not the active one changes
/// nothing. An owned window hands the activation to its owner. Else:
///
/// 1. The window that was active before the thread's active window, if it
///    can take the activation (and is not the window that goes).
/// 2. A hide or a destroy: if the thread had an earlier active window that
///    cannot take it, the top window of the thread that can; if the active
///    window was the thread's first, nobody (never-active windows are left
///    alone) and the foreground goes back where it came from.
/// 3. A minimize: the top window that is not topmost and can take it
///    without having been active, of any process. The thread's own active
///    window or the foreground window: nothing changes. A window of the
///    thread: it takes the activation. Another one: the foreground goes to
///    it.
pub fn successor(s: &Scene<'_>, going: u32, cause: Cause) -> Next {
    let Some(g) = find(s.list, going) else { return Next::Stay };
    let is_active = going == s.active;
    if !is_active && cause != Cause::Minimize {
        return Next::Stay;
    }
    let mine = |t: &Top| t.queue == s.queue && t.hwnd != going;
    if is_active
        && g.owner != 0
        && let Some(o) = find(s.list, g.owner)
        && mine(o)
        && o.can_take()
    {
        return Next::Take(o.hwnd);
    }
    // The windows that were active before the active one.
    let earlier: Vec<u32> = s.history.iter().copied().filter(|&h| h != s.active).collect();
    let previous = earlier.iter().filter_map(|&h| find(s.list, h)).find(|t| t.queue == s.queue);
    if let Some(p) = previous
        && p.hwnd != going
        && p.can_take()
    {
        return Next::Take(p.hwnd);
    }
    if cause != Cause::Minimize {
        if earlier.is_empty() {
            return Next::Away(None);
        }
        return match s.list.iter().find(|t| mine(t) && t.can_take()) {
            Some(t) => Next::Take(t.hwnd),
            None => Next::Away(None),
        };
    }
    match s.list.iter().find(|t| t.hwnd != going && t.plain_candidate()) {
        Some(t) if t.hwnd == s.active || t.hwnd == s.foreground => Next::Stay,
        Some(t) if t.queue == s.queue => Next::Take(t.hwnd),
        Some(t) => Next::Away(Some(t.hwnd)),
        None if is_active => Next::Away(None),
        None => Next::Stay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const ME: Key = (7, 7);
    const KEEPER: Key = (0xFFFF_FFF0, 0xFFFF_FFF0);
    const CONSOLE: Key = (0xFFFF_FFF1, 0xFFFF_FFF1);
    // Handles: letters for the test's windows, as the sheets name them.
    const A: u32 = 0xA;
    const B: u32 = 0xB;
    const C: u32 = 0xC;
    const D: u32 = 0xD;
    const E: u32 = 0xE;
    const X: u32 = 0x58;
    /// The keeper, the hidden window of its process, the console window,
    /// the thread's IME window.
    const K: u32 = 0x100;
    const KH: u32 = 0x101;
    const O: u32 = 0x102;
    const I: u32 = 0x103;

    fn win(hwnd: u32, flags: u8) -> Top {
        Top { hwnd, queue: ME, flags, owner: 0 }
    }

    fn plain(hwnd: u32) -> Top {
        win(hwnd, VISIBLE)
    }

    fn keeper() -> Top {
        Top { hwnd: K, queue: KEEPER, flags: VISIBLE | TOPMOST, owner: 0 }
    }

    fn console() -> Top {
        Top { hwnd: O, queue: CONSOLE, flags: VISIBLE, owner: 0 }
    }

    fn helper(owner: u32, flags: u8) -> Top {
        Top { hwnd: I, queue: ME, flags: HELPER | DISABLED | flags, owner }
    }

    fn scene<'a>(list: &'a [Top], history: &'a [u32], active: u32, foreground: u32) -> Scene<'a> {
        Scene { list, history, queue: ME, active, foreground }
    }

    #[test]
    fn a_raise_stays_in_its_band_and_takes_owned_windows_along() {
        // z: C^ K^ | B A
        let list = [win(C, VISIBLE | TOPMOST), keeper(), plain(B), plain(A)];
        assert_eq!(raise(&list, A, None), vec![C, K, A, B]);
        assert_eq!(raise(&list, B, None), handles(&list), "already the top of its band");
        assert_eq!(raise(&list, K, None), vec![K, C, B, A], "the keeper took the foreground");
        // An owned popup and the helper follow their owner, in their order.
        let mut owned = plain(D);
        owned.owner = A;
        let list = [plain(B), owned, helper(A, 0), plain(A)];
        assert_eq!(raise(&list, A, None), vec![D, I, A, B]);
    }

    #[test]
    fn the_first_activation_of_a_threads_first_window_is_a_move() {
        // GetFocus.yaml ime_window (a): the helper is made below the first
        // window, which owns it; raising the window puts the helper above it.
        let first = plain(A);
        let h = helper(A, 0);
        let mut list = vec![console()];
        list.insert(new_slot(&list, &first, None), first);
        list.insert(new_slot(&list, &h, None), h);
        assert_eq!(handles(&list), vec![A, I, O]);
        assert_eq!(raise(&list, A, None), vec![I, A, O]);
        // A later window that is the first to be activated while an earlier,
        // never active window exists does not move (0x10001847).
        let list = [win(B, 0), win(A, 0), helper(A, 0), console()];
        assert_eq!(raise(&list, B, None), handles(&list));
    }

    #[test]
    fn a_window_passes_the_helper_unless_it_owns_it() {
        // (c): N was created before the focus changed: the helper is above it.
        let list = [helper(A, 0), win(0x4E, 0), plain(A)];
        assert_eq!(raise(&list, 0x4E, None), vec![0x4E, I, A]);
        // (b): a window created after the last focus change lies above it.
        let list = [win(0x4E, 0), helper(A, 0), plain(A)];
        assert_eq!(raise(&list, 0x4E, None), handles(&list));
        // (d): the active window is not below its own helper.
        let list = [helper(A, 0), plain(A)];
        assert_eq!(raise(&list, A, None), handles(&list));
    }

    #[test]
    fn the_helper_goes_on_top_of_the_active_windows_band() {
        // SetFocus to a child of A: z was N~ i A.
        let list = [win(0x4E, 0), helper(A, 0), plain(A)];
        assert_eq!(helper_to_top(&list, I, None), vec![I, 0x4E, A]);
        // A topmost active window: the helper is in the topmost band.
        let list = [win(C, TOPMOST), win(A, VISIBLE | TOPMOST), keeper(), helper(A, TOPMOST), plain(B)];
        assert_eq!(helper_to_top(&list, I, None), vec![I, C, A, K, B]);
        // Activated without a focus change: only moved when it has to.
        let list = [plain(B), helper(B, 0), plain(A)];
        assert_eq!(helper_above_owner(&list, I, B), vec![I, B, A]);
        let list = [helper(B, 0), plain(A), plain(B)];
        assert_eq!(helper_above_owner(&list, I, B), handles(&list));
        let list = [win(B, VISIBLE | TOPMOST), keeper(), helper(B, TOPMOST), plain(A)];
        assert_eq!(helper_above_owner(&list, I, B), vec![I, B, K, A]);
    }

    #[test]
    fn a_minimize_puts_the_window_at_the_bottom_of_its_band() {
        // gaw_never_active scenario 4: z C^ A^ K^ B, A minimized: C^ K^ A^_ B.
        let list = [win(C, VISIBLE | TOPMOST), win(A, VISIBLE | TOPMOST), keeper(), plain(B)];
        assert_eq!(sink(&list, A), vec![C, K, A, B]);
        // A window that is not topmost goes below the other processes' windows.
        let list = [plain(A), plain(B), console()];
        assert_eq!(sink(&list, A), vec![B, O, A]);
        assert_eq!(sink(&list, O), handles(&list), "already the bottom: no move");
        // The helper of a minimized active window goes down with it.
        let list = [helper(A, TOPMOST), win(A, VISIBLE | TOPMOST), win(B, VISIBLE | TOPMOST), keeper(), console()];
        let order = sink(&list, A);
        assert_eq!(order, vec![B, K, I, A, O]);
    }

    #[test]
    fn a_thread_without_the_foreground_right_stays_below_the_foreground_process() {
        // The keeper has the foreground; its process's hidden window is the
        // barrier of the band of the windows that are not topmost.
        let list = [keeper(), win(KH, 0), helper(A, 0), plain(A), console()];
        let b = plain(B);
        assert_eq!(new_slot(&list, &b, Some(KH)), 2);
        assert_eq!(new_slot(&list, &b, None), 1);
        let list = [keeper(), win(KH, 0), plain(B), helper(A, 0), plain(A)];
        assert_eq!(raise(&list, B, Some(KH)), handles(&list), "it cannot pass: no move");
        assert_eq!(raise(&list, A, Some(KH)), vec![K, KH, I, A, B], "inside its own windows it moves");
        assert_eq!(raise(&list, B, None), vec![K, B, KH, I, A], "with the right it passes");
        // A topmost window is not held back.
        let t = win(C, TOPMOST);
        assert_eq!(new_slot(&list, &t, Some(KH)), 0);
    }

    #[test]
    fn the_window_active_before_takes_it_whatever_the_z_order() {
        // gaw_successor: B, C, A were active in this order; B was raised above A.
        let list = [keeper(), plain(B), plain(A), plain(C), console()];
        let hist = [A, C, B];
        for cause in [Cause::Hide, Cause::Destroy, Cause::Minimize] {
            assert_eq!(successor(&scene(&list, &hist, A, A), A, cause), Next::Take(C), "{cause:?}");
        }
        // The chain: A destroyed, then C.
        let list = [keeper(), plain(C), plain(B), console()];
        assert_eq!(successor(&scene(&list, &[C, A, B], C, C), C, Cause::Destroy), Next::Take(B));
        // A topmost window that was active before takes a minimize too.
        let list = [win(A, VISIBLE | TOPMOST), win(B, VISIBLE | TOPMOST), keeper(), console()];
        assert_eq!(successor(&scene(&list, &[A, B], A, A), A, Cause::Minimize), Next::Take(B));
    }

    #[test]
    fn never_active_windows_are_left_alone_by_a_hide_or_destroy() {
        // gaw_never_active: B and C shown with SW_SHOWNA above A.
        let list = [keeper(), plain(C), plain(B), plain(A), console()];
        let s = scene(&list, &[A], A, A);
        assert_eq!(successor(&s, A, Cause::Hide), Next::Away(None));
        assert_eq!(successor(&s, A, Cause::Destroy), Next::Away(None));
        // A minimize takes the upper one.
        assert_eq!(successor(&s, A, Cause::Minimize), Next::Take(C));
        // An inactive window hidden or destroyed changes nothing.
        assert_eq!(successor(&s, B, Cause::Hide), Next::Stay);
        assert_eq!(successor(&s, B, Cause::Destroy), Next::Stay);
    }

    #[test]
    fn an_earlier_active_window_that_cannot_take_it_gives_way_to_the_top_one() {
        // GetForegroundWindow.yaml handover_same_thread: E disabled, A
        // topmost and never active, B plain: A.
        let list =
            [win(A, VISIBLE | TOPMOST), keeper(), plain(X), plain(B), win(E, VISIBLE | DISABLED), console()];
        let s = scene(&list, &[X, E], X, X);
        assert_eq!(successor(&s, X, Cause::Hide), Next::Take(A));
        // The earlier one is gone and nobody else can: nobody.
        let list = [keeper(), plain(X), win(B, VISIBLE | NOACTIVATE), console()];
        assert_eq!(successor(&scene(&list, &[X, E], X, X), X, Cause::Destroy), Next::Away(None));
    }

    #[test]
    fn a_minimize_looks_for_the_top_plain_window_of_any_process() {
        // gaw_never_active scenario 4: a plain B and a topmost C above a topmost A: B.
        let list = [win(C, VISIBLE | TOPMOST), win(A, VISIBLE | TOPMOST), keeper(), plain(B), console()];
        assert_eq!(successor(&scene(&list, &[A], A, A), A, Cause::Minimize), Next::Take(B));
        // Only a topmost C: the console window of another process (never the keeper).
        let list = [win(C, VISIBLE | TOPMOST), win(A, VISIBLE | TOPMOST), keeper(), console()];
        assert_eq!(successor(&scene(&list, &[A], A, A), A, Cause::Minimize), Next::Away(Some(O)));
        // Scenario 5: no-activate, disabled, tool and minimized windows do not qualify.
        let list = [
            keeper(),
            win(D, VISIBLE | TOOL),
            win(C, VISIBLE | DISABLED),
            win(B, VISIBLE | NOACTIVATE),
            plain(A),
            win(E, VISIBLE | MINIMIZED),
            console(),
        ];
        assert_eq!(successor(&scene(&list, &[A], A, A), A, Cause::Minimize), Next::Away(Some(O)));
    }

    #[test]
    fn minimizing_an_inactive_window_hands_on_as_if_it_were_the_active_one() {
        // gaw_min_inactive 1: E was active before A; X is minimized: E.
        let list = [keeper(), plain(X), plain(A), plain(E), console()];
        assert_eq!(successor(&scene(&list, &[A, E], A, A), X, Cause::Minimize), Next::Take(E));
        // 2 and 5: no window was active before A: A is the top plain one, nothing happens.
        let list = [keeper(), plain(A), plain(X), console()];
        assert_eq!(successor(&scene(&list, &[A], A, A), X, Cause::Minimize), Next::Stay);
        // 3: X itself was active before A: nothing either.
        let list = [keeper(), plain(A), plain(X), plain(E), console()];
        assert_eq!(successor(&scene(&list, &[A, X, E], A, A), X, Cause::Minimize), Next::Stay);
        // 4: E minimized, X was active before A: X.
        assert_eq!(successor(&scene(&list, &[A, X, E], A, A), E, Cause::Minimize), Next::Take(X));
        // 7: A topmost and no plain window of the thread: the console window.
        let list = [win(X, VISIBLE | TOPMOST), win(A, VISIBLE | TOPMOST), keeper(), console()];
        assert_eq!(successor(&scene(&list, &[A], A, A), X, Cause::Minimize), Next::Away(Some(O)));
        // 6: a lone inactive topmost window while the keeper is in the foreground.
        let list = [win(B, VISIBLE | TOPMOST), keeper(), console()];
        assert_eq!(successor(&scene(&list, &[], 0, K), B, Cause::Minimize), Next::Away(Some(O)));
        // The window found already is the foreground window: nothing changes
        // (a thread in the background minimizes its active window).
        let list = [keeper(), console(), plain(A)];
        assert_eq!(successor(&scene(&list, &[A], A, O), A, Cause::Minimize), Next::Stay);
    }

    #[test]
    fn an_owned_window_hands_the_activation_to_its_owner() {
        let mut o = plain(D);
        o.owner = A;
        let list = [keeper(), o, plain(A), plain(B), console()];
        assert_eq!(successor(&scene(&list, &[D, B, A], D, D), D, Cause::Destroy), Next::Take(A));
    }
}
