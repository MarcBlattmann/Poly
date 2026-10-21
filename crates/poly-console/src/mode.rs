//! `GetConsoleMode` / `SetConsoleMode` (Learn: "SetConsoleMode",
//! "Low-Level Console Modes", "High-Level Console Modes").
//!
//! - A new console's input buffer has every input mode on except
//!   `ENABLE_WINDOW_INPUT` and `ENABLE_VIRTUAL_TERMINAL_INPUT` (Learn:
//!   "When a console is created, all input modes except ... are enabled by
//!   default"): [`DEFAULT_INPUT_MODE`].
//! - A new screen buffer has both classic output modes on ("both output
//!   modes are enabled by default"): [`DEFAULT_OUTPUT_MODE`].
//! - `ENABLE_INSERT_MODE` and `ENABLE_QUICK_EDIT_MODE` change only when the
//!   new mode has `ENABLE_EXTENDED_FLAGS` ("To enable ... the
//!   ENABLE_EXTENDED_FLAGS flag must also be set"); otherwise the old
//!   values stay.
//! - Unknown bits, and `ENABLE_ECHO_INPUT` without `ENABLE_LINE_INPUT`
//!   ("can be used only if the ENABLE_LINE_INPUT mode is also enabled"), are
//!   `ERROR_INVALID_PARAMETER` — Poly's reading of the page, to be
//!   confirmed by the spec agent.

use crate::consts::*;

/// `ERROR_INVALID_PARAMETER` (winerror.h).
pub const ERROR_INVALID_PARAMETER: u32 = 87;

pub const DEFAULT_INPUT_MODE: u32 = ENABLE_PROCESSED_INPUT
    | ENABLE_LINE_INPUT
    | ENABLE_ECHO_INPUT
    | ENABLE_MOUSE_INPUT
    | ENABLE_INSERT_MODE
    | ENABLE_QUICK_EDIT_MODE
    | ENABLE_EXTENDED_FLAGS
    | ENABLE_AUTO_POSITION;

pub const DEFAULT_OUTPUT_MODE: u32 = ENABLE_PROCESSED_OUTPUT | ENABLE_WRAP_AT_EOL_OUTPUT;

/// Every input mode bit `wincon.h` / `consoleapi.h` defines.
pub const VALID_INPUT_MODE: u32 = 0x03FF;
/// Every output mode bit.
pub const VALID_OUTPUT_MODE: u32 = 0x001F;

const EXTENDED: u32 = ENABLE_INSERT_MODE | ENABLE_QUICK_EDIT_MODE | ENABLE_EXTENDED_FLAGS;

/// The input mode `SetConsoleMode(input, new)` leaves, given the `old`
/// one, or the Win32 error.
pub fn set_input(old: u32, new: u32) -> Result<u32, u32> {
    if new & !VALID_INPUT_MODE != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if new & ENABLE_ECHO_INPUT != 0 && new & ENABLE_LINE_INPUT == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if new & ENABLE_EXTENDED_FLAGS != 0 { Ok(new) } else { Ok((new & !EXTENDED) | (old & EXTENDED)) }
}

/// The output mode `SetConsoleMode(output, new)` leaves, or the Win32
/// error.
pub fn set_output(new: u32) -> Result<u32, u32> {
    if new & !VALID_OUTPUT_MODE != 0 { Err(ERROR_INVALID_PARAMETER) } else { Ok(new) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_learn() {
        assert_eq!(DEFAULT_INPUT_MODE, 0x1F7);
        assert_eq!(DEFAULT_INPUT_MODE & (ENABLE_WINDOW_INPUT | ENABLE_VIRTUAL_TERMINAL_INPUT), 0);
        assert_eq!(DEFAULT_OUTPUT_MODE, 3);
    }

    #[test]
    fn input_mode_checks() {
        assert_eq!(
            set_input(DEFAULT_INPUT_MODE, 0),
            Ok(ENABLE_INSERT_MODE | ENABLE_QUICK_EDIT_MODE | ENABLE_EXTENDED_FLAGS)
        );
        assert_eq!(set_input(DEFAULT_INPUT_MODE, ENABLE_EXTENDED_FLAGS), Ok(ENABLE_EXTENDED_FLAGS));
        assert_eq!(set_input(0, ENABLE_ECHO_INPUT), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set_input(0, 0x400), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set_input(0, ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT), Ok(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT));
        assert_eq!(set_input(0, ENABLE_VIRTUAL_TERMINAL_INPUT), Ok(ENABLE_VIRTUAL_TERMINAL_INPUT));
    }

    #[test]
    fn output_mode_checks() {
        assert_eq!(set_output(7), Ok(7));
        assert_eq!(set_output(0x20), Err(ERROR_INVALID_PARAMETER));
    }
}
