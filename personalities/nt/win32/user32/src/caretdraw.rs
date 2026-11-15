//! The blinking caret on the interactive desktop (see `caret.rs` for the calls).

use super::H;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32k::caret as c;

/// Windows' default blink time on the oracle (GetCaretBlinkTime.yaml): 500 ms.
const DEFAULT_BLINK_MS: u32 = 500;
static BLINK_MS: AtomicU32 = AtomicU32::new(DEFAULT_BLINK_MS);

pub fn blink_time() -> u32 {
    BLINK_MS.load(Ordering::Relaxed)
}

pub fn set_blink_time(ms: u32) {
    BLINK_MS.store(ms, Ordering::Relaxed);
}

pub fn created(_hwnd: H) {}

pub fn destroyed() {}

pub fn before_change() {}

pub fn after_change(_r: Option<c::Snapshot>) {}
