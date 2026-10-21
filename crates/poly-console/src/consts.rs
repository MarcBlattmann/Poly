//! Constants of the console API, from the public SDK headers named on each
//! group (their meanings from the Learn pages of the functions that use
//! them).

// ----- input modes (consoleapi.h; Learn: "SetConsoleMode")

pub const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
pub const ENABLE_LINE_INPUT: u32 = 0x0002;
pub const ENABLE_ECHO_INPUT: u32 = 0x0004;
pub const ENABLE_WINDOW_INPUT: u32 = 0x0008;
pub const ENABLE_MOUSE_INPUT: u32 = 0x0010;
pub const ENABLE_INSERT_MODE: u32 = 0x0020;
pub const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
pub const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
pub const ENABLE_AUTO_POSITION: u32 = 0x0100;
pub const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

// ----- output modes (consoleapi.h)

pub const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
pub const ENABLE_WRAP_AT_EOL_OUTPUT: u32 = 0x0002;
pub const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
pub const DISABLE_NEWLINE_AUTO_RETURN: u32 = 0x0008;
pub const ENABLE_LVB_GRID_WORLDWIDE: u32 = 0x0010;

// ----- character attributes (wincon.h; Learn: "Console Screen Buffers")

pub const FOREGROUND_BLUE: u16 = 0x0001;
pub const FOREGROUND_GREEN: u16 = 0x0002;
pub const FOREGROUND_RED: u16 = 0x0004;
pub const FOREGROUND_INTENSITY: u16 = 0x0008;
pub const BACKGROUND_BLUE: u16 = 0x0010;
pub const BACKGROUND_GREEN: u16 = 0x0020;
pub const BACKGROUND_RED: u16 = 0x0040;
pub const BACKGROUND_INTENSITY: u16 = 0x0080;
pub const COMMON_LVB_REVERSE_VIDEO: u16 = 0x4000;
pub const COMMON_LVB_UNDERSCORE: u16 = 0x8000;
/// The attribute a screen buffer starts with: light grey on black.
pub const DEFAULT_ATTRIBUTE: u16 = FOREGROUND_RED | FOREGROUND_GREEN | FOREGROUND_BLUE;

// ----- input records (wincontypes.h; Learn: "INPUT_RECORD")

pub const KEY_EVENT: u16 = 0x0001;
pub const MOUSE_EVENT: u16 = 0x0002;
pub const WINDOW_BUFFER_SIZE_EVENT: u16 = 0x0004;
pub const MENU_EVENT: u16 = 0x0008;
pub const FOCUS_EVENT: u16 = 0x0010;
/// `sizeof(INPUT_RECORD)`: `EventType` (a `WORD`, padded to 4) and the
/// 16-byte event union.
pub const INPUT_RECORD_SIZE: usize = 20;

// ----- control key state (wincontypes.h; Learn: "KEY_EVENT_RECORD")

pub const RIGHT_ALT_PRESSED: u32 = 0x0001;
pub const LEFT_ALT_PRESSED: u32 = 0x0002;
pub const RIGHT_CTRL_PRESSED: u32 = 0x0004;
pub const LEFT_CTRL_PRESSED: u32 = 0x0008;
pub const SHIFT_PRESSED: u32 = 0x0010;
pub const NUMLOCK_ON: u32 = 0x0020;
pub const SCROLLLOCK_ON: u32 = 0x0040;
pub const CAPSLOCK_ON: u32 = 0x0080;
pub const ENHANCED_KEY: u32 = 0x0100;

// ----- virtual-key codes (winuser.h; Learn: "Virtual-Key Codes")

pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_SPACE: u16 = 0x20;
pub const VK_PRIOR: u16 = 0x21;
pub const VK_NEXT: u16 = 0x22;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_INSERT: u16 = 0x2D;
pub const VK_DELETE: u16 = 0x2E;
pub const VK_F1: u16 = 0x70;

// ----- control events (wincon.h; Learn: "HandlerRoutine callback function")

pub const CTRL_C_EVENT: u32 = 0;
pub const CTRL_BREAK_EVENT: u32 = 1;
pub const CTRL_CLOSE_EVENT: u32 = 2;
pub const CTRL_LOGOFF_EVENT: u32 = 5;
pub const CTRL_SHUTDOWN_EVENT: u32 = 6;

// ----- AttachConsole (wincon.h)

/// `ATTACH_PARENT_PROCESS`: `(DWORD)-1`.
pub const ATTACH_PARENT_PROCESS: u32 = u32::MAX;

// ----- code pages (Learn: "Code Page Identifiers")

pub const CP_437: u32 = 437;
pub const CP_1252: u32 = 1252;
pub const CP_US_ASCII: u32 = 20127;
pub const CP_UTF8: u32 = 65001;

/// `CONSOLE_SCREEN_BUFFER_INFO` and `CONSOLE_SCREEN_BUFFER_INFOEX` sizes
/// (wincon.h, x64: `COORD`s and `SMALL_RECT` are made of `SHORT`s).
pub const SCREEN_BUFFER_INFO_SIZE: usize = 22;
pub const SCREEN_BUFFER_INFOEX_SIZE: usize = 96;

/// `STATUS_CONTROL_C_EXIT` (ntstatus.h): the exit code of a process the
/// default control handler ends (Learn: "HandlerRoutine": the default
/// handler calls `ExitProcess`).
pub const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;
