//! What `Window` answered, kept while the window state is unchanged (ADR 0249).
//!
//! `IsWindow`, `IsZoomed`, `GetClientRect`, `MonitorFromWindow` and many more each asked the
//! kernel for the window's whole record; Chrome's UI thread does so hundreds of times while
//! switching tabs. The kernel maps a window-state page into the process on the first ask
//! (`Request::SharedPage`), whose first eight bytes are a generation it raises before every change
//! of the window manager. An answer is kept with the generation read *before* it was asked for,
//! and used again while the page shows that generation: any change since makes it ask again.
//! Answers are kept per thread, since they come in the thread's DPI units. A slot another thread
//! is using is passed by: the call then just asks the kernel.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_win32k::proto::{Reply, Request};

/// The page's address: 0 not asked yet, 1 none (the kernel has no page), else the address.
static PAGE: AtomicU64 = AtomicU64::new(0);

const SLOTS: usize = 32;

struct Entry {
    /// The request: `Window` of this window, or `Desktop` (0).
    window: Option<u64>,
    tid: u32,
    generation: u64,
    reply: Reply,
}

struct Slot {
    busy: AtomicBool,
    entry: UnsafeCell<Option<Entry>>,
}

// SAFETY: `entry` is only touched by the thread that set `busy` (an acquire / release pair).
unsafe impl Sync for Slot {}

static CACHE: [Slot; SLOTS] = [const { Slot { busy: AtomicBool::new(false), entry: UnsafeCell::new(None) } }; SLOTS];

impl Slot {
    /// Run `f` on the entry unless another thread is in it.
    fn with<R>(&self, f: impl FnOnce(&mut Option<Entry>) -> R) -> Option<R> {
        if self.busy.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            return None;
        }
        // SAFETY: `busy` is ours until it is cleared below.
        let r = f(unsafe { &mut *self.entry.get() });
        self.busy.store(false, Ordering::Release);
        Some(r)
    }
}

/// The window-state generation now, if the process has the page.
fn generation() -> Option<u64> {
    let mut page = PAGE.load(Ordering::Acquire);
    if page == 0 {
        page = match crate::call(Request::SharedPage {}) {
            // A 32-bit process cannot reach a page above its address space.
            Reply::Value { value } if value > 1 && value <= usize::MAX as u64 => value,
            _ => 1,
        };
        PAGE.store(page, Ordering::Release);
    }
    if page == 1 {
        return None;
    }
    // SAFETY: the kernel mapped the page there, read-only, for the life of the process; its
    // first eight bytes are an aligned 64-bit counter.
    Some(unsafe { (*(page as *const AtomicU64)).load(Ordering::Acquire) })
}

/// Whether `req` is answered from the cache: `Window` (the window's record) and `Desktop` (the
/// screen, work area and DPI as the thread sees them).
pub fn cacheable(req: &Request) -> bool {
    matches!(req, Request::Window { .. } | Request::Desktop {})
}

/// The reply to `req` (one [`cacheable`] says yes to) as `fetch` gives it, from the cache while
/// the window state is unchanged.
pub fn reply(req: &Request, fetch: impl FnOnce() -> Reply) -> Reply {
    let Some(generation) = generation() else { return fetch() };
    let window = match req {
        Request::Window { hwnd } => Some(*hwnd),
        _ => None,
    };
    // SAFETY: no preconditions.
    let tid = unsafe { crate::GetCurrentThreadId() };
    let h = window.unwrap_or(0) as usize;
    let slot = &CACHE[(h ^ (h >> 16) ^ (tid as usize).wrapping_mul(0x9E37)) % SLOTS];
    let hit = slot.with(|e| match e {
        Some(e) if e.window == window && e.tid == tid && e.generation == generation => Some(e.reply.clone()),
        _ => None,
    });
    if let Some(Some(r)) = hit {
        return r;
    }
    let r = fetch();
    // Failures of the call itself (a full buffer, a refused request) are not kept.
    if !matches!(r, Reply::Error { code } if code == crate::ERROR_INVALID_PARAMETER) {
        slot.with(|e| *e = Some(Entry { window, tid, generation, reply: r.clone() }));
    }
    r
}
