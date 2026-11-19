//! DPI scaling at more than 100 percent (ADR 0231): the arithmetic behind the four DPI
//! awareness modes. win32k keeps one coordinate space, the monitor's physical pixels; every
//! caller works in units of its own DPI and is converted at the boundary (`wm::dpi`).
//!
//! Sheets: `specs/windows/user32.dll/SetProcessDpiAwarenessContext.yaml` (`at_200_*`),
//! `GetWindowRect.yaml` (`at_200_units`, `at_200_thread_contexts`), `GetCursorPos.yaml`,
//! `DisplayConfigSetDeviceInfo.yaml` (the scale change), `DisplayConfigGetDeviceInfo.yaml`
//! (`at_200_dpi_scale`), `shcore.dll/GetDpiForMonitor.yaml` (`scale_factor`). Learn: "High DPI
//! Desktop Application Development on Windows", "DPI_AWARENESS_CONTEXT", "WM_DPICHANGED",
//! `DEVICE_SCALE_FACTOR` (shtypes.h).

use crate::frame::Rect;

/// The scale steps of the display configuration's DPI-scale packets, in percent
/// (DisplayConfigGetDeviceInfo.yaml `at_200_dpi_scale`). Step 0 is the recommended scale.
pub const STEPS: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];

/// The smallest screen a scale may leave, in units of 96 DPI: the largest step offered on a
/// 2560x1600 display is 250 percent, on a 1920x1080 one 175 percent (DisplayConfigGetDeviceInfo.yaml).
const MIN_LOGICAL: (u32, u32) = (1024, 600);

/// The DPI of a scale in percent.
pub fn dpi_of(percent: u32) -> u32 {
    (96 * percent.max(1)).div_ceil(100)
}

/// The step of a scale (the nearest step at or below it).
pub fn step_of(percent: u32) -> usize {
    STEPS.iter().rposition(|&s| s <= percent).unwrap_or(0)
}

/// The largest step a display of `width` x `height` physical pixels offers.
pub fn max_step(width: u32, height: u32) -> usize {
    STEPS.iter().rposition(|&s| width * 100 / s >= MIN_LOGICAL.0 && height * 100 / s >= MIN_LOGICAL.1).unwrap_or(0)
}

/// `GetScaleFactorForMonitor`'s `DEVICE_SCALE_FACTOR` (shtypes.h) for a scale in percent. The
/// 200 percent step reads SCALE_180_PERCENT on the oracle's monitor (GetDpiForMonitor.yaml
/// `scale_factor`); the others are the factor of the same value.
pub fn device_scale_factor(percent: u32) -> u32 {
    const FACTORS: [u32; 16] = [100, 120, 125, 140, 150, 160, 175, 180, 200, 225, 250, 300, 350, 400, 450, 500];
    if percent == 200 {
        return 180;
    }
    FACTORS.iter().rev().find(|&&f| f <= percent).copied().unwrap_or(100)
}

