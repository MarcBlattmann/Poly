//! The browser: the state the Files window and the file picker share.
//!
//! A [`Browser`] holds the current folder, the history (back and forward),
//! every entry of the folder ([`Browser::entries`], kept sorted) and the
//! **rows**: the entries shown after hiding dot files, the filter text and
//! the picker's filter. Everything the UI addresses (clicks, the cursor,
//! [`Browser::entry`]) is by row.
//!
//! **Listing** ([`Browser::open_dir`]) reads the folder and stats every
//! entry, following symbolic links (a broken link stays in the list as a
//! [`Kind::Symlink`]). Types are guessed by name only ([`mime::guess`]
//! without content), so listing never opens a file; content sniffing
//! happens per file when it is opened ([`crate::apps::open`]). A regular
//! file with an execute bit and no telling name is shown as a program.
//!
//! **Sorting**: folders always come first, in either order; then by the
//! key (size, type description, modification time; unknown times first),
//! ties and the name key broken by [`natural_cmp`]: case-insensitive, with
//! digit runs compared as numbers ("file2" before "file10").
//!
//! **Selection** is a set of entries plus an anchor (where a Shift range
//! starts) and a cursor (the keyboard focus). It follows the files, not
//! the rows: re-sorting, filtering and [`Browser::reload`] keep the same
//! files selected, minus those no longer shown (an action never touches a
//! file the user cannot see). Moving into another folder clears it; going
//! up or back to a parent selects the folder just left.
//!
//! **Keys** ([`Browser::move_cursor`]): in the list view Up/Down move by
//! one row; in the grid view Left/Right move by one and Up/Down by a line
//! of `columns`. Home/End/PageUp/PageDown work in both. With `extend`
//! (Shift) the selection becomes the range from the anchor to the cursor.
//! **Type-ahead** ([`Browser::type_ahead`]) jumps to the next name
//! starting with what was typed; the typed prefix resets after a second
//! without typing, and pressing one letter repeatedly cycles through the
//! names starting with it.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::fs::{Fs, FsError, Kind, Meta};
use crate::mime::{self, FileIcon};
use crate::path::{file_name, is_hidden, join, normalize, parent};

/// One file or folder in the listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    /// What it is, links followed ([`Kind::Symlink`] for a broken link).
    pub kind: Kind,
    pub size: u64,
    /// Guessed from the name ([`mime::guess`]).
    pub mime: &'static str,
    pub icon: FileIcon,
    /// A dot file.
    pub hidden: bool,
    pub modified: Option<u64>,
    /// A regular file with an execute bit.
    pub executable: bool,
    /// The entry is a symbolic link (to `kind`).
    pub symlink: bool,
}

impl Entry {
    /// The entry for `name` in folder `dir`, given what the listing said
    /// and the followed metadata (`None` if `stat` failed).
    pub fn new(dir: &str, name: &str, listed: Kind, meta: Option<Meta>) -> Entry {
        let meta = meta
            .unwrap_or(Meta { kind: if listed == Kind::Symlink { Kind::Symlink } else { listed }, ..Meta::default() });
        let executable = meta.kind == Kind::File && meta.executable();
        let mut m = mime::guess(name, meta.kind, None);
        if executable && m == mime::OCTET_STREAM {
            m = mime::EXECUTABLE;
        }
        Entry {
            name: name.into(),
            path: join(dir, name),
            kind: meta.kind,
            size: meta.size,
            mime: m,
            icon: mime::icon(m),
            hidden: is_hidden(name),
            modified: meta.modified,
            executable,
            symlink: listed == Kind::Symlink,
        }
    }

    /// Whether this is a folder (or a link to one).
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }
}

/// What the list is sorted by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    /// The type's description.
    Kind,
    Modified,
}

/// Which way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

/// How the entries are laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum View {
    #[default]
    List,
    Grid,
}

/// A cursor movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp(usize),
    PageDown(usize),
}

