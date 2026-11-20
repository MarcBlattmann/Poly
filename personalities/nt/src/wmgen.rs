//! The window manager behind a generation (ADR 0249).
//!
//! `user32` keeps what `Window` answered and uses it again while the window-state page's
//! generation has not moved. The generation has to move on every change of the window manager,
//! whoever makes it: a request, the compositor's input, a thread or process that ends. So the
//! window manager is only reachable through [`Tracked`]: reading through `Deref`, changing through
//! [`Tracked::mutate`], which calls the kernel's hook (it raises the generation) before it hands
//! out `&mut Wm`. A change is thus never visible before the generation that announces it.

use poly_win32k::wm::Wm;

/// The window manager and the hook that announces its changes.
pub struct Tracked {
    wm: Wm,
    changed: fn(),
}

fn nothing() {}

impl Tracked {
    pub fn new(wm: Wm) -> Tracked {
        Tracked { wm, changed: nothing }
    }

    /// What runs before every change (the kernel raises the window-state generation).
    pub fn set_hook(&mut self, changed: fn()) {
        self.changed = changed;
    }

    /// The window manager, to change it: the hook runs first.
    pub fn mutate(&mut self) -> &mut Wm {
        (self.changed)();
        &mut self.wm
    }
}

impl Default for Tracked {
    fn default() -> Tracked {
        Tracked::new(Wm::default())
    }
}

impl core::ops::Deref for Tracked {
    type Target = Wm;

    fn deref(&self) -> &Wm {
        &self.wm
    }
}

impl core::fmt::Debug for Tracked {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tracked").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, Ordering};

    static BUMPS: AtomicU64 = AtomicU64::new(0);

    fn bump() {
        BUMPS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn a_change_raises_the_generation_first_and_a_read_does_not() {
        let mut t = Tracked::default();
        t.set_hook(bump);
        let before = BUMPS.load(Ordering::SeqCst);
        let _ = t.info(0x10010);
        let _ = t.is_window(0x10010);
        assert_eq!(BUMPS.load(Ordering::SeqCst), before, "reads");
        let _ = t.mutate();
        assert_eq!(BUMPS.load(Ordering::SeqCst), before + 1, "a change");
    }
}
