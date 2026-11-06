//! win32k without the machine (ADR 0128).
//!
//! Windows GUI apps on Poly: `user32.dll`, `gdi32.dll` and `win32u.dll` are
//! Poly's own (real PE DLLs from Rust, ADR 0079) and run window procedures,
//! `DefWindowProc` and GDI drawing in the app's process; the state that is
//! shared between threads and processes (windows, message queues, timers,
//! hooks, input routing, window stations and desktops, the surfaces of
//! top-level windows) is win32k's, in the NT personality, reached through
//! table 1 of the NT system calls. Each visible top-level window is one
//! surface of the native compositor, over the **desktop link**:
//!
//! ```text
//!  app process                         kernel (NT personality)                 native compositor
//!  ───────────                         ───────────────────────                 ─────────────────
//!  user32 / gdi32 ──PolyUser* stubs──► win32k services (table 1) ──records──► /dev/win32k ──► scene windows,
//!   (window procs,    (win32u.dll)      windows, queues, timers,   ◄─records── (one opener)     Poly's frame,
//!    GDI drawing into                   hooks, input routing,                                    input, focus
//!    the surface) ◄──── callouts ─────  surfaces (memory objects) ──── buffers mapped by both ──►
//! ```
//!
//! Everything here is tested on the host and touches no process, memory
//! or device:
//!
//! - [`services`]: Poly's win32k system calls, their numbers and names (the
//!   ABI between `win32u.dll` and the personality; `win32u`'s stubs are
//!   generated from it).
//! - [`link`]: the desktop link's records and their wire form.
//! - [`desktop`]: win32k's end of the link: the surfaces of top-level
//!   windows, the records they produce, the replay for a (re)connecting
//!   compositor, the compositor's records read back.
//! - [`frame`]: Poly's frame as the non-client area of a top-level window:
//!   rectangles and the non-client hit test.
//! - [`kbd`]: the keyboard layout (the oracle's German layout): virtual
//!   keys, scan codes, characters and dead keys for `MapVirtualKeyW`,
//!   `ToUnicode` and `TranslateMessage`.
//! - [`exports`]: what `user32.dll`, `gdi32.dll` and `win32u.dll` must
//!   export, checked by `cargo xtask win32`.
//!
//! Behaviour of individual Windows functions comes from the spec sheets
//! under `specs/windows/`; what is not implemented yet says so
//! (`STATUS_NOT_IMPLEMENTED` from the services, `ERROR_CALL_NOT_IMPLEMENTED`
//! from the DLLs' stubs).

#![no_std]

extern crate alloc;

pub mod desktop;
pub mod exports;
pub mod frame;
pub mod kbd;
pub mod link;
pub mod metrics;
pub mod proto;
pub mod rgn;
pub mod services;
pub mod surfaces;
pub mod user;
pub mod wire;
pub mod wm;
