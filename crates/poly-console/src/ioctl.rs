//! Poly's own protocol between `kernelbase` and the console host (ADR
//! 0080): the names of console objects and the `NtDeviceIoControlFile`
//! codes on console handles. Windows' own console driver protocol is not
//! public and is not imitated; only `kernelbase` speaks this one.
//!
//! # Names
//!
//! Console handles are file objects of the device `\Device\PolyConsole`:
//!
//! | Name | Opens |
//! |---|---|
//! | `\Device\PolyConsole\New` | a new console (`AllocConsole`) and a connection to it |
//! | `\Device\PolyConsole\<id>` | a connection to console `<id>` (attaching to it) |
//! | `\Device\PolyConsole\Process\<pid>` | a connection to the console process `<pid>` is attached to (`AttachConsole`) |
//! | `Input` relative to a console handle | its input buffer (`CONIN$`) |
//! | `Output` relative to a console handle | its screen buffer (`CONOUT$`) |
//!
//! A process is attached to a console while it holds a connection;
//! the console goes away when no connection is left.
//!
//! # Control codes
//!
//! `CTL_CODE(FILE_DEVICE_CONSOLE, function, METHOD_BUFFERED,
//! FILE_ANY_ACCESS)` (`winioctl.h`'s layout) with Poly's function numbers
//! from 0x800 (the range `winioctl.h` leaves to others than Microsoft).
//! Integers are little-endian `u32`s unless noted; `IO_STATUS_BLOCK.Information`
//! is the number of output bytes.

/// `FILE_DEVICE_CONSOLE` (`winioctl.h`).
pub const FILE_DEVICE_CONSOLE: u32 = 0x50;

/// `CTL_CODE(FILE_DEVICE_CONSOLE, function, METHOD_BUFFERED, FILE_ANY_ACCESS)`.
pub const fn code(function: u32) -> u32 {
    (FILE_DEVICE_CONSOLE << 16) | (function << 2)
}

/// Input or output handle → its mode (out: `u32`).
pub const GET_MODE: u32 = code(0x800);
/// Input or output handle; in: the new mode.
pub const SET_MODE: u32 = code(0x801);
/// Input handle; out: UTF-16 text (`ReadConsoleW`). May wait.
pub const READ_W: u32 = code(0x802);
/// Output handle; in: UTF-16 text (`WriteConsoleW`); out: `u32` units written.
pub const WRITE_W: u32 = code(0x803);
/// Output handle; in: bytes in the output code page (`WriteConsoleA`);
/// out: `u32` bytes written.
pub const WRITE_A: u32 = code(0x804);
/// Input handle; out: bytes in the input code page (`ReadConsoleA`). May wait.
pub const READ_A: u32 = code(0x805);
/// Input handle; in: [`READ_INPUT_REMOVE`] / [`READ_INPUT_WAIT`] flags;
/// out: `INPUT_RECORD`s. With `WAIT` it waits for at least one.
pub const READ_INPUT: u32 = code(0x806);
/// Input handle; out: `u32` records waiting.
pub const INPUT_COUNT: u32 = code(0x807);
/// Input handle: throw the input away.
pub const FLUSH_INPUT: u32 = code(0x808);
/// Output handle; out: `CONSOLE_SCREEN_BUFFER_INFOEX` (96 bytes).
pub const GET_INFO: u32 = code(0x809);
/// Output handle; in: the attribute (`u32`).
pub const SET_ATTRIBUTE: u32 = code(0x80A);
/// Output handle; in: `COORD` (two `i16`).
pub const SET_CURSOR: u32 = code(0x80B);
/// Output handle; in: [`Fill`] (16 bytes); out: `u32` cells written.
pub const FILL: u32 = code(0x80C);
/// Any console handle; in: 0 (input) or 1 (output); out: the code page.
pub const GET_CP: u32 = code(0x80D);
/// Any console handle; in: 0 or 1, then the code page.
pub const SET_CP: u32 = code(0x80E);
/// Any console handle; out: the title (UTF-16, no NUL); a short buffer
/// gets what fits and `STATUS_BUFFER_OVERFLOW`.
pub const GET_TITLE: u32 = code(0x80F);
/// Any console handle; in: the title (UTF-16, no NUL).
pub const SET_TITLE: u32 = code(0x810);
/// A connection; in: the address (`u64`) of the routine the console host
/// starts a thread at, with the event as its argument, for a control event.
pub const SET_CTRL_ROUTINE: u32 = code(0x811);
/// Any console handle; out: the console's id (`u32`).
pub const GET_ID: u32 = code(0x812);
/// Any console handle; in: the event: raised for every process attached
/// (`GenerateConsoleCtrlEvent`).
pub const GENERATE_CTRL: u32 = code(0x813);
/// Input handle; in: `INPUT_RECORD`s (`WriteConsoleInputW`); out: `u32`
/// records written.
pub const WRITE_INPUT: u32 = code(0x814);

/// `READ_INPUT`: take the records (`ReadConsoleInputW`) rather than look
/// (`PeekConsoleInputW`).
pub const READ_INPUT_REMOVE: u32 = 1;
/// `READ_INPUT`: wait until there is at least one record.
pub const READ_INPUT_WAIT: u32 = 2;

/// `FILL`'s input: `kind` ([`FILL_CHARACTER`] or [`FILL_ATTRIBUTE`]),
/// `value`, `count`, then the start `COORD`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fill {
    pub kind: u32,
    pub value: u32,
    pub count: u32,
    pub x: i16,
    pub y: i16,
}

pub const FILL_CHARACTER: u32 = 0;
pub const FILL_ATTRIBUTE: u32 = 1;

impl Fill {
    pub const SIZE: usize = 16;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.kind.to_le_bytes());
        b[4..8].copy_from_slice(&self.value.to_le_bytes());
        b[8..12].copy_from_slice(&self.count.to_le_bytes());
        b[12..14].copy_from_slice(&self.x.to_le_bytes());
        b[14..16].copy_from_slice(&self.y.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Fill> {
        if b.len() < Self::SIZE {
            return None;
        }
        let u = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Some(Fill {
            kind: u(0),
            value: u(4),
            count: u(8),
            x: i16::from_le_bytes([b[12], b[13]]),
            y: i16::from_le_bytes([b[14], b[15]]),
        })
    }
}

/// The device's name and the names below it.
pub const DEVICE: &str = "\\Device\\PolyConsole";
pub const NEW: &str = "New";
pub const PROCESS: &str = "Process";
pub const INPUT: &str = "Input";
pub const OUTPUT: &str = "Output";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_buffered_console_codes() {
        assert_eq!(GET_MODE, 0x0050_2000);
        assert_eq!(GET_MODE & 3, 0);
        assert_eq!(GENERATE_CTRL >> 16, FILE_DEVICE_CONSOLE);
        let f = Fill { kind: FILL_ATTRIBUTE, value: 0x1E, count: 7, x: 3, y: -1 };
        assert_eq!(Fill::from_bytes(&f.to_bytes()), Some(f));
        assert_eq!(Fill::from_bytes(&[0; 4]), None);
    }
}
