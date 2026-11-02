//! Keyboard shortcuts (ADR 0128), as Alpine Shell keeps them
//! (`electron/modules/shellShortcutBindings.js`, `shellShortcuts.js`): one
//! keymap of actions to key combinations, the defaults and the user's
//! changes to them, recording a new combination (the first key that is
//! not a modifier ends it, Escape alone cancels), refusing one that has no
//! modifier, lacks the arrow an arrow binding needs, is kept by the system
//! or is already taken (Alpine refuses rather than swaps), and matching the
//! keys the compositor sees against it.
//!
//! Keys are named as the keyboard layout types them, so a binding follows
//! the layout as Windows' virtual keys do: `Win+Z` is the key that types Z,
//! wherever the layout puts it. Names are Alpine's labels ("Space", "F4",
//! "Page Up", "←"); the settings file stores a combination as its names
//! joined by `+` (`Ctrl+Space`, `Win+Arrows`; a modifier alone means
//! tapping it).

use crate::wm::Arrow;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_input::key::{Key, Modifiers};
use poly_input::keysym::keysym;
use poly_input::layout::Layout;

/// Modifiers held with a key: Win, Ctrl, Alt, Shift.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods(pub u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const WIN: Mods = Mods(1);
    pub const CTRL: Mods = Mods(2);
    pub const ALT: Mods = Mods(4);
    pub const SHIFT: Mods = Mods(8);

    pub const fn union(self, o: Mods) -> Mods {
        Mods(self.0 | o.0)
    }

    pub const fn has(self, o: Mods) -> bool {
        self.0 & o.0 == o.0 && o.0 != 0
    }

    /// Win, Ctrl or Alt (Shift alone does not make a shortcut).
    pub const fn commands(self) -> bool {
        self.0 & (Mods::WIN.0 | Mods::CTRL.0 | Mods::ALT.0) != 0
    }
}

/// The modifiers' names, in Alpine's order.
const MOD_NAMES: [(Mods, &str); 4] =
    [(Mods::WIN, "Win"), (Mods::CTRL, "Ctrl"), (Mods::ALT, "Alt"), (Mods::SHIFT, "Shift")];

/// The arrows' names and directions.
const ARROWS: [(&str, Arrow); 4] = [("←", Arrow::Left), ("↑", Arrow::Up), ("→", Arrow::Right), ("↓", Arrow::Down)];

/// The modifier a key is, if it is one.
pub fn modifier_of(key: Key) -> Option<Mods> {
    match key {
        Key::MetaLeft | Key::MetaRight => Some(Mods::WIN),
        Key::ControlLeft | Key::ControlRight => Some(Mods::CTRL),
        Key::AltLeft | Key::AltRight => Some(Mods::ALT),
        Key::ShiftLeft | Key::ShiftRight => Some(Mods::SHIFT),
        _ => None,
    }
}

/// The name of `key` on `layout`: Alpine's label of the keys that do not
/// type, the character a typing key types (unshifted, upper case; `+` is
/// "Plus", as `+` joins the names). None for modifiers, locks and keys
/// without a character (dead keys).
pub fn key_name(key: Key, layout: &Layout) -> Option<String> {
    use Key::*;
    let fixed = match key {
        Space => "Space",
        Enter | NumpadEnter => "Enter",
        Backspace => "Backspace",
        Tab => "Tab",
        Escape => "Esc",
        Insert => "Insert",
        Delete => "Delete",
        Home => "Home",
        End => "End",
        PageUp => "Page Up",
        PageDown => "Page Down",
        PrintScreen => "Print",
        Pause => "Pause",
        ArrowLeft => "←",
        ArrowUp => "↑",
        ArrowRight => "→",
        ArrowDown => "↓",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        Numpad0 => "Num 0",
        Numpad1 => "Num 1",
        Numpad2 => "Num 2",
        Numpad3 => "Num 3",
        Numpad4 => "Num 4",
        Numpad5 => "Num 5",
        Numpad6 => "Num 6",
        Numpad7 => "Num 7",
        Numpad8 => "Num 8",
        Numpad9 => "Num 9",
        ShiftLeft | ShiftRight | ControlLeft | ControlRight | AltLeft | AltRight | MetaLeft | MetaRight | CapsLock
        | NumLock | ScrollLock | ContextMenu => return None,
        _ => "",
    };
    if !fixed.is_empty() {
        return Some(fixed.into());
    }
    let c = keysym(layout, key, Modifiers::NONE).to_char().filter(|c| !c.is_whitespace())?;
    if c == '+' {
        return Some("Plus".into());
    }
    Some(c.to_uppercase().collect())
}

