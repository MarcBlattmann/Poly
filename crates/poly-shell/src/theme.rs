//! The shell's measurements and colours, in logical pixels: the look of
//! the owner's Alpine Shell re-created (ADR 0107 lists where each value
//! comes from). Colours are `0xRRGGBB`; "white at a" is white drawn with
//! coverage `a` over what is below, as Alpine's `white/10`-style fills.

/// The ridge: full width, flat, opaque.
pub mod ridge {
    pub const HEIGHT: i32 = 24;
    /// Alpine's default bar colour, rgb(32, 32, 32).
    pub const BACKGROUND: u32 = 0x20_20_20;
    pub const TEXT: u32 = 0xff_ff_ff;
    pub const TEXT_SIZE: f32 = 12.0;
    /// Items sit in 18-pixel pills; between items 4 pixels.
    pub const PILL_HEIGHT: i32 = 18;
    pub const PILL_PADDING: i32 = 8;
    pub const GAP: i32 = 4;
    /// The right group's margin from the screen's edge.
    pub const RIGHT_MARGIN: i32 = 4;
    /// The status pill's fill: white at 10 %.
    pub const PILL_ALPHA: u8 = 26;
    pub const ICON: i32 = 14;
    /// The menu glyph on the left, 8 pixels from the edge.
    pub const MENU_ICON_X: i32 = 8;
}

/// The launcher. Alpine's sizes, but floating in the middle of the screen
/// (the owner's direction) instead of hanging from the bar.
pub mod launcher {
    pub const WIDTH: i32 = 720;
    /// Never closer than this to the screen's sides.
    pub const SIDE_MARGIN: i32 = 16;
    pub const RADIUS: i32 = 14;
    pub const BACKGROUND: u32 = 0x20_20_20;
    /// A 1-pixel outline, white at 12 %.
    pub const OUTLINE_ALPHA: u8 = 31;
    pub const SEARCH_HEIGHT: i32 = 48;
    pub const SEARCH_PADDING: i32 = 16;
    pub const SEARCH_GAP: i32 = 8;
    pub const SEARCH_ICON: i32 = 16;
    /// The search glyph at 60 % opacity.
    pub const SEARCH_ICON_ALPHA: u8 = 153;
    pub const INPUT_SIZE: f32 = 15.0;
    pub const PLACEHOLDER: &str = "Search apps";
    /// Placeholder text: white at 40 %.
    pub const PLACEHOLDER_ALPHA: u8 = 102;
    /// Dividers: white at 10 %.
    pub const DIVIDER_ALPHA: u8 = 26;
    /// The list's padding (left, right, bottom).
    pub const LIST_PADDING: i32 = 6;
    pub const SECTION_HEIGHT: i32 = 28;
    pub const SECTION_SIZE: f32 = 11.0;
    /// Section titles: white at 50 %.
    pub const SECTION_ALPHA: u8 = 128;
    pub const ROW_HEIGHT: i32 = 48;
    pub const ROW_GAP: i32 = 2;
    pub const ROW_PADDING: i32 = 10;
    pub const ROW_RADIUS: i32 = 6;
    pub const ROW_ICON_GAP: i32 = 12;
    /// The selected row: white at 10 %; a hovered one at 5 %.
    pub const SELECTED_ALPHA: u8 = 26;
    pub const HOVER_ALPHA: u8 = 13;
    pub const ICON_BOX: i32 = 28;
    pub const TITLE_SIZE: f32 = 13.0;
    pub const SUBTITLE_SIZE: f32 = 11.0;
    pub const SUBTITLE_ALPHA: u8 = 128;
    pub const FOOTER_HEIGHT: i32 = 36;
    pub const FOOTER_SIZE: f32 = 11.0;
    /// Rows shown at most (Alpine's default of 8).
    pub const MAX_ROWS: usize = 8;
    /// The search row sits this far above the screen's middle, as a share
    /// of the screen's height (the visual centre, so the list has room).
    pub const RAISE_PERCENT: i32 = 10;
    /// Alpine's floating shadow token, `0 10px 15px -3px rgba(0,0,0,.5)`,
    /// approximated by layers.
    pub const SHADOW_OFFSET: i32 = 10;
    pub const SHADOW_SPREAD: i32 = 12;
    pub const SHADOW_ALPHA: u8 = 128;
}

/// The desktop behind the windows: a vertical gradient (no wallpaper
/// files yet), dark like Alpine's default theme.
pub mod wallpaper {
    pub const TOP: u32 = 0x2c_3e_50;
    pub const BOTTOM: u32 = 0x10_16_1d;
    /// A soft light from the upper middle.
    pub const GLOW: u32 = 0x4a_6a_82;
}