/// What a file picker accepts. Folders are always shown (to browse into).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickFilter {
    /// Accepted types: `image/png`, `image/*`, `*` (all), or a name
    /// pattern `*.ext`. Empty accepts every file.
    pub mime_patterns: Vec<String>,
    /// Only folders can be picked: files are hidden.
    pub folders_only: bool,
}

impl PickFilter {
    /// Whether `entry` is shown.
    pub fn accepts(&self, entry: &Entry) -> bool {
        if entry.is_dir() {
            return true;
        }
        if self.folders_only {
            return false;
        }
        self.mime_patterns.is_empty()
            || self.mime_patterns.iter().any(|p| {
                let p = p.trim();
                if p == "*" || p == "*/*" {
                    true
                } else if let Some(ext) = p.strip_prefix("*.") {
                    let n = entry.name.len();
                    n > ext.len()
                        && entry
                            .name
                            .get(n - ext.len() - 1..)
                            .is_some_and(|t| t.strip_prefix('.').is_some_and(|t| t.eq_ignore_ascii_case(ext)))
                } else if let Some(top) = p.strip_suffix("/*") {
                    entry.mime.split_once('/').is_some_and(|(t, _)| t.eq_ignore_ascii_case(top))
                } else {
                    entry.mime.eq_ignore_ascii_case(p)
                }
            })
    }
}

/// Natural, case-insensitive order: runs of digits compare as numbers,
/// everything else by lowercase character; equal names (ignoring case and
/// leading zeros) fall back to byte order so the result is total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut core::iter::Peekable<core::str::Chars<'_>>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let ord = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = c.to_lowercase().cmp(d.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// How long a type-ahead prefix lasts, in milliseconds.
pub const TYPE_AHEAD_MS: u64 = 1000;

/// A folder's listing with sorting, filtering, selection and history.
#[derive(Clone, Debug, Default)]
pub struct Browser {
    cwd: String,
    back: Vec<String>,
    forward: Vec<String>,
    entries: Vec<Entry>,
    /// Entry indices of the rows.
    visible: Vec<usize>,
    /// Row of each entry, if shown.
    row_of: Vec<Option<usize>>,
    /// Entry indices.
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    cursor: Option<usize>,
    view: View,
    sort: SortKey,
    order: SortOrder,
    show_hidden: bool,
    /// Lowercased.
    filter: String,
    pick: Option<PickFilter>,
    typed: String,
    typed_at: u64,
}

impl Browser {
    /// An empty browser; call [`Browser::open_dir`].
    pub fn new() -> Browser {
        Browser::default()
    }

    /// The current folder (empty before the first `open_dir`).
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Every entry of the folder, sorted, including hidden ones.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// How many rows are shown.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    /// Whether no rows are shown.
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// The entry in `row`.
    pub fn entry(&self, row: usize) -> Option<&Entry> {
        self.entries.get(*self.visible.get(row)?)
    }