/// What ends a combination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// The modifier pressed and released alone (the Windows key).
    Tap,
    /// A key, by name.
    Key(String),
    /// Any arrow (snapping: the arrow says where).
    Arrows,
}

/// A key combination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub mods: Mods,
    pub trigger: Trigger,
}

impl Chord {
    pub fn key(mods: Mods, name: &str) -> Chord {
        Chord { mods, trigger: Trigger::Key(name.into()) }
    }

    /// Read `Ctrl+Space`, `Win+Arrows`, `Win` (a tap); None if it is not
    /// a combination.
    pub fn parse(text: &str) -> Option<Chord> {
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (last, rest) = parts.split_last()?;
        let mut mods = Mods::NONE;
        for p in rest {
            let m = MOD_NAMES.iter().find(|(_, n)| n == p)?.0;
            if mods.has(m) {
                return None;
            }
            mods = mods.union(m);
        }
        if let Some(&(m, _)) = MOD_NAMES.iter().find(|(_, n)| n == last) {
            // A modifier alone: tapped.
            return rest.is_empty().then_some(Chord { mods: m, trigger: Trigger::Tap });
        }
        let trigger = match *last {
            "" => return None,
            "Arrows" => Trigger::Arrows,
            name => Trigger::Key(name.into()),
        };
        Some(Chord { mods, trigger })
    }

    /// The text [`Chord::parse`] reads.
    pub fn to_text(&self) -> String {
        let mut parts: Vec<&str> = MOD_NAMES.iter().filter(|(m, _)| self.mods.has(*m)).map(|(_, n)| *n).collect();
        match &self.trigger {
            Trigger::Tap => {}
            Trigger::Key(name) => parts.push(name),
            Trigger::Arrows => parts.push("Arrows"),
        }
        parts.join("+")
    }

    /// The key caps (Alpine's `chordKeys`): the modifiers, then the key
    /// ("← ↑ → ↓" for the arrows).
    pub fn caps(&self) -> Vec<String> {
        let mut caps: Vec<String> =
            MOD_NAMES.iter().filter(|(m, _)| self.mods.has(*m)).map(|(_, n)| (*n).to_string()).collect();
        match &self.trigger {
            Trigger::Tap => {}
            Trigger::Key(name) if name == "Plus" => caps.push("+".into()),
            Trigger::Key(name) => caps.push(name.clone()),
            Trigger::Arrows => caps.push("← ↑ → ↓".into()),
        }
        caps
    }

    /// Every (modifiers, key) it answers to, for finding clashes: a tap is
    /// its modifier with no key; the switcher's combination with Shift
    /// added goes backwards, so it holds that too.
    fn expands(&self, action: Action) -> Vec<(Mods, &str)> {
        let mut v: Vec<(Mods, &str)> = match &self.trigger {
            Trigger::Tap => alloc::vec![(self.mods, "")],
            Trigger::Key(n) => alloc::vec![(self.mods, n.as_str())],
            Trigger::Arrows => ARROWS.iter().map(|(n, _)| (self.mods, *n)).collect(),
        };
        if action == Action::SwitchWindows && !self.mods.has(Mods::SHIFT) {
            let more: Vec<(Mods, &str)> = v.iter().map(|&(m, n)| (m.union(Mods::SHIFT), n)).collect();
            v.extend(more);
        }
        v
    }
}

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    /// Show or hide the launcher (Alpine's Search hotkey).
    Launcher,
    /// The launcher's second hotkey.
    LauncherAlt,
    /// Open the settings.
    Settings,
    /// The notification history.
    Notifications,
    /// The window switcher.
    SwitchWindows,
    /// Snap the active window (with an arrow).
    SnapWindow,
    /// Close the active window.
    CloseWindow,
}

