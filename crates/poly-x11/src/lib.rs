//! The X11 wire protocol and an X11 window manager for Xwayland (ADR 0118).
//!
//! ```text
//!  WM fd bytes ──▶ conn (setup, sequence numbers) ──▶ replies, errors, events ──▶ xwm ──▶ actions for the compositor
//!               ◀── requests (CreateWindow, ConfigureWindow, ChangeProperty, SetInputFocus, SendEvent, ...) ◀──┘
//! ```
//!
//! - [`wire`]: byte order, padding, reading and writing the protocol's
//!   integer types (the client chooses little-endian, `'l'`).
//! - [`setup`]: the connection setup request and the server's reply
//!   (vendor, resource ID base and mask, the screens with their root
//!   windows and sizes).
//! - [`request`]: the core requests a window manager makes, encoded as the
//!   protocol describes them (opcode, data byte, length in 4-byte units,
//!   then the fields; value lists by mask bit).
//! - [`event`]: the 32-byte events, errors and the split of the server's
//!   byte stream into replies, errors and events.
//! - [`conn`]: a client connection without I/O: the caller feeds it the
//!   bytes it read and sends what it queued; it numbers requests, matches
//!   replies and errors to them by sequence number, and hands out
//!   resource IDs.
//! - [`atoms`]: the predefined atoms and the names the window manager
//!   interns.
//! - [`icccm`]: the properties of the Inter-Client Communication
//!   Conventions Manual and of the Extended Window Manager Hints the
//!   window manager reads (`WM_NAME`, `_NET_WM_NAME`, `WM_CLASS`,
//!   `WM_PROTOCOLS`, `WM_HINTS`, `WM_NORMAL_HINTS`, `WM_TRANSIENT_FOR`,
//!   `_NET_WM_WINDOW_TYPE`, `_MOTIF_WM_HINTS`) and writes (`WM_STATE`).
//! - [`xwm`]: the window manager Xwayland gets on its `-wm` descriptor, as a
//!   state machine: X events in, X requests and [`xwm::Action`]s for the
//!   compositor out.
//!
//! Sources, all public: "X Window System Protocol, Version 11" (the wire
//! encoding of its appendix B, the predefined atoms), the "Inter-Client
//! Communication Conventions Manual" 2.0, the freedesktop.org "Extended
//! Window Manager Hints" 1.5, Xwayland(1) and the `xwayland-shell-v1`
//! protocol XML (`WL_SURFACE_SERIAL`, `WL_SURFACE_ID`), and Motif's public
//! `Xm/MwmUtil.h` for the layout of `_MOTIF_WM_HINTS`. No Xlib, XCB, X
//! server or window manager source was read (CLEAN_ROOM.md).
//!
//! `no_std` with `alloc`: everything is tested on the host with byte
//! buffers.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod atoms;
pub mod conn;
pub mod event;
pub mod icccm;
pub mod request;
pub mod setup;
pub mod wire;
pub mod xwm;

/// An X resource ID (window, pixmap, ...) or atom; 0 is `None`.
pub type Xid = u32;

/// `CurrentTime` in requests that take a timestamp.
pub const CURRENT_TIME: u32 = 0;
