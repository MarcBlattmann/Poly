//! The launcher's state: shown or not, what was typed, the matching apps
//! and which is selected. Keys as Alpine Shell's launcher takes them: text
//! narrows the list, Up and Down (or Ctrl+P / Ctrl+N) move the selection,
//! Enter starts the selected app, Escape hides the launcher, Backspace
//! deletes a character (Ctrl+Backspace or Ctrl+W a word, Ctrl+U all).

use crate::apps::App;
use crate::search::search;
use alloc::string::String;
use alloc::vec::Vec;

/// A key the launcher understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    /// Delete the word before the caret.
    BackspaceWord,
    /// Delete everything.
    Clear,
    Up,
    Down,
    Enter,
    Escape,
}

/// What a key did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing visible changed.
    Unchanged,
    /// The query, the list or the selection changed.
    Changed,
    /// The launcher was hidden.
    Hidden,
    /// Start this app (the launcher was hidden).
    Launch(App),
}

/// The launcher.
#[derive(Clone, Debug, Default)]
pub struct Launcher {
    visible: bool,
    query: String,
    /// Indices into the catalog, best match first.
    results: Vec<usize>,
    selected: usize,
    /// The row under the pointer.
    hover: Option<usize>,
    limit: usize,
}

impl Launcher {
    pub fn new(limit: usize) -> Launcher {
        Launcher { limit, ..Launcher::default() }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn results(&self) -> &[usize] {
        &self.results
    }

    pub fn selected(&self) -> Option<usize> {
        (!self.results.is_empty()).then_some(self.selected)
    }

    pub fn hover(&self) -> Option<usize> {
        self.hover
    }

    /// Show it, empty.
    pub fn show(&mut self) {
        self.visible = true;
        self.query.clear();
        self.results.clear();
        self.selected = 0;
        self.hover = None;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.hover = None;
    }

    /// Show it if hidden, hide it if shown (the hotkey).
    pub fn toggle(&mut self) {
        if self.visible { self.hide() } else { self.show() }
    }

    /// How many rows may show (the screen's room); the selection stays in it.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(1);
    }

    fn refilter(&mut self, apps: &[App]) {
        self.results = search(apps, &self.query, self.limit)
            .into_iter()
            .filter_map(|a| apps.iter().position(|x| core::ptr::eq(x, a)))
            .collect();
        self.selected = 0;
        self.hover = None;
    }

    /// The row under the pointer (`None`: none).
    pub fn set_hover(&mut self, row: Option<usize>) -> bool {
        let row = row.filter(|&r| r < self.results.len());
        if row == self.hover {
            return false;
        }
        self.hover = row;
        // Moving the pointer moves the selection, as in Alpine.
        if let Some(r) = row {
            self.selected = r;
        }
        true
    }

    /// Start the app in row `row`.
    pub fn activate(&mut self, apps: &[App], row: usize) -> Outcome {
        match self.results.get(row).and_then(|&i| apps.get(i)) {
            Some(app) => {
                let app = app.clone();
                self.hide();
                Outcome::Launch(app)
            }
            None => Outcome::Unchanged,
        }
    }

    /// A key, with `apps` the catalog.
    pub fn key(&mut self, apps: &[App], key: Key) -> Outcome {
        if !self.visible {
            return Outcome::Unchanged;
        }
        match key {
            Key::Char(c) if !c.is_control() => {
                self.query.push(c);
                self.refilter(apps);
            }
            Key::Char(_) => return Outcome::Unchanged,
            Key::Backspace => {
                if self.query.pop().is_none() {
                    return Outcome::Unchanged;
                }
                self.refilter(apps);
            }
            Key::BackspaceWord => {
                let trimmed = self.query.trim_end_matches(' ');
                let cut = trimmed.rfind(' ').map_or(0, |i| i + 1);
                if self.query.is_empty() {
                    return Outcome::Unchanged;
                }
                self.query.truncate(cut);
                self.refilter(apps);
            }
            Key::Clear => {
                if self.query.is_empty() {
                    return Outcome::Unchanged;
                }
                self.query.clear();
                self.refilter(apps);
            }
            Key::Up | Key::Down => {
                let n = self.results.len();
                if n < 2 {
                    return Outcome::Unchanged;
                }
                self.selected = if key == Key::Down { (self.selected + 1) % n } else { (self.selected + n - 1) % n };
            }
            Key::Enter => return self.activate(apps, self.selected),
            Key::Escape => {
                self.hide();
                return Outcome::Hidden;
            }
        }
        Outcome::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::builtins;

    fn typed(l: &mut Launcher, apps: &[App], s: &str) {
        for c in s.chars() {
            l.key(apps, Key::Char(c));
        }
    }

    #[test]
    fn typing_filters_and_enter_launches() {
        let apps = builtins();
        let mut l = Launcher::new(8);
        assert_eq!(l.key(&apps, Key::Char('x')), Outcome::Unchanged, "hidden: keys do nothing");
        l.show();
        assert!(l.results().is_empty(), "an empty query lists nothing");
        typed(&mut l, &apps, "dem");
        assert_eq!(l.query(), "dem");
        assert_eq!(l.results().len(), 1);
        assert_eq!(apps[l.results()[0]].name, "Poly Demo");
        match l.key(&apps, Key::Enter) {
            Outcome::Launch(app) => assert_eq!(app.argv, ["/bin/wl-demo"]),
            o => panic!("{o:?}"),
        }
        assert!(!l.is_visible());
    }

    #[test]
    fn selection_editing_and_escape() {
        let apps = builtins();
        let mut l = Launcher::new(8);
        l.toggle();
        typed(&mut l, &apps, "t");
        // "t" is in both built-ins ("Wayland Test Client" by a word start).
        assert_eq!(l.results().len(), 2);
        assert_eq!(l.selected(), Some(0));
        l.key(&apps, Key::Down);
        assert_eq!(l.selected(), Some(1));
        l.key(&apps, Key::Down);
        assert_eq!(l.selected(), Some(0), "wraps");
        l.key(&apps, Key::Up);
        assert_eq!(l.selected(), Some(1));
        // Hovering selects.
        assert!(l.set_hover(Some(0)));
        assert_eq!(l.selected(), Some(0));
        assert!(!l.set_hover(Some(9)) || l.hover().is_none());
        assert_eq!(l.key(&apps, Key::Backspace), Outcome::Changed);
        assert_eq!(l.query(), "");
        assert_eq!(l.key(&apps, Key::Backspace), Outcome::Unchanged);
        typed(&mut l, &apps, "poly de");
        l.key(&apps, Key::BackspaceWord);
        assert_eq!(l.query(), "poly ");
        l.key(&apps, Key::Clear);
        assert_eq!(l.query(), "");
        assert_eq!(l.key(&apps, Key::Char('\u{8}')), Outcome::Unchanged, "control characters are not text");
        assert_eq!(l.key(&apps, Key::Escape), Outcome::Hidden);
        assert!(!l.is_visible());
        l.toggle();
        assert_eq!(l.query(), "", "shown again: empty");
    }
}