/// `v * num / den`, rounded to the nearest, halves away from zero.
pub fn mul_div_round(v: i32, num: u32, den: u32) -> i32 {
    let den = i64::from(den.max(1));
    let p = i64::from(v) * i64::from(num);
    let r = if p >= 0 { (2 * p + den) / (2 * den) } else { -((-2 * p + den) / (2 * den)) };
    r.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// `v * num / den` rounded down (for sizes).
pub fn mul_div_floor(v: i32, num: u32, den: u32) -> i32 {
    let p = i64::from(v) * i64::from(num);
    p.div_euclid(i64::from(den.max(1))).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// The units a caller works in: its DPI against the monitor's (physical pixels are the units of
/// the monitor's DPI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Units {
    /// The caller's DPI: 96 for an unaware or GDI-scaled thread, the system DPI for a system
    /// aware one, the monitor's for a per-monitor aware one.
    pub caller: u32,
    /// The monitor's DPI now.
    pub monitor: u32,
}

impl Units {
    pub const fn new(caller: u32, monitor: u32) -> Units {
        Units { caller, monitor }
    }

    /// The caller works in physical pixels.
    pub fn identity(&self) -> bool {
        self.caller == self.monitor
    }

    /// A physical coordinate in the caller's units.
    pub fn to_caller(&self, v: i32) -> i32 {
        if self.identity() { v } else { mul_div_round(v, self.caller, self.monitor) }
    }

    /// A coordinate of the caller in physical pixels.
    pub fn to_phys(&self, v: i32) -> i32 {
        if self.identity() { v } else { mul_div_round(v, self.monitor, self.caller) }
    }

    /// A physical size (a width, a height) in the caller's units: rounded down, so a window's
    /// client area never reads larger than its window (GetWindowRect.yaml `at_200_thread_contexts`:
    /// a 374x229 client area reads 187x114).
    pub fn size_to_caller(&self, v: i32) -> i32 {
        if self.identity() { v } else { mul_div_floor(v, self.caller, self.monitor) }
    }

    /// A physical rectangle in the caller's units: the origin rounded, the size rounded down.
    pub fn rect_to_caller(&self, r: Rect) -> Rect {
        if self.identity() {
            return r;
        }
        let (l, t) = (self.to_caller(r.left), self.to_caller(r.top));
        Rect::new(l, t, l + self.size_to_caller(r.right - r.left), t + self.size_to_caller(r.bottom - r.top))
    }

    /// A rectangle of the caller in physical pixels (each edge rounded).
    pub fn rect_to_phys(&self, r: Rect) -> Rect {
        if self.identity() {
            return r;
        }
        Rect::new(self.to_phys(r.left), self.to_phys(r.top), self.to_phys(r.right), self.to_phys(r.bottom))
    }

    /// A packed `MAKELPARAM` point of 16-bit signed halves, physical to the caller's units.
    pub fn lparam_to_caller(&self, l: u64) -> u64 {
        if self.identity() {
            return l;
        }
        let x = self.to_caller(i32::from(l as u16 as i16));
        let y = self.to_caller(i32::from((l >> 16) as u16 as i16));
        (l & !0xFFFF_FFFF) | u64::from(x as u16) | (u64::from(y as u16) << 16)
    }
}

/// The window size `WM_DPICHANGED` suggests at `new` DPI for a window of `size` at `old` DPI:
/// each side scaled and rounded (DisplayConfigSetDeviceInfo.yaml: 400x300 at 192 is 350x263 at
/// 168; 250x120 is 219x105).
pub fn scaled_size(size: (i32, i32), new: u32, old: u32) -> (i32, i32) {
    (mul_div_round(size.0, new, old), mul_div_round(size.1, new, old))
}

/// The suggested size of a window across several changes: Windows comes back to exactly the
/// first size when the scale comes back (350x263 at 168 is 400x300 again at 192, not 400x301), so
/// the size is scaled from the one the window had before the first change, as long as the window
/// kept the size suggested since.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SuggestBase {
    /// The DPI and size the scaling starts from.
    pub dpi: u32,
    pub size: (i32, i32),
    /// The size suggested last.
    pub last: (i32, i32),
}

/// The size to suggest for a window of `size` at `old` DPI going to `new`, and the base to keep.
pub fn suggest(base: Option<SuggestBase>, size: (i32, i32), new: u32, old: u32) -> ((i32, i32), SuggestBase) {
    let b = match base {
        Some(b) if b.last == size && b.dpi != 0 => b,
        _ => SuggestBase { dpi: old, size, last: size },
    };
    let s = scaled_size(b.size, new, b.dpi);
    (s, SuggestBase { last: s, ..b })
}

/// Where a window the system scales (unaware, GDI-scaled or system aware) moves when the monitor's
/// DPI changes from `old` to `new`: its visible frame keeps its physical place. `inv` is the
/// invisible part of its left border in its own units (the frame less one pixel: 7 at 96 DPI and
/// 12 at 192 for an overlapped window, 0 for a popup); the top has none. The size in its own
/// units stays (DisplayConfigSetDeviceInfo.yaml `system_aware`, `unaware`).
pub fn virtualized_move(left: i32, top: i32, inv: i32, old: u32, new: u32) -> (i32, i32) {
    (mul_div_round(left + inv, old, new) - inv, mul_div_round(top, old, new))
}

/// The session the kernel's arguments describe (`poly.win32k.display=WxH`,
/// `poly.win32k.scale=P`; tools/pipeline's `session_kernel_args`): the physical screen and the
/// scale in percent; `None` for an argument that does not parse.
pub fn parse_display(display: Option<&str>, scale: Option<&str>) -> Option<((i32, i32), u32)> {
    let (w, h) = match display {
        Some(d) => {
            let (w, h) = d.split_once('x')?;
            (w.parse::<i32>().ok()?, h.parse::<i32>().ok()?)
        }
        None => (1920, 1080),
    };
    let p = match scale {
        Some(s) => s.parse::<u32>().ok()?,
        None => 100,
    };
    ((64..=16384).contains(&w) && (64..=16384).contains(&h) && (100..=500).contains(&p)).then_some(((w, h), p))
}

