//! The shell's measurements and colours, in logical pixels: the look of
//! the owner's Alpine Shell re-created (ADR 0127 lists where each value
//! comes from: its components' Tailwind classes at 4 pixels a unit, its
//! `@alpine/ui` tokens, and its screenshots). Colours are `0xRRGGBB`;
//! "white at a" is white drawn with coverage `a` over what is below, as
//! Alpine's `white/10`-style fills.

/// Alpine's fonts (ADR 0127): Geist for everything, Averia Serif Libre
/// for display titles, Geist Mono for key hints.
pub use crate::draw::{Face, Style};

/// The ridge: full width, flat, opaque (`topBar.tsx`, `faces/*.tsx`).
pub mod ridge {
    pub const HEIGHT: i32 = 24;
    /// Alpine's default bar colour, rgb(32, 32, 32)
    /// (`electron/modules/topbarColorDefaults.js`).
    pub const BACKGROUND: u32 = 0x20_20_20;
    pub const TEXT: u32 = 0xff_ff_ff;
    pub const TEXT_SIZE: f32 = 12.0;
    /// Pills are 18 pixels (`h-4.5`), 8 pixels of padding (`px-2`), 4
    /// pixels apart (`gap-1`).
    pub const PILL_HEIGHT: i32 = 18;
    pub const PILL_PADDING: i32 = 8;
    pub const GAP: i32 = 4;
    /// The right group's margin from the screen's edge (`mr-1`).
    pub const RIGHT_MARGIN: i32 = 4;
    /// The status pill's fill (`bg-white/10`), and a hovered clock's.
    pub const PILL_ALPHA: u8 = 26;
    pub const ICON: i32 = 14;
    /// The system menu's dashed circle, in a 20-pixel box with 8 pixels of
    /// padding (`px-2 h-5`).
    pub const MENU_ICON_X: i32 = 8;
    /// The notice face in the middle: a 6-pixel dot, 6 pixels to 11-pixel
    /// text at 75 % (`faces/noticeFace.tsx`).
    pub const NOTICE_DOT: i32 = 6;
    pub const NOTICE_GAP: i32 = 6;
    pub const NOTICE_SIZE: f32 = 11.0;
    pub const NOTICE_ALPHA: u8 = 191;
    pub const NOTICE_DOT_ALPHA: u8 = 115;
}

/// Surfaces that hang from the ridge (`chrome/chromeShape.ts`,
/// `ChromeCanvas.tsx`, `shared/popupPlacement.ts`).
pub mod chrome {
    pub const SHOULDER: i32 = 14;
    pub const RADIUS: i32 = 14;
    /// A surface stays this far from the screen's sides, else it is put
    /// against the edge without a shoulder there.
    pub const SCREEN_PAD: i32 = 6;
    /// Opening: 460 ms, easeOutExpo; closing: 300 ms, easeInCubic; one
    /// surface turning into another: 400 ms, easeOutExpo.
    pub const OPEN_MS: u64 = 460;
    pub const CLOSE_MS: u64 = 300;
    pub const MORPH_MS: u64 = 400;
    /// How often the shell asks for a frame while something moves.
    pub const FRAME_MS: u64 = 16;
}

/// Alpine's white-on-dark text and fills (`launcher/launcherColors.ts`).
pub mod ink {
    /// `text-white/50`, `/25`, `/40` (icons), `/60` (chips).
    pub const DIM: u8 = 128;
    pub const FAINT: u8 = 64;
    pub const ICON: u8 = 102;
    pub const CHIP_TEXT: u8 = 153;
    /// `bg-white/5` (hover), `/10` (selected, dividers, chips).
    pub const HOVER: u8 = 13;
    pub const SELECTED: u8 = 26;
}

