//! A minimal D-Bus session bus for Poly's desktop (ADR 0117): enough of
//! the bus for clients to reach the shell's notifications through the
//! standard `org.freedesktop.Notifications` interface, the way GTK, libnotify
//! (`notify-send`) and most desktop programs do.
//!
//! Sources, all public: the D-Bus Specification (version 0.43: the type
//! system and marshalling, the message format and header fields, valid
//! names, the authentication protocol, the message bus with its unique and
//! well-known names, match rules and the `org.freedesktop.DBus` methods
//! and signals) and the Desktop Notifications Specification (version 1.2:
//! `Notify`, `CloseNotification`, `GetCapabilities`,
//! `GetServerInformation`, the hints, the `NotificationClosed` reasons and
//! `ActionInvoked`). No D-Bus implementation's source was read
//! (CLEAN_ROOM.md).
//!
//! - [`wire`]: values, signatures, marshalling in both byte orders.
//! - [`message`]: messages, their header fields, and a reader that cuts a
//!   byte stream into messages.
//! - [`auth`]: the SASL exchange before messages flow (server side, and the
//!   lines a client sends).
//! - [`bus`]: the message bus: connections, names, routing, match rules,
//!   the bus driver; services of the bus's own process are "local".
//! - [`notifications`]: `org.freedesktop.Notifications` requests decoded,
//!   replies and signals encoded.
//!
//! `no_std` with `alloc`, no I/O: the caller owns the sockets and hands the
//! bus each connection's bytes ([`bus::Bus::receive`]), as with
//! `poly-wayland`.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod auth;
pub mod bus;
pub mod message;
pub mod notifications;
pub mod wire;

pub use message::{Message, MessageType};
pub use wire::Value;