impl Action {
    pub const ALL: [Action; 7] = [
        Action::Launcher,
        Action::LauncherAlt,
        Action::Settings,
        Action::Notifications,
        Action::SwitchWindows,
        Action::SnapWindow,
        Action::CloseWindow,
    ];

    /// The settings file's name (`shortcut.<id>`).
    pub fn id(self) -> &'static str {
        match self {
            Action::Launcher => "launcher",
            Action::LauncherAlt => "launcher-alt",
            Action::Settings => "settings",
            Action::Notifications => "notifications",
            Action::SwitchWindows => "switch-windows",
            Action::SnapWindow => "snap-window",
            Action::CloseWindow => "close-window",
        }
    }

    /// The settings page's label.
    pub fn label(self) -> &'static str {
        match self {
            Action::Launcher => "Search hotkey",
            Action::LauncherAlt => "Search, second hotkey",
            Action::Settings => "Settings",
            Action::Notifications => "Notifications",
            Action::SwitchWindows => "Switch windows",
            Action::SnapWindow => "Snap window",
            Action::CloseWindow => "Close window",
        }
    }

    /// Poly's combination for it.
    pub fn default_chord(self) -> Chord {
        match self {
            Action::Launcher => Chord { mods: Mods::WIN, trigger: Trigger::Tap },
            Action::LauncherAlt => Chord::key(Mods::CTRL, "Space"),
            Action::Settings => Chord::key(Mods::WIN, "I"),
            Action::Notifications => Chord::key(Mods::WIN, "N"),
            Action::SwitchWindows => Chord::key(Mods::ALT, "Tab"),
            Action::SnapWindow => Chord { mods: Mods::WIN, trigger: Trigger::Arrows },
            Action::CloseWindow => Chord::key(Mods::ALT, "F4"),
        }
    }

    fn by_id(id: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.id() == id)
    }
}

/// Why a recorded combination was not taken (Alpine's refusals).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NeedsModifier,
    NeedsArrow,
    Reserved,
    Taken(Action),
    /// A key without a name on this layout (a dead key).
    Unknown,
}

impl Refusal {
    /// What the row says (Alpine's `refusalMessage`, Poly for Windows).
    pub fn message(self) -> String {
        match self {
            Refusal::NeedsModifier => "Hold Win, Ctrl or Alt with the key.".into(),
            Refusal::NeedsArrow => "Press the modifiers with an arrow key.".into(),
            Refusal::Reserved => "Poly keeps that one for itself.".into(),
            Refusal::Taken(a) => format!("Already used by {}.", a.label()),
            Refusal::Unknown => "Couldn't save that shortcut.".into(),
        }
    }
}

/// Combinations nobody may take (Alpine reserves the ones Windows keeps;
/// Poly keeps Ctrl+Alt+Delete free for the system).
const RESERVED: [(Mods, &str); 1] = [(Mods(Mods::CTRL.0 | Mods::ALT.0), "Delete")];

/// The keymap: the defaults and the user's changes to them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    /// Combinations that differ from the defaults, in [`Action::ALL`]'s
    /// order.
    overrides: Vec<(Action, Chord)>,
}

impl Keymap {
    /// The combination `action` has now.
    pub fn chord(&self, action: Action) -> Chord {
        self.overrides.iter().find(|(a, _)| *a == action).map_or_else(|| action.default_chord(), |(_, c)| c.clone())
    }

    /// Whether `action` has a combination of its own (the reset button).
    pub fn is_custom(&self, action: Action) -> bool {
        self.overrides.iter().any(|(a, _)| *a == action)
    }

    pub fn any_custom(&self) -> bool {
        !self.overrides.is_empty()
    }

    /// Give `action` `chord` (back to the default if it is the default).
    pub fn set(&mut self, action: Action, chord: Chord) {
        self.overrides.retain(|(a, _)| *a != action);
        if chord != action.default_chord() {
            self.overrides.push((action, chord));
            self.overrides.sort_by_key(|(a, _)| *a);
        }
    }

    /// Back to the default; whether anything changed.
    pub fn reset(&mut self, action: Action) -> bool {
        let before = self.overrides.len();
        self.overrides.retain(|(a, _)| *a != action);
        self.overrides.len() != before
    }