/// The launcher (`launcher/launcherPanel.tsx`, `searchBar.tsx`,
/// `resultList.tsx`, `resultRow.tsx`, `launcherFooter.tsx`,
/// `launcherSizing.ts`): it hangs from the ridge's middle.
pub mod launcher {
    pub const WIDTH: i32 = 720;
    /// Never closer than this to the screen's sides.
    pub const SIDE_MARGIN: i32 = 16;
    pub const SEARCH_HEIGHT: i32 = 48;
    pub const SEARCH_PADDING: i32 = 16;
    pub const SEARCH_GAP: i32 = 8;
    pub const SEARCH_ICON: i32 = 16;
    /// The search glyph at 60 % opacity.
    pub const SEARCH_ICON_ALPHA: u8 = 153;
    pub const INPUT_SIZE: f32 = 15.0;
    /// Alpine searches apps, windows and settings; Poly apps and settings.
    pub const PLACEHOLDER: &str = "Search apps and settings";
    /// Placeholder text: white at 40 %.
    pub const PLACEHOLDER_ALPHA: u8 = 102;
    /// Dividers: white at 10 %.
    pub const DIVIDER_ALPHA: u8 = 26;
    /// The list's padding (`px-1.5 pb-1.5`).
    pub const LIST_PADDING: i32 = 6;
    /// A section title: 11 px medium at 50 %, `px-2.5 pt-2 pb-1`, then the
    /// list's 2-pixel gap (31 in all, as the live Alpine Shell measures).
    pub const SECTION_HEIGHT: i32 = 31;
    pub const SECTION_SIZE: f32 = 11.0;
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
    /// Glyph icons: 18 pixels at 40 % in the 28-pixel box.
    pub const GLYPH: i32 = 18;
    pub const TITLE_SIZE: f32 = 13.0;
    pub const SUBTITLE_SIZE: f32 = 11.0;
    pub const SUBTITLE_ALPHA: u8 = 128;
    pub const FOOTER_HEIGHT: i32 = 36;
    pub const FOOTER_SIZE: f32 = 11.0;
    /// Key chips: 10 px on white at 10 %, text at 60 %, radius 4,
    /// `px-1.5 py-px`, 16-pixel lines.
    pub const CHIP_SIZE: f32 = 10.0;
    pub const CHIP_HEIGHT: i32 = 18;
    /// Rows shown at most (Alpine's default of 8).
    pub const MAX_ROWS: usize = 8;
    /// The list is at most 560 pixels, or 70 % of the screen.
    pub const LIST_CAP: i32 = 560;
    pub const LIST_SHARE_PERCENT: i32 = 70;
}

/// The desktop behind the windows (ADR 0127): Alpine Shell's two bundled
/// pictures, Grainy Valley the default, then gradients. The colours are
/// what is drawn without the pictures: Grainy Valley falls back to a drawn
/// valley, the others to a gradient from `top` to `bottom` with a soft
/// light of `glow`.
pub mod wallpaper {
    /// The default.
    pub const DEFAULT: &str = "grainy-valley";

    /// (ID, label, top, bottom, glow).
    pub const PRESETS: &[(&str, &str, u32, u32, u32)] = &[
        ("grainy-valley", "Grainy Valley", 0x9a_9c_9e, 0x1a_1a_19, 0xd8_d8_d6),
        ("quiet-moment", "Quiet Moment", 0xb8_a4_b0, 0x2a_36_2c, 0xe6_c4_bc),
        ("slate", "Slate", 0x2c_3e_50, 0x10_16_1d, 0x4a_6a_82),
        ("dusk", "Dusk", 0x3b_2a_4a, 0x14_0f_1c, 0x7a_5a_8c),
        ("forest", "Forest", 0x24_44_3a, 0x0d_17_14, 0x4f_7f_6a),
        ("graphite", "Graphite", 0x3a_3a_3a, 0x12_12_12, 0x5e_5e_5e),
    ];

    /// The colours (top, bottom, glow) of preset `id` (the default's if
    /// unknown).
    pub fn colours(id: &str) -> (u32, u32, u32) {
        let p = PRESETS.iter().find(|p| p.0 == id).unwrap_or(&PRESETS[0]);
        (p.2, p.3, p.4)
    }
}

