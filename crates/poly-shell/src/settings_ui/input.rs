//! Input: hit testing, pointer, shortcut recording and keys.

use super::*;

impl SettingsWindow {
    /// The target at logical (`x`, `y`).
    pub(super) fn target_at(&self, x: i32, y: i32, w: i32, h: i32, cx: &Context<'_>) -> Option<Target> {
        let (_, targets) = self.layout(w, h, cx);
        targets.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, t)| *t)
    }

    /// The volume at pointer `x` on the slider.
    pub(super) fn volume_at(&self, x: i32, w: i32, h: i32, cx: &Context<'_>) -> Option<u32> {
        let (items, _) = self.layout(w, h, cx);
        items.iter().find_map(|i| match i {
            Item::Slider { rect, .. } => Some(((x - rect.x) * 100 / rect.width.max(1)).clamp(0, 100) as u32),
            _ => None,
        })
    }

    /// The pointer moved to (`x`, `y`): the hover changes, a drag moves
    /// the volume. Returns whether to draw again, and a change.
    pub fn motion(&mut self, x: i32, y: i32, w: i32, h: i32, cx: &Context<'_>) -> (bool, Option<Change>) {
        if self.dragging {
            let v = self.volume_at(x, w, h, cx);
            return match v.filter(|&v| v != cx.settings.volume) {
                Some(v) => (true, Some(Change::Volume(v))),
                None => (false, None),
            };
        }
        let hover = self.target_at(x, y, w, h, cx);
        let redraw = hover != self.hover;
        self.hover = hover;
        (redraw, None)
    }

    /// The pointer left the window.
    pub fn leave(&mut self) -> bool {
        self.hover.take().is_some()
    }

    /// Recording ended with `outcome` (keys named on `layout`): the
    /// combination to store, or the refusal shown in the row.
    pub fn recorded(&mut self, outcome: Recorded, keymap: &Keymap, layout: &Layout) -> Option<Change> {
        self.refusal = None;
        let Recorded::Chord(action, mods, key) = outcome else { return None };
        let checked = match key_name(key, layout) {
            Some(name) => keymap.validate(action, mods, &name),
            None => Err(Refusal::Unknown),
        };
        match checked {
            Ok(chord) => Some(Change::Shortcut(action, chord)),
            Err(why) => {
                self.refusal = Some((action, why.message()));
                None
            }
        }
    }

    /// Whether a shortcut is being recorded (the compositor gives it the
    /// keys first).
    pub fn recording(&self) -> bool {
        self.recorder.active().is_some()
    }

    /// Go to page `p` (remembered for the arrows); the search ends.
    pub(super) fn go(&mut self, p: Page) -> bool {
        if p != self.page {
            self.recorder.stop();
            self.refusal = None;
        }
        let was_searching = !self.search.is_empty();
        self.search.clear();
        self.searching = false;
        if p == self.page {
            return was_searching;
        }
        self.history.truncate(self.at + 1);
        self.history.push(p);
        self.at = self.history.len() - 1;
        self.page = p;
        true
    }

    /// Open the window at page `p` (from outside: the ridge, a hotkey).
    pub fn open_at(&mut self, p: Page) {
        self.go(p);
    }

    /// A left button press or release at (`x`, `y`).
    pub fn button(
        &mut self,
        x: i32,
        y: i32,
        pressed: bool,
        w: i32,
        h: i32,
        cx: &Context<'_>,
    ) -> (bool, Option<Change>) {
        if !pressed {
            self.dragging = false;
            return (false, None);
        }
        let target = self.target_at(x, y, w, h, cx);
        if target != Some(Target::Search) && self.searching && self.search.is_empty() {
            self.searching = false;
        }
        // A press anywhere else ends recording (Alpine's blur).
        let stopped = !matches!(target, Some(Target::Record(_))) && self.recorder.stop();
        let Some(target) = target else { return (stopped, None) };
        let st = cx.settings;
        let change = match target {
            Target::Nav(p) => return (self.go(p), None),
            Target::Tab(i) => return (self.go(SECTIONS[i].1[0]), None),
            Target::Search => {
                self.searching = true;
                return (true, None);
            }
            Target::Result(i) => {
                let found = search(&self.search);
                return match found.get(i) {
                    Some(&(p, _)) => {
                        self.go(p);
                        (true, None)
                    }
                    None => (false, None),
                };
            }
            Target::Back | Target::Forward => {
                let at = if target == Target::Back { self.at.checked_sub(1) } else { Some(self.at + 1) };
                if let Some(&p) = at.and_then(|a| self.history.get(a)) {
                    self.at = at.unwrap_or(0);
                    self.page = p;
                    self.search.clear();
                    return (true, None);
                }
                return (false, None);
            }
            Target::Mode(i) => {
                let mut modes = cx.info.modes.clone();
                if modes.is_empty() {
                    modes.push(cx.info.display);
                }
                modes.get(i).map(|&(mw, mh)| Change::Resolution(mw, mh))
            }
            Target::Scale(s) => Some(Change::Scale(s)),
            Target::Keyboard(i) => KEYBOARDS.get(i).map(|k| Change::Keyboard(k.0.to_string())),
            Target::Wallpaper(i) => PRESETS.get(i).map(|p| Change::Wallpaper(p.0.to_string())),
            Target::Volume => {
                self.dragging = true;
                self.volume_at(x, w, h, cx).map(Change::Volume)
            }
            Target::Mute => Some(Change::Muted(!st.muted)),
            Target::Zone(i) => ZONES.get(i).map(|z| Change::TimeZone(z.0.to_string())),
            Target::Theme(light) => Some(Change::Theme(if light { "light" } else { "dark" }.into())),
            Target::Tiling => Some(Change::Tiling(!st.tiling)),
            Target::SnapKeys => Some(Change::SnapKeys(!st.snap_keys)),
            Target::SnapDrag => Some(Change::SnapDrag(!st.snap_drag)),
            Target::Record(a) => {
                self.refusal = None;
                if self.recorder.active() == Some(a) {
                    self.recorder.stop();
                } else {
                    self.recorder.start(a, cx.utc);
                }
                return (true, None);
            }
            Target::ResetShortcut(a) => {
                self.refusal = None;
                Some(Change::ResetShortcut(a))
            }
            Target::ResetShortcuts => {
                self.refusal = None;
                Some(Change::ResetShortcuts)
            }
        };
        (true, change)
    }

    /// A key while the window has the keyboard: typing into the search
    /// (once its field was clicked, or any letter: typing searches, as in
    /// Alpine), Enter opens the first result, Escape clears it; otherwise
    /// Up and Down step through the pages. Returns whether to draw again.
    pub fn key(&mut self, key: Key) -> bool {
        match key {
            Key::Char(c) if !c.is_control() => {
                self.searching = true;
                self.search.push(c);
                true
            }
            Key::Backspace if self.searching => self.search.pop().is_some(),
            Key::BackspaceWord | Key::Clear if self.searching => {
                self.search.clear();
                true
            }
            Key::Escape if self.searching || !self.search.is_empty() => {
                self.search.clear();
                self.searching = false;
                true
            }
            Key::Enter if !self.search.is_empty() => match search(&self.search).first() {
                Some(&(p, _)) => self.go(p),
                None => false,
            },
            Key::Up => {
                self.step_page(false);
                true
            }
            Key::Down => {
                self.step_page(true);
                true
            }
            _ => false,
        }
    }

    /// The search's text.
    pub fn search_text(&self) -> &str {
        &self.search
    }

    /// Up or Down: the previous or next page.
    pub fn step_page(&mut self, forward: bool) {
        let i = self.page.index();
        let n = Page::ALL.len();
        let p = Page::ALL[if forward { (i + 1) % n } else { (i + n - 1) % n }];
        self.go(p);
    }
}