    pub fn reset_all(&mut self) -> bool {
        !core::mem::take(&mut self.overrides).is_empty()
    }

    /// A `shortcut.<id>=<chord>` line of the settings file; false if it is
    /// not one this version reads (an unknown action, a bad combination,
    /// or one that clashes with what is already there).
    pub fn read_entry(&mut self, id: &str, value: &str) -> bool {
        let (Some(action), Some(chord)) = (Action::by_id(id), Chord::parse(value)) else { return false };
        let fits = match (&chord.trigger, action) {
            (Trigger::Tap, _) => true,
            (Trigger::Arrows, Action::SnapWindow) => chord.mods.commands(),
            (Trigger::Arrows, _) | (_, Action::SnapWindow) => false,
            (Trigger::Key(_), _) => chord.mods.commands(),
        };
        if !fits || self.clash(action, &chord).is_some() {
            return false;
        }
        self.set(action, chord);
        true
    }

    /// The lines for the settings file: (id, combination) of each change.
    pub fn entries(&self) -> Vec<(&'static str, String)> {
        self.overrides.iter().map(|(a, c)| (a.id(), c.to_text())).collect()
    }

    /// Another action that answers to some of what `chord` would.
    fn clash(&self, action: Action, chord: &Chord) -> Option<Action> {
        let mine = chord.expands(action);
        Action::ALL.into_iter().filter(|&a| a != action).find(|&other| {
            let theirs = self.chord(other);
            let theirs = theirs.expands(other);
            mine.iter().any(|m| theirs.contains(m))
        })
    }

    /// Check a recorded combination (`mods` held, the key named `key`) for
    /// `action` (Alpine's `validateChord`): the combination to store, or
    /// why not.
    pub fn validate(&self, action: Action, mods: Mods, key: &str) -> Result<Chord, Refusal> {
        let arrows = action.default_chord().trigger == Trigger::Arrows;
        if arrows && !ARROWS.iter().any(|(n, _)| *n == key) {
            return Err(Refusal::NeedsArrow);
        }
        if !mods.commands() {
            return Err(Refusal::NeedsModifier);
        }
        if RESERVED.iter().any(|&(m, k)| m == mods && k == key) {
            return Err(Refusal::Reserved);
        }
        let chord = if arrows { Chord { mods, trigger: Trigger::Arrows } } else { Chord::key(mods, key) };
        match self.clash(action, &chord) {
            Some(other) => Err(Refusal::Taken(other)),
            None => Ok(chord),
        }
    }
}

/// How long recording waits for a combination (Alpine's 15 s).
pub const RECORDING_SECONDS: i64 = 15;

/// What recording ended with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// Escape alone.
    Cancelled,
    /// The modifiers held and the key that ended it.
    Chord(Action, Mods, Key),
}

/// Recording a combination (Alpine's `createShortcutRecorder`): the
/// modifiers pressed since it started, the keys it swallowed (their
/// releases are swallowed too, after it ended).
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    active: Option<(Action, i64)>,
    held: Vec<Key>,
    swallowed: Vec<Key>,
}

impl Recorder {
    /// Record for `action` from `now` (Unix seconds).
    pub fn start(&mut self, action: Action, now: i64) {
        self.active = Some((action, now));
        self.held.clear();
    }

    pub fn stop(&mut self) -> bool {
        self.active.take().is_some()
    }

    /// The action being recorded for.
    pub fn active(&self) -> Option<Action> {
        self.active.map(|(a, _)| a)
    }

    /// Stop if nothing came within [`RECORDING_SECONDS`]; whether it did.
    pub fn time_out(&mut self, now: i64) -> bool {
        match self.active {
            Some((_, since)) if now - since >= RECORDING_SECONDS => self.stop(),
            _ => false,
        }
    }