/// Notifications: Alpine Shell's notice strip (`topbar/noticeStrip.tsx`),
/// hanging from the ridge at the top right, newest first.
pub mod notice {
    pub const WIDTH: i32 = 360;
    /// Alpine's `px-3.5 py-2.5`, `gap-2.5`.
    pub const PADDING_X: i32 = 14;
    pub const PADDING_Y: i32 = 10;
    /// The dot left of the text (`size-2 mt-1.5`), then 10 pixels to the
    /// text.
    pub const DOT: i32 = 8;
    pub const DOT_TOP: i32 = 6;
    pub const DOT_GAP: i32 = 10;
    pub const TEXT_SIZE: f32 = 12.0;
    pub const LINE: i32 = 16;
    /// The title white at 95 %, the message at 70 %.
    pub const TITLE_ALPHA: u8 = 242;
    pub const MESSAGE_ALPHA: u8 = 179;
    /// The dot: white at 45 %; rose for a critical one (`bg-rose-500`).
    pub const DOT_ALPHA: u8 = 115;
    pub const CRITICAL: u32 = 0xf4_3f_5e;
    /// The dismiss cross: 13-pixel text at 60 % (`px-1.5 py-0.5`).
    pub const CLOSE: i32 = 13;
    pub const CLOSE_ALPHA: u8 = 153;
    /// Action buttons: 11 px on white at 15 %, radius 6, `px-2 py-1`,
    /// 8 pixels below the text (`mt-2`).
    pub const BUTTON_HEIGHT: i32 = 24;
    pub const BUTTON_SIZE: f32 = 11.0;
    pub const BUTTON_ALPHA: u8 = 38;
    /// At most this many lines of the message.
    pub const MAX_LINES: usize = 4;
    /// Between two notices in the strip: a divider.
    pub const DIVIDER_ALPHA: u8 = 26;
}

/// The panels that open from the ridge.
pub mod panel {
    /// The system menu (`systemMenu.tsx`): `p-1.5`, rows `px-2 py-1` of
    /// 12-pixel text at 80 % with 12-pixel icons 10 pixels before it.
    pub const MENU_PADDING: i32 = 6;
    pub const MENU_ROW: i32 = 26;
    pub const MENU_ICON: i32 = 12;
    pub const MENU_SIZE: f32 = 12.0;
    pub const MENU_ALPHA: u8 = 204;
    pub const MENU_MIN_WIDTH: i32 = 140;
    /// The control centre (`status/controlCenter.tsx`): 260 wide, `p-3`,
    /// `gap-3`.
    pub const CONTROL_WIDTH: i32 = 260;
    pub const CONTROL_PADDING: i32 = 12;
    pub const CONTROL_GAP: i32 = 12;
    /// Its rows: a 17-pixel icon 4 pixels in, 12 pixels to the label (12 px
    /// over a 13 px value at 60 %), a 15-pixel chevron.
    pub const ITEM_ICON: i32 = 17;
    pub const ITEM_HEIGHT: i32 = 33;
    pub const CHEVRON: i32 = 15;
    pub const DIM_ALPHA: u8 = 153;
    /// The volume slider (`status/controlSlider.tsx`): on `#3F3F3F` at
    /// 20 %, `p-3`, radius 6, a 15-pixel icon, a 10-pixel track (radius
    /// 8) white over white at 20 %.
    pub const SLIDER_FILL: u32 = 0x3f_3f_3f;
    pub const SLIDER_FILL_ALPHA: u8 = 51;
    pub const SLIDER_HEIGHT: i32 = 39;
    pub const SLIDER_TRACK: i32 = 10;
    pub const SLIDER_REST_ALPHA: u8 = 51;
    /// The settings button: 24 pixels, a 15-pixel gear.
    pub const BUTTON: i32 = 24;
    /// The calendar (`calendar/calendarFlyout.tsx`): one day, a 46-pixel
    /// gutter and a 264-pixel column; 44 pixels an hour in a 416-pixel
    /// window onto the day.
    pub const CAL_GUTTER: i32 = 46;
    pub const CAL_COLUMN: i32 = 264;
    pub const CAL_HOUR: i32 = 44;
    pub const CAL_SCROLL: i32 = 416;
    pub const CAL_HEADER: i32 = 40;
    pub const CAL_DAY: i32 = 52;
    pub const CAL_ALL_DAY: i32 = 27;
    /// Lines between days and under the header (white at 12 %), hour lines
    /// (at 10 %), the now line (`red-500`).
    pub const CAL_BORDER_ALPHA: u8 = 31;
    pub const CAL_LINE_ALPHA: u8 = 26;
    pub const CAL_NOW: u32 = 0xef_44_44;
    /// The notification history: Poly's (Alpine has none), in the style
    /// of its lists.
    pub const HISTORY_WIDTH: i32 = 360;
}

