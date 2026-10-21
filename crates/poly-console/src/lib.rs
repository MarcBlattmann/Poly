//! The Windows console without the machine (ADR 0080).
//!
//! Poly's console host lives in the NT personality: a console is an object
//! backed by one of Poly's terminals (the serial port, the screen, a
//! pseudo-terminal), and Windows programs reach it through console handles
//! (file objects on `\Device\PolyConsole`). Everything that does not touch
//! a terminal or a process is here, tested on the host:
//!
//! ```text
//!   terminal bytes ──► input::TermDecoder ──► INPUT_RECORDs ──► Console input buffer
//!                                   Ctrl+C (processed input) ──► control events
//!   ReadConsole / ReadFile ◄── cooked::LineEditor (line input, echo) ◄─┘
//!   WriteConsole / WriteFile ──► codepage ──► screen::Screen (processed output,
//!                       VT sequences) ──► the model + VT bytes for the terminal
//! ```
//!
//! - [`consts`]: the modes, attributes, events and key codes of `wincon.h`,
//!   `consoleapi.h` and `wincontypes.h`.
//! - [`mode`]: `SetConsoleMode`'s checks and the modes a console starts with.
//! - [`codepage`]: the console code pages (UTF-8, 437, 1252, US-ASCII).
//! - [`input`]: `INPUT_RECORD`s and terminal bytes decoded into key events.
//! - [`vt`]: the VT sequence parser and SGR ↔ attribute mapping.
//! - [`screen`]: the screen buffer, its processed output and the VT bytes
//!   that keep the terminal showing the same.
//! - [`cooked`]: cooked-mode line editing (`ENABLE_LINE_INPUT`).
//! - [`console`]: one console: all of the above together.
//! - [`ioctl`]: Poly's own protocol between `kernelbase` and the console
//!   host (`NtDeviceIoControlFile` codes and their buffers).
//! - [`ctrl`]: the per-process control-handler list
//!   (`SetConsoleCtrlHandler`).
//!
//! Sources (clean room, CLEAN_ROOM.md): Microsoft Learn: "Consoles",
//! "Console Functions" and each function's reference page, "Console
//! Handles", "Console Virtual Terminal Sequences", "Low-Level Console
//! Modes", "High-Level Console Modes", "Creation of a Console",
//! "HandlerRoutine callback function", "CONSOLE_SCREEN_BUFFER_INFOEX",
//! "INPUT_RECORD", "KEY_EVENT_RECORD", "Code Page Identifiers"; the public
//! SDK headers' layouts and constants (`wincon.h`, `consoleapi.h`,
//! `wincontypes.h`, `winuser.h` for virtual-key codes); ECMA-48 for the
//! terminal side; the published IBM PC code page 437 and Windows-1252
//! character sets. No Windows binary, conhost, Wine or ReactOS source was
//! looked at. What these leave open is Poly's choice, marked "to be
//! confirmed by the spec agent".

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod codepage;
pub mod console;
pub mod consts;
pub mod cooked;
pub mod ctrl;
pub mod input;
pub mod ioctl;
pub mod mode;
pub mod screen;
pub mod vt;

pub use console::{Console, ReadOutcome};
pub use input::{InputRecord, KeyEvent};
pub use screen::Screen;