/// The text of the first element named `name` (any namespace prefix) of a manifest.
fn element_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let mut at = 0;
    while let Some(i) = xml[at..].find(name) {
        let start = at + i;
        at = start + name.len();
        let before = xml[..start].chars().next_back();
        let after = xml[at..].chars().next();
        if !matches!(before, Some('<' | ':')) || !after.is_some_and(|c| c == '>' || c.is_ascii_whitespace()) {
            continue;
        }
        // An opening tag: its text runs from its '>' to the next '<'.
        if xml[..start].trim_end_matches(':').trim_end_matches(|c: char| c.is_ascii_alphanumeric()).ends_with("</") {
            continue;
        }
        let open_end = at + xml[at..].find('>')?;
        if xml[..open_end].ends_with('/') {
            return Some("");
        }
        let text_end = open_end + 1 + xml[open_end + 1..].find('<')?;
        return Some(xml[open_end + 1..text_end].trim());
    }
    None
}

/// The DPI awareness an executable's manifest declares (`guic::AWARE_*`; Learn, "Application
/// manifests" `dpiAware`, `dpiAwareness`; SetProcessDpiAwarenessContext.yaml `manifest`):
/// `dpiAwareness` is a list whose first known word counts and wins over `dpiAware` (`true`:
/// system, `true/pm` and `per monitor`: per-monitor, `false`: unaware).
pub fn manifest_awareness(manifest: &[u8]) -> Option<u8> {
    use crate::guic::*;
    let xml = core::str::from_utf8(manifest).ok()?;
    if let Some(list) = element_text(xml, "dpiAwareness") {
        for word in list.split(',') {
            let w = word.trim().to_ascii_lowercase();
            let a = match w.as_str() {
                "unaware" => AWARE_UNAWARE,
                "system" => AWARE_SYSTEM,
                "permonitor" => AWARE_PER_MONITOR,
                "permonitorv2" => AWARE_PER_MONITOR_V2,
                _ => continue,
            };
            return Some(a);
        }
    }
    let v = element_text(xml, "dpiAware")?.to_ascii_lowercase();
    match v.as_str() {
        "true" => Some(AWARE_SYSTEM),
        "false" => Some(AWARE_UNAWARE),
        "true/pm" | "per monitor" => Some(AWARE_PER_MONITOR),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_declare_an_awareness() {
        use crate::guic::*;
        let m = |inner: &str| {
            alloc::format!(
                "<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\"><application xmlns=\"urn:schemas-microsoft-com:asm.v3\"><windowsSettings>{inner}</windowsSettings></application></assembly>"
            )
        };
        let dpi_aware = |v: &str| {
            m(&alloc::format!(
                "<dpiAware xmlns=\"http://schemas.microsoft.com/SMI/2005/WindowsSettings\">{v}</dpiAware>"
            ))
        };
        let awareness = |v: &str| {
            alloc::format!(
                "<dpiAwareness xmlns=\"http://schemas.microsoft.com/SMI/2016/WindowsSettings\">{v}</dpiAwareness>"
            )
        };
        assert_eq!(manifest_awareness(dpi_aware("false").as_bytes()), Some(AWARE_UNAWARE));
        assert_eq!(manifest_awareness(dpi_aware("true").as_bytes()), Some(AWARE_SYSTEM));
        assert_eq!(manifest_awareness(dpi_aware("True/PM").as_bytes()), Some(AWARE_PER_MONITOR));
        assert_eq!(
            manifest_awareness(m(&awareness("PerMonitorV2, PerMonitor")).as_bytes()),
            Some(AWARE_PER_MONITOR_V2)
        );
        assert_eq!(manifest_awareness(m(&awareness("bogus, system")).as_bytes()), Some(AWARE_SYSTEM));
        assert_eq!(manifest_awareness(m(&awareness("permonitor")).as_bytes()), Some(AWARE_PER_MONITOR));
        let both = m(&alloc::format!("<dpiAware>true/pm</dpiAware>{}", awareness("system")));
        assert_eq!(manifest_awareness(both.as_bytes()), Some(AWARE_SYSTEM));
        let prefixed = m("<ws2:dpiAwareness>PerMonitorV2</ws2:dpiAwareness>");
        assert_eq!(manifest_awareness(prefixed.as_bytes()), Some(AWARE_PER_MONITOR_V2));
        assert_eq!(manifest_awareness(m("").as_bytes()), None);
        assert_eq!(manifest_awareness(b"<assembly/>"), None);
    }

    #[test]
    fn rounding_is_to_the_nearest_with_halves_away_from_zero() {
        assert_eq!(mul_div_round(1599, 96, 192), 800);
        assert_eq!(mul_div_round(113, 96, 192), 57);
        assert_eq!(mul_div_round(-3, 1, 2), -2);
        assert_eq!(mul_div_round(2560, 192, 168), 2926);
        assert_eq!(mul_div_round(1600, 192, 168), 1829);
        assert_eq!(mul_div_round(1280 * 2, 96, 168), 1463);
        assert_eq!(mul_div_round(800 * 2, 96, 168), 914);
        assert_eq!(mul_div_floor(229, 96, 192), 114);
        assert_eq!(mul_div_floor(-1, 1, 2), -1);
    }

    #[test]
    fn an_unaware_caller_at_200_percent_sees_half_the_pixels() {
        let u = Units::new(96, 192);
        assert!(!u.identity());
        // The screen and its work area (GetMonitorInfoW.yaml at_200_values).
        assert_eq!((u.to_caller(2560), u.to_caller(1600), u.to_caller(1504)), (1280, 800, 752));
        // A window of an aware thread as an unaware one reads it (GetWindowRect.yaml).
        assert_eq!(u.rect_to_caller(Rect::new(100, 100, 500, 400)), Rect::new(50, 50, 250, 200));
        assert_eq!(u.rect_to_caller(Rect::new(113, 158, 113 + 374, 158 + 229)), Rect::new(57, 79, 57 + 187, 79 + 114));
        // Its own window, doubled and back.
        let r = Rect::new(100, 100, 500, 400);
        assert_eq!(u.rect_to_phys(r), Rect::new(200, 200, 1000, 800));
        assert_eq!(u.rect_to_caller(u.rect_to_phys(r)), r);
        // The cursor clamped to the last physical pixel (GetCursorPos.yaml at_200_units).
        assert_eq!((u.to_caller(2559), u.to_caller(1599)), (1280, 800));
        let l = u.lparam_to_caller(184 | (88 << 16));
        assert_eq!((l & 0xFFFF, l >> 16), (92, 44));
        assert!(Units::new(192, 192).identity());
        assert_eq!(Units::new(96, 96).rect_to_caller(r), r);
    }

    #[test]
    fn whole_coordinates_survive_the_trip_through_a_finer_physical_space() {
        // At 175 percent an unaware unit is 1.75 physical pixels: every coordinate comes back.
        let u = Units::new(96, 168);
        for v in -300..3000 {
            assert_eq!(u.to_caller(u.to_phys(v)), v, "{v}");
        }
    }

    #[test]
    fn suggested_sizes_scale_from_the_first_size() {
        assert_eq!(scaled_size((400, 300), 168, 192), (350, 263));
        assert_eq!(scaled_size((250, 120), 168, 192), (219, 105));
        let (s, b) = suggest(None, (400, 300), 168, 192);
        assert_eq!(s, (350, 263));
        let (s, b) = suggest(Some(b), (350, 263), 192, 168);
        assert_eq!(s, (400, 300));
        assert_eq!(b.last, (400, 300));
        // A window resized in between starts again from its size.
        let (s, _) = suggest(Some(b), (500, 300), 168, 192);
        assert_eq!(s, (438, 263));
        let (s, _) = suggest(None, (219, 105), 192, 168);
        assert_eq!(s, (250, 120));
    }

    #[test]
    fn windows_the_system_scales_keep_their_visible_frame_in_place() {
        // DisplayConfigSetDeviceInfo.yaml: unaware 100,100 -> 115,114 -> 100,100; system aware
        // 100,100 -> 116,114 -> 100,100; a popup 600,150 -> 686,171 -> 600,150.
        assert_eq!(virtualized_move(100, 100, 7, 192, 168), (115, 114));
        assert_eq!(virtualized_move(115, 114, 7, 168, 192), (100, 100));
        assert_eq!(virtualized_move(100, 100, 12, 192, 168), (116, 114));
        assert_eq!(virtualized_move(116, 114, 12, 168, 192), (100, 100));
        assert_eq!(virtualized_move(600, 150, 0, 192, 168), (686, 171));
        assert_eq!(virtualized_move(686, 171, 0, 168, 192), (600, 150));
    }

    #[test]
    fn the_steps_of_a_display() {
        assert_eq!(dpi_of(200), 192);
        assert_eq!(dpi_of(175), 168);
        assert_eq!(dpi_of(100), 96);
        assert_eq!(step_of(200), 4);
        assert_eq!(step_of(175), 3);
        assert_eq!(max_step(2560, 1600), 6);
        assert_eq!(max_step(1920, 1080), 3);
        assert_eq!(max_step(1024, 768), 0);
        assert_eq!(device_scale_factor(200), 180);
        assert_eq!(device_scale_factor(100), 100);
        assert_eq!(device_scale_factor(175), 175);
        assert_eq!(device_scale_factor(125), 125);
    }

    #[test]
    fn the_kernel_arguments_of_a_session() {
        assert_eq!(parse_display(Some("2560x1600"), Some("200")), Some(((2560, 1600), 200)));
        assert_eq!(parse_display(None, None), Some(((1920, 1080), 100)));
        assert_eq!(parse_display(Some("2560x"), Some("200")), None);
        assert_eq!(parse_display(Some("2560x1600"), Some("50")), None);
    }
}
