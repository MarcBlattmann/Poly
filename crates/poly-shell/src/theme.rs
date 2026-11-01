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

/// The desktop behind the windows: generated gradients (no wallpaper
/// files yet; Alpine's bundled images are not ours), dark like Alpine's
/// default theme. Each preset is a vertical gradient from `top` to
/// `bottom` with a soft light of `glow` from the upper middle.
pub mod wallpaper {
    pub const TOP: u32 = 0x2c_3e_50;
    pub const BOTTOM: u32 = 0x10_16_1d;
    /// A soft light from the upper middle.
    pub const GLOW: u32 = 0x4a_6a_82;

    /// (ID, label, top, bottom, glow); the first is the default.
    pub const PRESETS: &[(&str, &str, u32, u32, u32)] = &[
        ("slate", "Slate", TOP, BOTTOM, GLOW),
        ("dusk", "Dusk", 0x3b_2a_4a, 0x14_0f_1c, 0x7a_5a_8c),
        ("forest", "Forest", 0x24_44_3a, 0x0d_17_14, 0x4f_7f_6a),
        ("ember", "Ember", 0x4a_2f_24, 0x17_0e_0b, 0x8c_5a_44),
        ("ocean", "Ocean", 0x1f_3f_5a, 0x0a_14_1d, 0x3f_7f_a6),
        ("graphite", "Graphite", 0x3a_3a_3a, 0x12_12_12, 0x5e_5e_5e),
    ];

    /// The colours (top, bottom, glow) of preset `id` (the default's if
    /// unknown).
    pub fn colours(id: &str) -> (u32, u32, u32) {
        let p = PRESETS.iter().find(|p| p.0 == id).unwrap_or(&PRESETS[0]);
        (p.2, p.3, p.4)
    }
}

/// Notifications: Alpine Shell's notice measurements, as floating cards
/// (ADR 0117), stacked in the top right corner below the ridge.
pub mod notice {
    pub const WIDTH: i32 = 360;
    /// From the screen's right edge and from the ridge; between two cards.
    pub const MARGIN: i32 = 8;
    pub const GAP: i32 = 8;
    pub const PADDING_X: i32 = 14;
    pub const PADDING_Y: i32 = 10;
    /// The dot left of the text, a little down, then 10 pixels to the text.
    pub const DOT: i32 = 8;
    pub const DOT_TOP: i32 = 4;
    pub const DOT_GAP: i32 = 10;
    pub const TEXT_SIZE: f32 = 12.0;
    pub const LINE: i32 = 16;
    /// The title white at 95 %, the message at 70 %.
    pub const TITLE_ALPHA: u8 = 242;
    pub const MESSAGE_ALPHA: u8 = 179;
    /// The dot: white at 45 %; a critical one rose.
    pub const DOT_ALPHA: u8 = 115;
    pub const CRITICAL: u32 = 0xf4_3f_5e;
    /// The dismiss cross: 13 pixels, white at 60 %.
    pub const CLOSE: i32 = 13;
    pub const CLOSE_ALPHA: u8 = 153;
    /// Action buttons: 11-pixel text on white at 15 %, radius 6.
    pub const BUTTON_HEIGHT: i32 = 24;
    pub const BUTTON_SIZE: f32 = 11.0;
    pub const BUTTON_ALPHA: u8 = 38;
    /// At most this many lines of the message.
    pub const MAX_LINES: usize = 4;
}

/// The settings window's measurements: Alpine Shell's settings app in its
/// dark theme (ground rgb(32, 32, 32); its surfaces and inks are mixes of
/// the ground with white, approximated here in sRGB).
pub mod settings {
    pub const WIDTH: i32 = 760;
    pub const HEIGHT: i32 = 520;
    /// The header strip and the sidebar.
    pub const HEADER: i32 = 40;
    pub const SIDEBAR: i32 = 224;
    pub const NAV_HEIGHT: i32 = 36;
    pub const NAV_GAP: i32 = 2;
    pub const NAV_PADDING: i32 = 8;
    /// Fields have radius 8, cards 12; the page's padding (the gutter).
    pub const FIELD_RADIUS: i32 = 8;
    pub const CARD_RADIUS: i32 = 12;
    pub const GUTTER: i32 = 20;
    pub const ROW_HEIGHT: i32 = 48;
    pub const ROW_PADDING: i32 = 14;
    /// Surface 0 (the ground), 1 (the page), 3 (the selected item).
    pub const SURFACE_0: u32 = 0x20_20_20;
    pub const SURFACE_1: u32 = 0x29_29_29;
    pub const SURFACE_3: u32 = 0x3f_3f_3f;
    /// Inks: primary, quiet, faint.
    pub const INK: u32 = 0xed_ed_ed;
    pub const INK_QUIET: u32 = 0x9d_9d_9d;
    pub const INK_FAINT: u32 = 0x75_75_75;
    /// Hairlines are white at 8 %, ghost fills at 6 %, slider tracks at 18 %.
    pub const HAIRLINE_ALPHA: u8 = 20;
    pub const GHOST_ALPHA: u8 = 15;
    pub const TRACK_ALPHA: u8 = 46;
    /// The toggle's knob when off.
    pub const KNOB_OFF: u32 = 0xbc_bc_bc;
    pub const TITLE_SIZE: f32 = 22.0;
    pub const LABEL_SIZE: f32 = 14.0;
    pub const DENSE_SIZE: f32 = 13.0;
    pub const SMALL_SIZE: f32 = 12.0;
    pub const EYEBROW_SIZE: f32 = 11.0;
}

/// Overlays over the windows.
pub mod overlay {
    /// The zone a dragged window would snap to: white at 12 %, outlined
    /// at 40 %, radius 10.
    pub const SNAP_ALPHA: u8 = 31;
    pub const SNAP_OUTLINE_ALPHA: u8 = 102;
    pub const SNAP_RADIUS: i32 = 10;
}
