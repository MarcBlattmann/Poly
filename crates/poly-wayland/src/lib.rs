//! A Wayland server core and a software compositor (ADR 0106).
//!
//! ```text
//!  client bytes + fds ──▶ wire ──▶ server (objects, dispatch) ──▶ scene ──▶ render ──▶ Framebuffer
//!                         decode    core, xdg-shell, decoration,   windows,   premultiplied   virtio-gpu,
//!                         encode    presentation protocol logic    z-order    blend, frame    memory, ...
//! ```
//!
//! - [`wire`]: the wire format — message headers, the argument types, decoding
//!   and encoding against the protocol's tables.
//! - [`protocol`]: request and event types and tables for every interface of
//!   `wayland.xml`, `xdg-shell`, `xdg-decoration`, `viewporter`,
//!   `presentation-time`, `linux-dmabuf` and `xwayland-shell-v1`, generated
//!   from the XML by `cargo xtask wayland`.
//! - [`server`]: clients and their objects, request dispatch, and the
//!   protocol logic: `wl_display`/`wl_registry`/`wl_callback`,
//!   `wl_compositor`/`wl_surface`/`wl_region` (double-buffered state, damage,
//!   frame callbacks, buffer scale), `wl_shm`/`wl_shm_pool`/`wl_buffer`
//!   (`argb8888`, `xrgb8888`), `wl_output` (with its scale), `wl_seat`
//!   (capabilities), `xdg_wm_base`/`xdg_positioner`/`xdg_surface`/
//!   `xdg_toplevel`/`xdg_popup`, `zxdg_decoration_manager_v1` (always
//!   server-side), `wp_presentation`, and `xwayland_shell_v1` with X
//!   windows as scene windows (ADR 0118). The transport and shared memory
//!   are the caller's ([`server::Host`]).
//! - [`compose`]: the scene of windows and the renderer: damage tracking,
//!   HiDPI scaling, premultiplied alpha, Poly's neutral window frame, behind
//!   the [`compose::Framebuffer`] trait.
//! - [`seat`]: what the input stack plugs into; `server` routes it to clients.
//!
//! `no_std` with `alloc`: nothing here needs an operating system, so all of
//! it is tested on the host, and the kernel could use it.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod compose;
pub mod protocol;
pub mod seat;
pub mod server;
pub mod wire;
