//! The desktop's Wayland test clients as native Poly programs (ADR 0125,
//! ADR 0133): [`demo`] is `/bin/wl-demo`, the launcher's "Poly Demo" and
//! the window the desktop boot tests drive; [`client`] is `/bin/wl-client`,
//! the compositor's protocol test ("Wayland Test Client" in the launcher).
//!
//! They used to be C programs (`tests/linux/wayland/wl-demo.c`,
//! `wl-client.c`), which only a Linux host with a static C library could
//! build, so a desktop built on Windows had no test apps. These speak the
//! same wire protocol, written from the protocol XML (`wayland.xml`,
//! `xdg-shell.xml`, `xdg-decoration-unstable-v1.xml`; no libwayland), on
//! `poly-rt`'s sockets with handle passing and memory objects, and print
//! the same lines, so the boot tests check them the same way. The C
//! sources stay: boot group `wayland` still runs the Linux `wl-client` (the
//! Linux personality's `SCM_RIGHTS` and `/dev/shm`).
//!
//! [`wire`] is the connection: requests built and queued, flushed with at
//! most one handle, events parsed with the handles that came with them.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod client;
pub mod demo;
pub mod wire;
