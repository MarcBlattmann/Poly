//! The shell's settings (ADR 0117) and the file they persist in: display
//! (resolution, scale), keyboard layout, wallpaper, sound (volume, mute)
//! and time zone.
//!
//! The file is lines of `key=value` with `#` comments, written whole and
//! renamed into place by the compositor (`$XDG_CONFIG_HOME/poly/shell.conf`
//! on the root disk). Reading is forgiving: a value that does not parse
//! keeps the default, an unknown key is kept as it was and written back,
//! so a newer shell's settings survive an older one.

use crate::theme::wallpaper::PRESETS;
use crate::tz::{TimeZone, ZONES};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The keyboard layouts offered (`poly-input`'s names) and their labels.
pub const KEYBOARDS: &[(&str, &str)] = &[("us", "English (US)"), ("de-ch", "German (Switzerland)"), ("de", "German")];
/// The output scales offered.
pub const SCALES: &[u32] = &[1, 2];

/// The settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The output's integer scale.
    pub scale: u32,
    /// The display mode asked for (`None`: the display's preferred one).
    pub resolution: Option<(u32, u32)>,
    pub keyboard: String,
    /// A [`PRESETS`] ID.
    pub wallpaper: String,
    /// The master volume, 0 to 100.
    pub volume: u32,
    pub muted: bool,
    /// A [`ZONES`] name or a POSIX `TZ` rule.
    pub time_zone: String,
    /// Keys this version does not know, kept as they were.
    pub extra: Vec<(String, String)>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            scale: 1,
            resolution: None,
            keyboard: "us".into(),
            wallpaper: PRESETS[0].0.into(),
            volume: 80,
            muted: false,
            time_zone: "UTC".into(),
            extra: Vec::new(),
        }
    }
}

/// One setting changed (what the settings window and the hotkeys report,
/// and what the compositor applies).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Scale(u32),
    Resolution(u32, u32),
    Keyboard(String),
    Wallpaper(String),
    Volume(u32),
    Muted(bool),
    TimeZone(String),
}

impl Settings {
    /// Read the file's text (anything not understood keeps its default).
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            let known = match key {
                "scale" => value.parse().ok().filter(|v| SCALES.contains(v)).map(|v| s.scale = v).is_some(),
                "resolution" => {
                    let r = value.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                    s.resolution = r.filter(|&(w, h): &(u32, u32)| w >= 320 && h >= 200 && w <= 8192 && h <= 8192);
                    true
                }
                "keyboard" => {
                    if KEYBOARDS.iter().any(|k| k.0 == value) {
                        s.keyboard = value.into();
                    }
                    true
                }
                "wallpaper" => {
                    if PRESETS.iter().any(|p| p.0 == value) {
                        s.wallpaper = value.into();
                    }
                    true
                }
                "volume" => value.parse().ok().filter(|v| *v <= 100).map(|v| s.volume = v).is_some(),
                "muted" => {
                    s.muted = value == "true";
                    true
                }
                "timezone" => {
                    if TimeZone::named(value).is_some() {
                        s.time_zone = value.into();
                    }
                    true
                }
                _ => false,
            };
            if !known && !matches!(key, "scale" | "volume") {
                s.extra.push((key.into(), value.into()));
            }
        }
        s
    }

    /// The file's text.
    pub fn to_text(&self) -> String {
        let mut out = String::from("# Poly's shell settings (ADR 0117), written by the compositor.\n");
        out += &format!("scale={}\n", self.scale);
        if let Some((w, h)) = self.resolution {
            out += &format!("resolution={w}x{h}\n");
        }
        out += &format!("keyboard={}\n", self.keyboard);
        out += &format!("wallpaper={}\n", self.wallpaper);
        out += &format!("volume={}\n", self.volume);
        out += &format!("muted={}\n", self.muted);
        out += &format!("timezone={}\n", self.time_zone);
        for (k, v) in &self.extra {
            out += &format!("{k}={v}\n");
        }
        out
    }

    /// Apply a change; true if anything changed.
    pub fn apply(&mut self, c: &Change) -> bool {
        let before = self.clone();
        match c {
            Change::Scale(s) if SCALES.contains(s) => self.scale = *s,
            Change::Resolution(w, h) => self.resolution = Some((*w, *h)),
            Change::Keyboard(k) if KEYBOARDS.iter().any(|x| x.0 == k) => self.keyboard = k.clone(),
            Change::Wallpaper(w) if PRESETS.iter().any(|p| p.0 == w) => self.wallpaper = w.clone(),
            Change::Volume(v) => self.volume = (*v).min(100),
            Change::Muted(m) => self.muted = *m,
            Change::TimeZone(z) if TimeZone::named(z).is_some() => self.time_zone = z.clone(),
            _ => {}
        }
        *self != before
    }

    /// The time zone in effect (UTC if the setting does not parse).
    pub fn zone(&self) -> TimeZone {
        TimeZone::named(&self.time_zone).or_else(|| TimeZone::named("UTC")).expect("UTC parses")
    }

    /// The label of the time zone setting.
    pub fn zone_label(&self) -> String {
        ZONES.iter().find(|z| z.0 == self.time_zone).map_or_else(|| self.time_zone.clone(), |z| z.1.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings {
            scale: 2,
            resolution: Some((1920, 1080)),
            keyboard: "de-ch".into(),
            wallpaper: "forest".into(),
            volume: 35,
            muted: true,
            time_zone: "Europe/Zurich".into(),
            extra: alloc::vec![("future".into(), "yes".into())],
        };
        let text = s.to_text();
        assert!(text.contains("keyboard=de-ch\n") && text.contains("resolution=1920x1080\n"));
        assert_eq!(Settings::parse(&text), s);
    }

    #[test]
    fn forgiving_reading() {
        let s = Settings::parse(
            "# c\nscale=3\nvolume=150\nkeyboard=fr\nwallpaper=nope\ntimezone=Mars/X\nmuted=yes\nvolume\nunknown = 1\nresolution=10x10\n",
        );
        let d = Settings::default();
        assert_eq!((s.scale, s.volume, s.keyboard.as_str()), (d.scale, d.volume, "us"));
        assert_eq!((s.wallpaper.as_str(), s.time_zone.as_str(), s.muted), (d.wallpaper.as_str(), "UTC", false));
        assert_eq!(s.resolution, None);
        assert_eq!(s.extra, [("unknown".into(), "1".into())]);
        assert_eq!(Settings::parse(""), d);
        // A raw POSIX rule is a valid time zone.
        assert_eq!(Settings::parse("timezone=<+03>-3").zone().offset_minutes(0), 180);
    }

    #[test]
    fn changes() {
        let mut s = Settings::default();
        assert!(s.apply(&Change::Volume(30)));
        assert!(!s.apply(&Change::Volume(30)), "no change");
        assert!(s.apply(&Change::Volume(130)));
        assert_eq!(s.volume, 100);
        assert!(s.apply(&Change::Keyboard("de".into())));
        assert!(!s.apply(&Change::Keyboard("xx".into())));
        assert!(!s.apply(&Change::Scale(5)));
        assert!(s.apply(&Change::TimeZone("Asia/Tokyo".into())));
        assert_eq!(s.zone_label(), "Tokyo");
        assert!(s.apply(&Change::Wallpaper("dusk".into())));
    }
}