    /// A key event: whether it is swallowed (the clients and the shortcuts
    /// do not see it), and what recording ended with if it ended.
    pub fn key(&mut self, key: Key, pressed: bool) -> (bool, Option<Recorded>) {
        if !pressed {
            self.held.retain(|&k| k != key);
            let had = self.swallowed.contains(&key);
            self.swallowed.retain(|&k| k != key);
            return (had, None);
        }
        if self.swallowed.contains(&key) {
            return (true, None);
        }
        let Some((action, _)) = self.active else { return (false, None) };
        self.swallowed.push(key);
        if modifier_of(key).is_some() {
            if !self.held.contains(&key) {
                self.held.push(key);
            }
            return (true, None);
        }
        let mods = self.held.iter().filter_map(|&k| modifier_of(k)).fold(Mods::NONE, Mods::union);
        self.active = None;
        if key == Key::Escape && mods == Mods::NONE {
            return (true, Some(Recorded::Cancelled));
        }
        (true, Some(Recorded::Chord(action, mods, key)))
    }
}

/// What a key did, by the keymap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Show or hide the launcher.
    Launcher,
    Settings,
    Notifications,
    /// The switcher opens (backwards with Shift).
    SwitchStart {
        backward: bool,
    },
    /// While it is open: the binding's key again (Shift: back), Left/Right.
    SwitchStep {
        forward: bool,
    },
    /// Releasing the binding's modifier, or Enter: switch.
    SwitchCommit,
    /// Escape: close it without switching.
    SwitchCancel,
    Close,
    Place(Arrow),
}

/// Matches the keys against the keymap; remembers a modifier pressed alone
/// (a tap is its release with nothing else pressed between).
#[derive(Clone, Debug, Default)]
pub struct Matcher {
    tap: Option<Key>,
}

impl Matcher {
    /// Key `key` went down or up with `mods` held before it, on `layout`.
    pub fn key(
        &mut self,
        map: &Keymap,
        key: Key,
        pressed: bool,
        mods: Mods,
        layout: &Layout,
        switcher_open: bool,
    ) -> Option<Hit> {
        let modifier = modifier_of(key);
        if !pressed {
            let tapped = self.tap == Some(key);
            if tapped {
                self.tap = None;
            }
            let m = modifier?;
            if switcher_open && map.chord(Action::SwitchWindows).mods.has(m) {
                return Some(Hit::SwitchCommit);
            }
            if tapped {
                let tap = Chord { mods: m, trigger: Trigger::Tap };
                return Action::ALL.into_iter().find(|&a| map.chord(a) == tap).and_then(hit);
            }
            return None;
        }
        if modifier.is_some() {
            self.tap = (mods == Mods::NONE).then_some(key);
            return None;
        }
        self.tap = None;
        let name = key_name(key, layout)?;
        if switcher_open {
            let sw = map.chord(Action::SwitchWindows);
            if sw.trigger == Trigger::Key(name.clone()) {
                let back = mods.has(Mods::SHIFT) && !sw.mods.has(Mods::SHIFT);
                return Some(Hit::SwitchStep { forward: !back });
            }
            return match name.as_str() {
                "→" => Some(Hit::SwitchStep { forward: true }),
                "←" => Some(Hit::SwitchStep { forward: false }),
                "Esc" => Some(Hit::SwitchCancel),
                "Enter" => Some(Hit::SwitchCommit),
                _ => None,
            };
        }
        for action in Action::ALL {
            let c = map.chord(action);
            match &c.trigger {
                Trigger::Key(n) if *n == name => {
                    if mods == c.mods {
                        return hit(action);
                    }
                    if action == Action::SwitchWindows && !c.mods.has(Mods::SHIFT) && mods == c.mods.union(Mods::SHIFT)
                    {
                        return Some(Hit::SwitchStart { backward: true });
                    }
                }
                Trigger::Arrows if mods == c.mods => {
                    if let Some(&(_, arrow)) = ARROWS.iter().find(|(n, _)| *n == name) {
                        return Some(Hit::Place(arrow));
                    }
                }
                _ => {}
            }
        }
        None
    }
}