    /// The shown entries, in order.
    pub fn rows(&self) -> impl Iterator<Item = &Entry> + '_ {
        self.visible.iter().filter_map(|&i| self.entries.get(i))
    }

    /// Whether `row` is selected.
    pub fn is_selected(&self, row: usize) -> bool {
        self.visible.get(row).is_some_and(|i| self.selected.contains(i))
    }

    /// The selected rows, in order.
    pub fn selected_rows(&self) -> Vec<usize> {
        (0..self.visible.len()).filter(|&r| self.is_selected(r)).collect()
    }

    /// The selected entries, in row order.
    pub fn selected_entries(&self) -> Vec<&Entry> {
        self.selected_rows().into_iter().filter_map(|r| self.entry(r)).collect()
    }

    /// The selected paths, in row order.
    pub fn selected_paths(&self) -> Vec<String> {
        self.selected_entries().into_iter().map(|e| e.path.clone()).collect()
    }

    /// The cursor's row.
    pub fn cursor(&self) -> Option<usize> {
        self.row(self.cursor)
    }

    /// The entry under the cursor.
    pub fn cursor_entry(&self) -> Option<&Entry> {
        self.cursor().and_then(|r| self.entry(r))
    }

    pub fn view(&self) -> View {
        self.view
    }

    pub fn sort(&self) -> (SortKey, SortOrder) {
        (self.sort, self.order)
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// The filter text (lowercased).
    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn pick_filter(&self) -> Option<&PickFilter> {
        self.pick.as_ref()
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Whether there is a parent folder.
    pub fn can_go_up(&self) -> bool {
        parent(&self.cwd).is_some()
    }

    fn row(&self, entry: Option<usize>) -> Option<usize> {
        self.row_of.get(entry?).copied().flatten()
    }

    /// Reads folder `path`; names that cannot be file names (`..`, with a
    /// `/`) are skipped, so an entry's path is always inside the folder.
    fn load(fs: &dyn Fs, path: &str) -> Result<Vec<Entry>, FsError> {
        let raw = fs.read_dir(path)?;
        Ok(raw
            .into_iter()
            .filter(|r| crate::ops::validate_name(&r.name).is_ok())
            .map(|r| {
                let child = join(path, &r.name);
                Entry::new(path, &r.name, r.kind, fs.stat(&child).ok())
            })
            .collect())
    }

    /// Shows `path` (normalized; a link to a folder is browsed under its
    /// own path), recording the current folder in the back history. On an
    /// error nothing changes.
    pub fn open_dir(&mut self, fs: &dyn Fs, path: &str) -> Result<(), FsError> {
        let path = normalize(path);
        let entries = Self::load(fs, &path)?;
        if !self.cwd.is_empty() && self.cwd != path {
            self.back.push(self.cwd.clone());
            self.forward.clear();
        }
        self.show(path, entries);
        Ok(())
    }

    /// Replaces the listing with `entries` of `path`, selecting the folder
    /// just left if it is in `path`.
    fn show(&mut self, path: String, entries: Vec<Entry>) {
        let left = core::mem::replace(&mut self.cwd, path);
        let same = left == self.cwd;
        self.entries = entries;
        self.selected.clear();
        self.anchor = None;
        self.cursor = None;
        self.typed.clear();
        self.sort_entries();
        self.refresh();
        if !same && parent(&left) == Some(self.cwd.as_str()) {
            self.select_name(file_name(&left));
        }
    }

    /// Goes back in history. `Ok(false)` if there is nothing to go back to.
    pub fn back(&mut self, fs: &dyn Fs) -> Result<bool, FsError> {
        let Some(path) = self.back.last().cloned() else { return Ok(false) };
        let entries = Self::load(fs, &path)?;
        self.back.pop();
        self.forward.push(self.cwd.clone());
        self.show(path, entries);
        Ok(true)
    }

    /// Goes forward in history. `Ok(false)` if there is nothing ahead.
    pub fn forward(&mut self, fs: &dyn Fs) -> Result<bool, FsError> {
        let Some(path) = self.forward.last().cloned() else { return Ok(false) };
        let entries = Self::load(fs, &path)?;
        self.forward.pop();
        self.back.push(self.cwd.clone());
        self.show(path, entries);
        Ok(true)
    }

    /// Opens the parent folder. `Ok(false)` at the root.
    pub fn up(&mut self, fs: &dyn Fs) -> Result<bool, FsError> {
        let Some(p) = parent(&self.cwd).map(String::from) else { return Ok(false) };
        self.open_dir(fs, &p)?;
        Ok(true)
    }

    /// Reads the current folder again, keeping the selection and cursor on
    /// the same names.
    pub fn reload(&mut self, fs: &dyn Fs) -> Result<(), FsError> {
        let entries = Self::load(fs, &self.cwd)?;
        self.keeping_selection(|b| {
            b.entries = entries;
            b.sort_entries();
        });
        Ok(())
    }

    /// Runs `f`, which may reorder or replace the entries, and restores the
    /// selection, anchor and cursor by path.
    fn keeping_selection(&mut self, f: impl FnOnce(&mut Self)) {
        let path = |b: &Self, i: Option<usize>| i.and_then(|i| b.entries.get(i)).map(|e| e.path.clone());
        let selected: Vec<String> = self.selected.iter().filter_map(|&i| path(self, Some(i))).collect();
        let (anchor, cursor) = (path(self, self.anchor), path(self, self.cursor));
        f(self);
        let index: BTreeMap<&str, usize> = self.entries.iter().enumerate().map(|(i, e)| (e.path.as_str(), i)).collect();
        let find = |p: Option<String>| p.and_then(|p| index.get(p.as_str()).copied());
        self.selected = selected.into_iter().filter_map(|p| find(Some(p))).collect();
        self.anchor = find(anchor);
        self.cursor = find(cursor);
        self.refresh();
    }

    fn sort_entries(&mut self) {
        let (key, order) = (self.sort, self.order);
        self.entries.sort_by(|a, b| {
            let (da, db) = (a.is_dir(), b.is_dir());
            if da != db {
                return db.cmp(&da);
            }
            let primary = match key {
                SortKey::Name => Ordering::Equal,
                SortKey::Size if da => Ordering::Equal,
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Kind => {
                    mime::description(a.mime).cmp(mime::description(b.mime)).then_with(|| a.mime.cmp(b.mime))
                }
                SortKey::Modified => a.modified.cmp(&b.modified),
            };
            let ord = primary.then_with(|| natural_cmp(&a.name, &b.name));
            if order == SortOrder::Descending { ord.reverse() } else { ord }
        });
    }

    /// Recomputes the rows and drops what is no longer shown from the
    /// selection.
    fn refresh(&mut self) {
        self.visible.clear();
        self.row_of = alloc::vec![None; self.entries.len()];
        for (i, e) in self.entries.iter().enumerate() {
            let shown = (self.show_hidden || !e.hidden)
                && (self.filter.is_empty() || e.name.to_lowercase().contains(self.filter.as_str()))
                && self.pick.as_ref().is_none_or(|p| p.accepts(e));
            if shown {
                self.row_of[i] = Some(self.visible.len());
                self.visible.push(i);
            }
        }
        let row_of = &self.row_of;
        let shown = |i: &usize| row_of.get(*i).copied().flatten().is_some();
        self.selected.retain(shown);
        self.anchor = self.anchor.filter(shown);
        self.cursor = self.cursor.filter(shown);
    }

    /// Shows only names containing `text` (case-insensitive).
    pub fn set_filter(&mut self, text: &str) {
        self.filter = text.to_lowercase();
        self.refresh();
    }

    /// Shows or hides dot files.
    pub fn set_show_hidden(&mut self, show: bool) {
        self.show_hidden = show;
        self.refresh();
    }

    /// Sorts by `key` in `order`.
    pub fn set_sort(&mut self, key: SortKey, order: SortOrder) {
        self.sort = key;
        self.order = order;
        self.keeping_selection(Self::sort_entries);
    }

    pub fn set_view(&mut self, view: View) {
        self.view = view;
    }

    /// Sets or clears what a file picker accepts.
    pub fn set_pick_filter(&mut self, filter: Option<PickFilter>) {
        self.pick = filter;
        self.refresh();
    }

    /// Selects exactly `row` and puts the anchor and cursor there.
    fn select_only(&mut self, row: usize) {
        if let Some(&i) = self.visible.get(row) {
            self.selected.clear();
            self.selected.insert(i);
            self.anchor = Some(i);
            self.cursor = Some(i);
        }
    }

    /// Selects rows `a..=b` (either order), replacing the selection unless
    /// `add`.
    fn select_range(&mut self, a: usize, b: usize, add: bool) {
        if !add {
            self.selected.clear();
        }
        let (lo, hi) = (a.min(b), a.max(b));
        for r in lo..=hi {
            if let Some(&i) = self.visible.get(r) {
                self.selected.insert(i);
            }
        }
    }

    /// Moves the cursor (see the module docs); `columns` is the grid's
    /// width in items. Returns whether anything changed.
    pub fn move_cursor(&mut self, mv: Move, extend: bool, columns: usize) -> bool {
        let n = self.visible.len();
        if n == 0 {
            return false;
        }
        let last = n - 1;
        let cols = if self.view == View::Grid { columns.max(1) } else { 1 };
        let target = match self.cursor() {
            None => {
                if mv == Move::End {
                    last
                } else {
                    0
                }
            }
            Some(r) => match mv {
                Move::Up if r >= cols => r - cols,
                Move::Up => r,
                Move::Down if r / cols < last / cols => (r + cols).min(last),
                Move::Down => r,
                Move::Left if self.view == View::Grid => r.saturating_sub(1),
                Move::Right if self.view == View::Grid => (r + 1).min(last),
                Move::Left | Move::Right => return false,
                Move::Home => 0,
                Move::End => last,
                Move::PageUp(k) => r.saturating_sub(k.max(1)),
                Move::PageDown(k) => r.saturating_add(k.max(1)).min(last),
            },
        };
        let Some(&t) = self.visible.get(target) else { return false };
        if extend {
            let anchor_row = self.row(self.anchor).or(self.cursor()).unwrap_or(target);
            self.anchor = self.visible.get(anchor_row).copied();
            self.select_range(anchor_row, target, false);
            self.cursor = Some(t);
        } else {
            self.select_only(target);
        }
        true
    }

    /// Selects every row.
    pub fn select_all(&mut self) {
        self.selected = self.visible.iter().copied().collect();
    }

    /// Selects nothing.
    pub fn select_none(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    /// A click on `row` (past the last row: on empty space) with Ctrl
    /// (toggle) and/or Shift (range from the anchor; with Ctrl added to the
    /// selection). A plain click on empty space clears the selection.
    pub fn click(&mut self, row: usize, ctrl: bool, shift: bool) {
        let Some(&i) = self.visible.get(row) else {
            if !ctrl && !shift {
                self.select_none();
            }
            return;
        };
        if shift {
            let anchor_row = self.row(self.anchor).unwrap_or(row);
            self.anchor = self.visible.get(anchor_row).copied();
            self.select_range(anchor_row, row, ctrl);
            self.cursor = Some(i);
        } else if ctrl {
            if !self.selected.remove(&i) {
                self.selected.insert(i);
            }
            self.anchor = Some(i);
            self.cursor = Some(i);
        } else {
            self.select_only(row);
        }
    }

    /// Type-ahead: adds `c` to the typed prefix (started afresh after
    /// [`TYPE_AHEAD_MS`] at `now_ms`) and selects the next row whose name
    /// starts with it. Returns whether one was found.
    pub fn type_ahead(&mut self, c: char, now_ms: u64) -> bool {
        if now_ms < self.typed_at || now_ms - self.typed_at > TYPE_AHEAD_MS {
            self.typed.clear();
        }
        self.typed_at = now_ms;
        self.typed.extend(c.to_lowercase());
        let n = self.visible.len();
        if n == 0 {
            return false;
        }
        let cur = self.cursor();
        let fresh = self.typed.chars().count() == 1;
        // A new prefix searches after the cursor (so pressing a letter
        // again goes to the next match); a longer one includes it.
        let start = match cur {
            Some(r) if fresh => r + 1,
            Some(r) => r,
            None => 0,
        };
        let find = |prefix: &str, start: usize| {
            (0..n)
                .map(|k| (start + k) % n)
                .find(|&r| self.entry(r).is_some_and(|e| e.name.to_lowercase().starts_with(prefix)))
        };
        let mut found = find(&self.typed, start);
        if found.is_none() {
            let mut chars = self.typed.chars();
            let first = chars.next();
            if let Some(f) = first.filter(|f| chars.all(|c| c == *f)) {
                let mut one = String::new();
                one.push(f);
                found = find(&one, cur.map_or(0, |r| r + 1));
            }
        }
        match found {
            Some(r) => {
                self.select_only(r);
                true
            }
            None => false,
        }
    }

    /// Selects the row named `name` alone (after a rename or a new folder).
    /// Returns whether it is shown.
    pub fn select_name(&mut self, name: &str) -> bool {
        let row = self.entries.iter().position(|e| e.name == name).and_then(|i| self.row(Some(i)));
        match row {
            Some(r) => {
                self.select_only(r);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfs::{MemFs, StdFs};
    use alloc::vec;

    fn names(b: &Browser) -> Vec<&str> {
        b.rows().map(|e| e.name.as_str()).collect()
    }

    fn selected(b: &Browser) -> Vec<&str> {
        b.selected_entries().into_iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "a", "B", "file02", "x9y", "x10y", "é"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a", "B", "file1", "File2", "file02", "file10", "x9y", "x10y", "é"]);
        assert_eq!(natural_cmp("a", "A"), Ordering::Greater, "byte order breaks ties");
        assert_eq!(natural_cmp("a1", "a"), Ordering::Greater);
        assert_eq!(natural_cmp("99999999999999999999999", "100000000000000000000000"), Ordering::Less);
    }

    fn sample() -> StdFs {
        let fs = StdFs::new();
        fs.create_dir("/d").unwrap();
        for dir in ["zeta", "Alpha", ".git"] {
            fs.create_dir(&join("/d", dir)).unwrap();
        }
        for (f, data) in
            [("file10.txt", "1234567890"), ("file2.txt", "12"), ("b.png", "x"), (".hidden", ""), ("c.rs", "abc")]
        {
            fs.write(&join("/d", f), data.as_bytes()).unwrap();
        }
        fs
    }

    #[test]
    fn listing_sorting_hiding_filtering() {
        let fs = sample();
        let mut b = Browser::new();
        b.open_dir(&fs, "/d/").unwrap();
        assert_eq!(b.cwd(), "/d");
        assert_eq!(names(&b), ["Alpha", "zeta", "b.png", "c.rs", "file2.txt", "file10.txt"]);
        assert_eq!(b.entries().len(), 8);
        let png = b.entry(2).unwrap();
        assert_eq!((png.mime, png.icon, png.path.as_str()), ("image/png", FileIcon::Image, "/d/b.png"));
        assert_eq!(b.entry(0).unwrap().icon, FileIcon::Folder);

        b.set_show_hidden(true);
        assert_eq!(names(&b), [".git", "Alpha", "zeta", ".hidden", "b.png", "c.rs", "file2.txt", "file10.txt"]);
        b.set_show_hidden(false);

        b.set_sort(SortKey::Size, SortOrder::Descending);
        assert_eq!(names(&b), ["zeta", "Alpha", "file10.txt", "c.rs", "file2.txt", "b.png"], "folders stay first");
        b.set_sort(SortKey::Kind, SortOrder::Ascending);
        assert_eq!(names(&b), ["Alpha", "zeta", "b.png", "c.rs", "file2.txt", "file10.txt"]);
        b.set_sort(SortKey::Name, SortOrder::Ascending);

        b.set_filter("FILE");
        assert_eq!(names(&b), ["file2.txt", "file10.txt"]);
        b.set_filter("");
        assert_eq!(b.len(), 6);
        assert!(b.open_dir(&fs, "/nope").is_err());
        assert_eq!(b.cwd(), "/d", "unchanged after an error");
    }

    #[test]
    fn modified_sort_and_broken_links() {
        let fs = MemFs::new().with(&[("/d/old", ""), ("/d/new", ""), ("/d/unknown", ""), ("/d/sub/", "")]);
        fs.set_modified("/d/old", 10);
        fs.set_modified("/d/new", 20);
        fs.symlink("/d/broken", "/nowhere");
        fs.symlink("/d/to-sub", "sub");
        fs.write("/d/tool", b"\x7fELF").unwrap();
        fs.set_mode("/d/tool", 0o755).unwrap();
        let mut b = Browser::new();
        b.open_dir(&fs, "/d").unwrap();
        b.set_sort(SortKey::Modified, SortOrder::Ascending);
        assert_eq!(names(&b), ["sub", "to-sub", "broken", "tool", "unknown", "old", "new"]);
        let link = b.entries().iter().find(|e| e.name == "to-sub").unwrap();
        assert!(link.symlink && link.is_dir());
        let broken = b.entries().iter().find(|e| e.name == "broken").unwrap();
        assert_eq!((broken.kind, broken.icon), (Kind::Symlink, FileIcon::Symlink));
        let tool = b.entries().iter().find(|e| e.name == "tool").unwrap();
        assert!(tool.executable);
        assert_eq!(tool.icon, FileIcon::Executable);
    }

    #[test]
    fn history_and_up() {
        let fs = sample();
        let mut b = Browser::new();
        assert!(!b.can_go_back());
        b.open_dir(&fs, "/d").unwrap();
        b.open_dir(&fs, "/d/zeta").unwrap();
        assert!(b.is_empty());
        assert!(b.up(&fs).unwrap());
        assert_eq!(b.cwd(), "/d");
        assert_eq!(selected(&b), ["zeta"], "the folder just left");
        assert!(b.back(&fs).unwrap());
        assert_eq!(b.cwd(), "/d/zeta");
        assert!(b.back(&fs).unwrap());
        assert_eq!(b.cwd(), "/d");
        assert!(b.can_go_forward());
        assert!(b.forward(&fs).unwrap());
        assert!(b.forward(&fs).unwrap());
        assert!(!b.forward(&fs).unwrap());
        assert_eq!(b.cwd(), "/d");
        b.open_dir(&fs, "/").unwrap();
        assert!(!b.can_go_forward(), "a new folder clears forward");
        assert!(!b.up(&fs).unwrap());
        // Opening the same folder again is not history.
        let depth = b.back.len();
        b.open_dir(&fs, "/").unwrap();
        assert_eq!(b.back.len(), depth);
    }

    #[test]
    fn selection_with_mouse_and_keys() {
        let fs = sample();
        let mut b = Browser::new();
        b.open_dir(&fs, "/d").unwrap();
        // rows: Alpha zeta b.png c.rs file2.txt file10.txt
        assert!(b.move_cursor(Move::Down, false, 1));
        assert_eq!((b.cursor(), selected(&b)), (Some(0), vec!["Alpha"]));
        b.move_cursor(Move::Down, false, 1);
        b.move_cursor(Move::Down, true, 1);
        b.move_cursor(Move::Down, true, 1);
        assert_eq!(selected(&b), ["zeta", "b.png", "c.rs"]);
        b.move_cursor(Move::Up, true, 1);
        assert_eq!(selected(&b), ["zeta", "b.png"]);
        assert!(!b.move_cursor(Move::Left, false, 1), "no sideways in the list");
        b.move_cursor(Move::End, false, 1);
        assert_eq!(selected(&b), ["file10.txt"]);
        b.move_cursor(Move::PageUp(2), false, 1);
        assert_eq!(selected(&b), ["c.rs"]);
        b.move_cursor(Move::PageDown(10), false, 1);
        assert_eq!(b.cursor(), Some(5));

        b.click(1, false, false);
        b.click(3, false, true);
        assert_eq!(selected(&b), ["zeta", "b.png", "c.rs"]);
        b.click(2, true, false);
        assert_eq!(selected(&b), ["zeta", "c.rs"]);
        b.click(5, true, true);
        assert_eq!(selected(&b), ["zeta", "b.png", "c.rs", "file2.txt", "file10.txt"], "ctrl+shift adds the range");
        b.click(99, true, false);
        assert_eq!(b.selected_rows().len(), 5);
        b.click(99, false, false);
        assert!(b.selected_rows().is_empty());
        b.select_all();
        assert_eq!(b.selected_paths().len(), 6);

        // Grid: Left/Right by one, Up/Down by a line of 4.
        b.set_view(View::Grid);
        b.click(1, false, false);
        b.move_cursor(Move::Down, false, 4);
        assert_eq!(b.cursor(), Some(5));
        b.move_cursor(Move::Down, false, 4);
        assert_eq!(b.cursor(), Some(5), "already on the last line");
        b.move_cursor(Move::Up, false, 4);
        assert_eq!(b.cursor(), Some(1));
        b.move_cursor(Move::Up, false, 4);
        assert_eq!(b.cursor(), Some(1));
        b.move_cursor(Move::Right, true, 4);
        assert_eq!(selected(&b), ["zeta", "b.png"]);
        b.click(2, false, false);
        b.move_cursor(Move::Down, false, 5);
        assert_eq!(b.cursor(), Some(5), "to the last item of a shorter next line");
    }

    #[test]
    fn selection_survives_reload_sort_and_filter() {
        let fs = sample();
        let mut b = Browser::new();
        b.open_dir(&fs, "/d").unwrap();
        b.click(2, false, false); // b.png
        b.click(4, true, false); // file2.txt
        b.set_sort(SortKey::Name, SortOrder::Descending);
        assert_eq!(selected(&b), ["file2.txt", "b.png"]);
        fs.write("/d/a-new", b"").unwrap();
        b.reload(&fs).unwrap();
        assert_eq!(selected(&b), ["file2.txt", "b.png"]);
        assert_eq!(b.cursor_entry().unwrap().name, "file2.txt");
        b.set_filter("png");
        assert_eq!(selected(&b), ["b.png"], "hidden rows leave the selection");
        b.set_filter("");
        assert_eq!(selected(&b), ["b.png"]);
        fs.remove_file("/d/b.png").unwrap();
        b.reload(&fs).unwrap();
        assert!(selected(&b).is_empty());
        assert!(b.select_name("a-new"));
        assert_eq!(b.selected_paths(), ["/d/a-new"]);
        assert!(!b.select_name(".hidden"), "not shown");
    }

    #[test]
    fn type_ahead() {
        let fs = MemFs::new().with(&[("/d/apple", ""), ("/d/avocado", ""), ("/d/banana", ""), ("/d/apricot", "")]);
        let mut b = Browser::new();
        b.open_dir(&fs, "/d").unwrap();
        // rows: apple apricot avocado banana
        assert!(b.type_ahead('A', 0));
        assert_eq!(selected(&b), ["apple"]);
        assert!(b.type_ahead('v', 500));
        assert_eq!(selected(&b), ["avocado"]);
        assert!(b.type_ahead('b', 2000), "a pause starts a new prefix");
        assert_eq!(selected(&b), ["banana"]);
        assert!(!b.type_ahead('x', 2100));
        assert_eq!(selected(&b), ["banana"]);
        // One letter again and again cycles.
        assert!(b.type_ahead('a', 5000));
        assert_eq!(selected(&b), ["apple"]);
        assert!(b.type_ahead('a', 5100));
        assert_eq!(selected(&b), ["apricot"]);
        assert!(b.type_ahead('a', 5200));
        assert_eq!(selected(&b), ["avocado"]);
        assert!(b.type_ahead('a', 5300));
        assert_eq!(selected(&b), ["apple"]);
        assert!(!b.type_ahead('p', 5400), "\"aaaap\" matches nothing; only repeats cycle");
        assert_eq!(selected(&b), ["apple"]);
    }

    #[test]
    fn picker_filter() {
        let fs = MemFs::new().with(&[("/d/a.png", ""), ("/d/b.JPG", ""), ("/d/c.txt", ""), ("/d/sub/", "")]);
        let mut b = Browser::new();
        b.open_dir(&fs, "/d").unwrap();
        b.click(3, false, false); // c.txt
        b.set_pick_filter(Some(PickFilter { mime_patterns: vec!["image/*".into()], folders_only: false }));
        assert_eq!(names(&b), ["sub", "a.png", "b.JPG"]);
        assert!(b.selected_rows().is_empty());
        b.set_pick_filter(Some(PickFilter {
            mime_patterns: vec!["*.txt".into(), "image/png".into()],
            folders_only: false,
        }));
        assert_eq!(names(&b), ["sub", "a.png", "c.txt"]);
        b.set_pick_filter(Some(PickFilter { mime_patterns: vec![], folders_only: true }));
        assert_eq!(names(&b), ["sub"]);
        b.set_pick_filter(Some(PickFilter::default()));
        assert_eq!(b.len(), 4);
        b.set_pick_filter(None);
        assert!(b.pick_filter().is_none());
    }
}