/// The settings window: Alpine Shell's settings app in its dark theme, as
/// its screenshots show it (`components/settings/settingsApp.tsx`,
/// `@alpine/ui`).
pub mod settings {
    pub const WIDTH: i32 = 900;
    pub const HEIGHT: i32 = 620;
    /// The header row (the title, the arrows, where we are) and the
    /// sidebar (`w-56`).
    pub const HEADER: i32 = 40;
    pub const SIDEBAR: i32 = 224;
    /// Sidebar rows (`h-9`, `gap-0.5`, `px-2`, inside `px-3`, 15-pixel
    /// icons 10 pixels before the text, radius 8).
    pub const NAV_HEIGHT: i32 = 36;
    pub const NAV_GAP: i32 = 2;
    pub const NAV_PADDING: i32 = 8;
    pub const NAV_INSET: i32 = 12;
    pub const NAV_ICON: i32 = 15;
    /// The search field (`h-8`, radius 8, a hairline), 4 pixels above the
    /// first row.
    pub const SEARCH_HEIGHT: i32 = 32;
    /// A section label in the sidebar starts 18 pixels after the row above
    /// and the next row 36 pixels after it.
    pub const EYEBROW_GAP: i32 = 36;
    /// Fields have radius 8, cards 12; the page's padding (the gutter).
    pub const FIELD_RADIUS: i32 = 8;
    pub const CARD_RADIUS: i32 = 12;
    pub const GUTTER: i32 = 20;
    pub const ROW_HEIGHT: i32 = 48;
    pub const ROW_PADDING: i32 = 14;
    /// Rows with a description under the label are 62 pixels apart.
    pub const ROW_PITCH: i32 = 62;
    /// Surfaces 0 (the ground), 1 (the page), 2 (fields, cards), 3 (the
    /// selected item); the strong line.
    pub const SURFACE_0: u32 = 0x1a_1a_19;
    pub const SURFACE_1: u32 = 0x21_21_20;
    pub const SURFACE_2: u32 = 0x29_29_28;
    pub const SURFACE_3: u32 = 0x33_3332;
    pub const LINE: u32 = 0x3b_3b_39;
    /// Inks: primary, quiet, faint.
    pub const INK: u32 = 0xeb_eb_e7;
    pub const INK_QUIET: u32 = 0x9a_9a_95;
    pub const INK_FAINT: u32 = 0x6f_6f_6b;
    /// The breadcrumb's underline.
    pub const UNDERLINE: u32 = 0x4b_4b_49;
    /// Ghost fills are white at 6 %, slider and toggle tracks at 18 %.
    pub const GHOST_ALPHA: u8 = 15;
    pub const TRACK_ALPHA: u8 = 46;
    /// The toggle's knob when off (the ground mixed with white, 70 %).
    pub const KNOB_OFF: u32 = 0xb8_b8_b2;
    /// The page title (Averia Serif Libre, `text-2xl`), the sidebar's
    /// title, section headings (14 px semibold), labels (14 px),
    /// descriptions (12 px), dense text (13 px), eyebrows (11 px semibold,
    /// 5 % letter spacing).
    pub const TITLE_SIZE: f32 = 22.0;
    /// Hairlines between rows: white at 8 %.
    pub const HAIRLINE_ALPHA: u8 = 20;
    pub const APP_TITLE_SIZE: f32 = 16.0;
    pub const HEADING_SIZE: f32 = 14.0;
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