/// What an action's combination does (snapping needs its arrow).
fn hit(action: Action) -> Option<Hit> {
    Some(match action {
        Action::Launcher | Action::LauncherAlt => Hit::Launcher,
        Action::Settings => Hit::Settings,
        Action::Notifications => Hit::Notifications,
        Action::SwitchWindows => Hit::SwitchStart { backward: false },
        Action::CloseWindow => Hit::Close,
        Action::SnapWindow => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_input::layout::{DE_CH, US};

    fn win(name: &str) -> Chord {
        Chord::key(Mods::WIN, name)
    }

    #[test]
    fn combinations_read_and_write() {
        for text in ["Win", "Ctrl+Space", "Win+I", "Win+Arrows", "Alt+F4", "Ctrl+Alt+Shift+Page Up", "Win+Plus"] {
            assert_eq!(Chord::parse(text).unwrap().to_text(), text);
        }
        assert_eq!(Chord::parse("Win").unwrap().trigger, Trigger::Tap);
        assert_eq!(Chord::parse("Win+Arrows").unwrap().caps(), ["Win", "← ↑ → ↓"]);
        assert_eq!(Chord::parse("Ctrl+Shift+Plus").unwrap().caps(), ["Ctrl", "Shift", "+"]);
        for bad in ["", "Win+", "Win+Ctrl", "Hyper+I", "Win+Win+I", "+"] {
            assert_eq!(Chord::parse(bad), None, "{bad:?}");
        }
        // Modifiers are written in Alpine's order.
        assert_eq!(Chord::parse("Shift+Win+K").unwrap().to_text(), "Win+Shift+K");
    }

    #[test]
    fn keys_are_named_as_the_layout_types_them() {
        assert_eq!(key_name(Key::I, &US).as_deref(), Some("I"));
        assert_eq!(key_name(Key::Space, &US).as_deref(), Some("Space"));
        assert_eq!(key_name(Key::ArrowLeft, &US).as_deref(), Some("←"));
        assert_eq!(key_name(Key::F4, &US).as_deref(), Some("F4"));
        assert_eq!(key_name(Key::PageUp, &US).as_deref(), Some("Page Up"));
        assert_eq!(key_name(Key::Comma, &US).as_deref(), Some(","));
        assert_eq!(key_name(Key::Equal, &US).as_deref(), Some("="));
        assert_eq!(key_name(Key::MetaLeft, &US), None);
        // Swiss German swaps Y and Z: the key in Y's place types Z.
        assert_eq!(key_name(Key::Y, &DE_CH).as_deref(), Some("Z"));
        assert_eq!(key_name(Key::Z, &DE_CH).as_deref(), Some("Y"));
        assert_eq!(key_name(Key::Semicolon, &DE_CH).as_deref(), Some("Ö"));
    }

    #[test]
    fn defaults_changes_and_resets() {
        let mut map = Keymap::default();
        assert_eq!(map.chord(Action::Settings), win("I"));
        assert!(!map.any_custom());
        map.set(Action::Settings, win("O"));
        assert!(map.is_custom(Action::Settings));
        assert_eq!(map.entries(), [("settings", "Win+O".to_string())]);
        // Setting the default again is no change.
        map.set(Action::Settings, win("I"));
        assert!(!map.any_custom());
        map.set(Action::Launcher, Chord::key(Mods::ALT, "Space"));
        map.set(Action::Notifications, win("M"));
        assert!(map.reset(Action::Launcher));
        assert!(!map.reset(Action::Launcher));
        assert!(map.reset_all());
        assert_eq!(map, Keymap::default());
    }

    #[test]
    fn refusals_as_alpines() {
        let map = Keymap::default();
        assert_eq!(map.validate(Action::Settings, Mods::WIN, "O"), Ok(win("O")));
        assert_eq!(map.validate(Action::Settings, Mods::SHIFT, "O"), Err(Refusal::NeedsModifier));
        assert_eq!(map.validate(Action::Settings, Mods::NONE, "F2"), Err(Refusal::NeedsModifier));
        assert_eq!(map.validate(Action::SnapWindow, Mods::WIN, "K"), Err(Refusal::NeedsArrow));
        assert_eq!(
            map.validate(Action::SnapWindow, Mods::CTRL.union(Mods::ALT), "→"),
            Ok(Chord { mods: Mods::CTRL.union(Mods::ALT), trigger: Trigger::Arrows })
        );
        assert_eq!(map.validate(Action::CloseWindow, Mods::CTRL.union(Mods::ALT), "Delete"), Err(Refusal::Reserved));
        // Taken: refused, as Alpine does (no swap), naming the other.
        assert_eq!(map.validate(Action::Settings, Mods::WIN, "N"), Err(Refusal::Taken(Action::Notifications)));
        assert_eq!(map.validate(Action::Settings, Mods::CTRL, "Space"), Err(Refusal::Taken(Action::LauncherAlt)));
        // Snapping holds every arrow; the switcher holds Shift's backwards.
        assert_eq!(map.validate(Action::Settings, Mods::WIN, "↑"), Err(Refusal::Taken(Action::SnapWindow)));
        assert_eq!(
            map.validate(Action::CloseWindow, Mods::ALT.union(Mods::SHIFT), "Tab"),
            Err(Refusal::Taken(Action::SwitchWindows))
        );
        // Its own combination is fine.
        assert_eq!(map.validate(Action::Notifications, Mods::WIN, "N"), Ok(win("N")));
        assert_eq!(Refusal::Taken(Action::Notifications).message(), "Already used by Notifications.");
        assert_eq!(Refusal::NeedsModifier.message(), "Hold Win, Ctrl or Alt with the key.");
    }

    #[test]
    fn the_settings_file_lines() {
        let mut map = Keymap::default();
        assert!(map.read_entry("launcher", "Ctrl+Alt+K"));
        assert!(map.read_entry("snap-window", "Ctrl+Alt+Arrows"));
        assert!(!map.read_entry("snap-window", "Win+K"), "snapping takes arrows");
        assert!(!map.read_entry("settings", "Win+Arrows"));
        assert!(!map.read_entry("settings", "K"), "no modifier");
        assert!(!map.read_entry("settings", "Win+N"), "taken");
        assert!(!map.read_entry("teleport", "Win+T"));
        assert_eq!(map.chord(Action::Launcher), Chord::key(Mods::CTRL.union(Mods::ALT), "K"));
        assert_eq!(map.entries().len(), 2);
    }

    #[test]
    fn recording_as_alpines_recorder() {
        let mut r = Recorder::default();
        // Not recording: nothing is swallowed.
        assert_eq!(r.key(Key::A, true), (false, None));
        assert_eq!(r.key(Key::A, false), (false, None));
        r.start(Action::Settings, 100);
        assert_eq!(r.active(), Some(Action::Settings));
        assert_eq!(r.key(Key::ControlLeft, true), (true, None));
        assert_eq!(r.key(Key::AltLeft, true), (true, None));
        assert_eq!(
            r.key(Key::K, true),
            (true, Some(Recorded::Chord(Action::Settings, Mods::CTRL.union(Mods::ALT), Key::K)))
        );
        assert_eq!(r.active(), None);
        // The releases of what it swallowed are swallowed too; others not.
        assert_eq!(r.key(Key::K, false), (true, None));
        assert_eq!(r.key(Key::AltLeft, false), (true, None));
        assert_eq!(r.key(Key::ControlLeft, false), (true, None));
        assert_eq!(r.key(Key::B, true), (false, None));
        // Escape alone cancels; with a modifier it is a combination.
        r.start(Action::Launcher, 100);
        assert_eq!(r.key(Key::Escape, true), (true, Some(Recorded::Cancelled)));
        r.key(Key::Escape, false);
        r.start(Action::Launcher, 100);
        r.key(Key::MetaLeft, true);
        assert_eq!(r.key(Key::Escape, true).1, Some(Recorded::Chord(Action::Launcher, Mods::WIN, Key::Escape)));
        // 15 seconds without a combination.
        r.start(Action::Launcher, 100);
        assert!(!r.time_out(114));
        assert!(r.time_out(115));
        assert_eq!(r.active(), None);
    }

    fn press(m: &mut Matcher, map: &Keymap, key: Key, mods: Mods, open: bool) -> Option<Hit> {
        m.key(map, key, true, mods, &US, open)
    }

    #[test]
    fn matching_the_keys() {
        let mut map = Keymap::default();
        let mut m = Matcher::default();
        // The Windows key tapped alone toggles the launcher.
        assert_eq!(press(&mut m, &map, Key::MetaLeft, Mods::NONE, false), None);
        assert_eq!(m.key(&map, Key::MetaLeft, false, Mods::WIN, &US, false), Some(Hit::Launcher));
        // Not when another key came between.
        press(&mut m, &map, Key::MetaLeft, Mods::NONE, false);
        assert_eq!(press(&mut m, &map, Key::I, Mods::WIN, false), Some(Hit::Settings));
        assert_eq!(m.key(&map, Key::MetaLeft, false, Mods::WIN, &US, false), None);
        assert_eq!(press(&mut m, &map, Key::Space, Mods::CTRL, false), Some(Hit::Launcher));
        assert_eq!(press(&mut m, &map, Key::Space, Mods::CTRL.union(Mods::SHIFT), false), None, "exact modifiers");
        assert_eq!(press(&mut m, &map, Key::ArrowLeft, Mods::WIN, false), Some(Hit::Place(Arrow::Left)));
        assert_eq!(press(&mut m, &map, Key::F4, Mods::ALT, false), Some(Hit::Close));
        assert_eq!(press(&mut m, &map, Key::N, Mods::WIN, false), Some(Hit::Notifications));
        assert_eq!(press(&mut m, &map, Key::A, Mods::NONE, false), None);
        // The switcher: start, step, back, commit on Alt's release.
        assert_eq!(press(&mut m, &map, Key::Tab, Mods::ALT, false), Some(Hit::SwitchStart { backward: false }));
        assert_eq!(
            press(&mut m, &map, Key::Tab, Mods::ALT.union(Mods::SHIFT), false),
            Some(Hit::SwitchStart { backward: true })
        );
        assert_eq!(press(&mut m, &map, Key::Tab, Mods::ALT, true), Some(Hit::SwitchStep { forward: true }));
        assert_eq!(
            press(&mut m, &map, Key::Tab, Mods::ALT.union(Mods::SHIFT), true),
            Some(Hit::SwitchStep { forward: false })
        );
        assert_eq!(press(&mut m, &map, Key::Escape, Mods::ALT, true), Some(Hit::SwitchCancel));
        assert_eq!(m.key(&map, Key::AltLeft, false, Mods::ALT, &US, true), Some(Hit::SwitchCommit));
        assert_eq!(m.key(&map, Key::AltLeft, false, Mods::ALT, &US, false), None);
        // Rebound: the new combinations work, the old ones do not.
        map.set(Action::Launcher, Chord::key(Mods::CTRL.union(Mods::ALT), "K"));
        map.set(Action::SnapWindow, Chord { mods: Mods::CTRL.union(Mods::ALT), trigger: Trigger::Arrows });
        map.set(Action::SwitchWindows, Chord::key(Mods::WIN, "Tab"));
        press(&mut m, &map, Key::MetaLeft, Mods::NONE, false);
        assert_eq!(m.key(&map, Key::MetaLeft, false, Mods::WIN, &US, false), None, "no tap bound");
        assert_eq!(press(&mut m, &map, Key::K, Mods::CTRL.union(Mods::ALT), false), Some(Hit::Launcher));
        assert_eq!(press(&mut m, &map, Key::ArrowLeft, Mods::WIN, false), None);
        assert_eq!(press(&mut m, &map, Key::ArrowUp, Mods::CTRL.union(Mods::ALT), false), Some(Hit::Place(Arrow::Up)));
        assert_eq!(press(&mut m, &map, Key::Tab, Mods::ALT, false), None);
        assert_eq!(press(&mut m, &map, Key::Tab, Mods::WIN, false), Some(Hit::SwitchStart { backward: false }));
        assert_eq!(m.key(&map, Key::MetaRight, false, Mods::WIN, &US, true), Some(Hit::SwitchCommit));
    }

    #[test]
    fn matching_follows_the_layout() {
        let mut map = Keymap::default();
        map.set(Action::Settings, win("Z"));
        let mut m = Matcher::default();
        // On Swiss German the key that types Z is where US has Y.
        assert_eq!(m.key(&map, Key::Y, true, Mods::WIN, &DE_CH, false), Some(Hit::Settings));
        assert_eq!(m.key(&map, Key::Z, true, Mods::WIN, &DE_CH, false), None);
        assert_eq!(m.key(&map, Key::Z, true, Mods::WIN, &US, false), Some(Hit::Settings));
    }
}
