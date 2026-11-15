//! The `winuser.h` structures `user32` reads from and writes to its callers' memory, in the
//! process's own layout: pointer-sized fields are `usize`, so the same source serves the x64
//! `user32` and the i686 one of a 32-bit process (ADR 0186), where every handle and pointer is
//! 4 bytes and the structures are shorter.

/// `WNDCLASSEXW` (80 bytes on x64, 48 in 32 bits).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct WndClassEx {
    pub size: u32,
    pub style: u32,
    pub wndproc: usize,
    pub cls_extra: i32,
    pub wnd_extra: i32,
    pub instance: usize,
    pub icon: usize,
    pub cursor: usize,
    pub brush: usize,
    pub menu_name: usize,
    pub class_name: usize,
    pub icon_sm: usize,
}

/// `WNDCLASSW` (72 bytes on x64, 40 in 32 bits).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct WndClass {
    pub style: u32,
    pub wndproc: usize,
    pub cls_extra: i32,
    pub wnd_extra: i32,
    pub instance: usize,
    pub icon: usize,
    pub cursor: usize,
    pub brush: usize,
    pub menu_name: usize,
    pub class_name: usize,
}

/// `GUITHREADINFO` (72 bytes on x64, 48 in 32 bits).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GuiThreadInfo {
    pub size: u32,
    pub flags: u32,
    pub active: usize,
    pub focus: usize,
    pub capture: usize,
    pub menu_owner: usize,
    pub move_size: usize,
    pub caret: usize,
    pub caret_rect: [i32; 4],
}

/// `TRACKMOUSEEVENT` (24 bytes on x64, 16 in 32 bits).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TrackMouseEvent {
    pub size: u32,
    pub flags: u32,
    pub hwnd: usize,
    pub hover: u32,
}

/// `ICONINFO` (32 bytes on x64, 20 in 32 bits).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct IconInfo {
    pub icon: u32,
    pub x: i32,
    pub y: i32,
    pub mask: usize,
    pub color: usize,
}

/// `CURSORINFO` (24 bytes on x64, 20 in 32 bits).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CursorInfo {
    pub size: u32,
    pub flags: u32,
    pub cursor: usize,
    pub x: i32,
    pub y: i32,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<WndClassEx>() == 80);
    assert!(core::mem::size_of::<WndClass>() == 72);
    assert!(core::mem::size_of::<GuiThreadInfo>() == 72);
    assert!(core::mem::size_of::<TrackMouseEvent>() == 24);
    assert!(core::mem::size_of::<IconInfo>() == 32);
    assert!(core::mem::size_of::<CursorInfo>() == 24);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(core::mem::size_of::<WndClassEx>() == 48);
    assert!(core::mem::size_of::<WndClass>() == 40);
    assert!(core::mem::size_of::<GuiThreadInfo>() == 48);
    assert!(core::mem::size_of::<TrackMouseEvent>() == 16);
    assert!(core::mem::size_of::<IconInfo>() == 20);
    assert!(core::mem::size_of::<CursorInfo>() == 20);
};
