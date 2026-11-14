//! The Applications folder (ADR 0175): every installed app in one place,
//! as a folder in Files.
//!
//! It is a **virtual** folder: there is no directory of bundles behind it.
//! Its items are the launcher's own catalog ([`crate::apps::installed`]:
//! Poly's own apps and the desktop files in `/usr/share/applications`), so
//! what the folder shows and what the launcher offers are one list, hidden
//! entries (`NoDisplay`, `Hidden`) and test apps included or left out by
//! the same rule. Opening an item does not start a program from Files: it
//! asks the desktop to start that app ([`open_request`]), which runs the
//! launcher's own code (the shell's own windows, the busy pointer of
//! ADR 0167).
//!
//! This module is the model: the items in order, what kind of app each is
//! and where it comes from, the search rule, what can be done with an item.
//! Files (`userland/poly-files-app`) shows it through its browser.

use crate::apps::App;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use poly_wayland::launch::Control;

/// The folder's location as people type it and as the sidebar names it.
pub const URI: &str = "applications:///";
/// The folder's path inside Files: [`URI`] as a path (not absolute, so it
/// has no parent and can never be a folder on a disk).
pub const DIR: &str = "applications:";
/// The folder's name.
pub const LABEL: &str = "Applications";

/// Whether `text`, typed as a location, means the folder: its URI, its
/// path or its name, in any case.
pub fn is_location(text: &str) -> bool {
    let t = text.trim().trim_end_matches('/');
    t.eq_ignore_ascii_case(DIR) || t.eq_ignore_ascii_case(LABEL)
}

/// Whether `path` is the folder or an item in it.
pub fn contains(path: &str) -> bool {
    path == DIR || label_of(path).is_some()
}

/// The path of the item listed as `label`.
pub fn path_of(label: &str) -> String {
    format!("{DIR}/{label}")
}

/// The listing name of the item at `path`, if it is one.
pub fn label_of(path: &str) -> Option<&str> {
    path.strip_prefix(DIR)?.strip_prefix('/').filter(|l| !l.is_empty())
}

/// Which world an app belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppKind {
    /// Poly's own: built in, or a native program (`poly-*`).
    Poly,
    /// A Linux program from a desktop file.
    Linux,
    /// A Windows program (`.exe`) from a desktop file.
    Windows,
}

impl AppKind {
    /// What kind of app `app` is. A desktop file does not say which world
    /// its program is from, so this reads the program's name: one of
    /// Poly's own (`poly-...`), a Windows executable, else Linux.
    pub fn of(app: &App) -> AppKind {
        if app.id.starts_with("poly:") {
            return AppKind::Poly;
        }
        let program = program(app).rsplit('/').next().unwrap_or_default();
        let exe = program.len() > 4 && program.is_char_boundary(program.len() - 4) && {
            let (_, ext) = program.split_at(program.len() - 4);
            ext.eq_ignore_ascii_case(".exe")
        };
        if exe {
            AppKind::Windows
        } else if program.starts_with("poly-") {
            AppKind::Poly
        } else {
            AppKind::Linux
        }
    }

    /// As the Kind column and the status line say it.
    pub fn label(self) -> &'static str {
        match self {
            AppKind::Poly => "Poly app",
            AppKind::Linux => "Linux app",
            AppKind::Windows => "Windows app",
        }
    }
}

/// The program an app runs: what follows Poly's Terminal for a
/// `Terminal=true` entry, else the first argument.
fn program(app: &App) -> &str {
    match app.argv.as_slice() {
        [term, e, prog, ..] if term == crate::apps::TERMINAL && e == "-e" => prog,
        [prog, ..] => prog,
        [] => "",
    }
}

/// What can be done with an item. Files builds an item's menu from
/// [`Item::actions`], so an action that is not offered has no control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start the app.
    Open,
    /// Remove it (ADR 0180): offered for the apps a package record or the
    /// AppImage installer owns ([`Item::removable`]); the installer's
    /// window asks before anything is removed.
    Uninstall,
}

impl Action {
    /// The menu's label.
    pub fn label(self) -> &'static str {
        match self {
            Action::Open => "Open",
            Action::Uninstall => "Uninstall",
        }
    }
}

/// One app in the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub app: App,
    pub kind: AppKind,
    /// Its name in the listing: the app's name as a file name (no `/`, no
    /// control characters), unique in the folder. The name drawn is
    /// `app.name`.
    pub label: String,
    /// Whether it can be uninstalled ([`Folder::mark_removable`]).
    pub removable: bool,
}

impl Item {
    /// Where the app comes from: built into Poly, or its desktop file.
    pub fn source(&self) -> String {
        if self.app.id.starts_with("poly:") {
            "built into Poly".into()
        } else {
            format!("from {}/{}", crate::apps::APPLICATIONS, self.app.id)
        }
    }

    /// The line Files shows for the selected item: name, kind, source and
    /// what the app is.
    pub fn details(&self) -> String {
        let mut s = format!("{}: {}, {}", self.app.name, self.kind.label(), self.source());
        if !self.app.comment.is_empty() {
            s.push_str(". ");
            s.push_str(&self.app.comment);
        }
        s
    }

    /// What can be done with it: open it, and uninstall it where the
    /// installer owns it (ADR 0180). Poly's own apps and the base system's
    /// are not removable.
    pub fn actions(&self) -> Vec<Action> {
        if self.removable { alloc::vec![Action::Open, Action::Uninstall] } else { alloc::vec![Action::Open] }
    }
}

/// The line that asks the desktop to show the installer's window that
/// uninstalls `item`'s app (for the compositor's control pipe).
pub fn uninstall_request(item: &Item) -> String {
    Control::Uninstall { id: item.app.id.clone() }.line()
}

/// The line that asks the desktop to start `item`'s app as the launcher
/// does (for the compositor's control pipe).
pub fn open_request(item: &Item) -> String {
    Control::Open { id: item.app.id.clone() }.line()
}

/// The app's name as a listing name.
fn listing_name(app: &App) -> String {
    let name: String = app
        .name
        .chars()
        .map(|c| match c {
            '/' => '\u{2215}',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." { app.id.replace('/', "\u{2215}") } else { name.into() }
}

/// The folder's contents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    items: Vec<Item>,
}

impl Folder {
    /// The folder for the installed apps `catalog`
    /// ([`crate::apps::installed`]): sorted by name without regard to case
    /// (then by ID), open windows left out.
    pub fn new(catalog: Vec<App>) -> Folder {
        let mut apps: Vec<App> = catalog.into_iter().filter(|a| a.window().is_none()).collect();
        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
        let mut items: Vec<Item> = Vec::with_capacity(apps.len());
        for app in apps {
            let mut label = listing_name(&app);
            if items.iter().any(|i| i.label == label) {
                label = format!("{label} ({})", app.id.replace('/', "\u{2215}"));
            }
            items.push(Item { kind: AppKind::of(&app), app, label, removable: false });
        }
        Folder { items }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Mark the items whose desktop file ID is in `ids` as removable.
    /// Poly's own apps never are, whatever the list says.
    pub fn mark_removable(&mut self, ids: &[String]) {
        for item in &mut self.items {
            item.removable = !item.app.id.starts_with("poly:") && ids.contains(&item.app.id);
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The item listed as `label`.
    pub fn by_label(&self, label: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.label == label)
    }

    /// The item at `path` ([`path_of`] its label).
    pub fn by_path(&self, path: &str) -> Option<&Item> {
        self.by_label(label_of(path)?)
    }

    /// The items a search for `query` shows: those whose listing name
    /// contains it, without regard to case (the rule of Files' search
    /// field); all of them for an empty query.
    pub fn matching(&self, query: &str) -> Vec<&Item> {
        let q = query.to_lowercase();
        self.items.iter().filter(|i| q.is_empty() || i.label.to_lowercase().contains(&q)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{catalog, sample_apps, window_entry};
    use alloc::vec;

    fn app(name: &str, id: &str, argv: &[&str]) -> App {
        App {
            name: name.into(),
            comment: String::new(),
            keywords: Vec::new(),
            argv: argv.iter().map(|a| (*a).into()).collect(),
            id: id.into(),
            icon: None,
        }
    }

    #[test]
    fn the_folder_lists_the_launchers_catalog_by_name() {
        let mut apps = sample_apps();
        apps.push(window_entry(7, "A window", "viewer"));
        apps.reverse();
        let f = Folder::new(apps);
        let names: Vec<&str> = f.items().iter().map(|i| i.app.name.as_str()).collect();
        assert_eq!(names, ["Files", "Notifications", "Pixel Viewer", "Quill Editor", "Settings", "Terminal"]);
        assert_eq!(f.len(), 6);
        // The same apps the launcher has, nothing more and nothing less.
        let mut ids: Vec<String> = f.items().iter().map(|i| i.app.id.clone()).collect();
        let mut launcher: Vec<String> = sample_apps().into_iter().map(|a| a.id).collect();
        ids.sort();
        launcher.sort();
        assert_eq!(ids, launcher);
        assert!(Folder::new(Vec::new()).is_empty());
    }

    #[test]
    fn names_sort_without_regard_to_case() {
        let f = Folder::new(vec![app("zebra", "z.desktop", &["z"]), app("Apple", "a.desktop", &["a"])]);
        let f2 = Folder::new(catalog(vec![app("zebra", "z.desktop", &["z"]), app("apple", "a.desktop", &["a"])]));
        assert_eq!(f.items()[0].app.name, "Apple");
        assert_eq!(f2.items()[0].app.name, "apple");
        assert_eq!(f2.items().last().unwrap().app.name, "zebra");
    }

    #[test]
    fn kinds_of_apps() {
        let kind = |a: &App| AppKind::of(a);
        for a in catalog(Vec::new()) {
            assert_eq!(kind(&a), AppKind::Poly, "{}", a.name);
        }
        assert_eq!(kind(&app("Chromium", "chromium.desktop", &["/usr/bin/chromium", "--x"])), AppKind::Linux);
        assert_eq!(kind(&app("Poly Fill", "poly-fill.desktop", &["/win/win_gui_fill.exe"])), AppKind::Windows);
        assert_eq!(kind(&app("Setup", "s.desktop", &["/win/SETUP.EXE"])), AppKind::Windows);
        assert_eq!(kind(&app("Viewer", "v.desktop", &["/bin/poly-viewer"])), AppKind::Poly);
        // A terminal program is the program, not Poly's Terminal around it.
        let vim = app("Vim", "vim.desktop", &[crate::apps::TERMINAL, "-e", "vim"]);
        assert_eq!(kind(&vim), AppKind::Linux);
        assert_eq!(kind(&app("Odd", "o.desktop", &[])), AppKind::Linux);
        assert_eq!(kind(&app("é", "e.desktop", &["é.exe"])), AppKind::Windows);
        assert_eq!(kind(&app("exe", "x.desktop", &[".exe"])), AppKind::Linux, "a name, not an extension");
        assert_eq!(
            [AppKind::Poly.label(), AppKind::Linux.label(), AppKind::Windows.label()],
            ["Poly app", "Linux app", "Windows app"]
        );
    }

    #[test]
    fn details_say_the_kind_and_the_source() {
        let f = Folder::new(sample_apps());
        assert_eq!(
            f.by_label("Files").unwrap().details(),
            "Files: Poly app, built into Poly. Browse and organise your files"
        );
        let chromium = Folder::new(vec![app("Chromium", "chromium.desktop", &["chromium"])]);
        assert_eq!(chromium.items()[0].source(), "from /usr/share/applications/chromium.desktop");
        assert_eq!(chromium.items()[0].details(), "Chromium: Linux app, from /usr/share/applications/chromium.desktop");
    }

    #[test]
    fn opening_asks_the_desktop_for_the_app_by_its_id() {
        let f = Folder::new(sample_apps());
        let settings = f.by_label("Settings").unwrap();
        assert_eq!(open_request(settings), "open poly:Settings\n");
        assert_eq!(Control::parse(&open_request(settings)), Some(Control::Open { id: "poly:Settings".into() }));
        // Poly's own apps are not uninstalled, whatever is said.
        assert_eq!(settings.actions(), [Action::Open]);
        assert_eq!(Action::Open.label(), "Open");
    }

    #[test]
    fn apps_the_installer_owns_can_be_uninstalled() {
        let mut f = Folder::new(vec![
            app("Galculator", "galculator.desktop", &["galculator"]),
            app("Chromium", "chromium.desktop", &["chromium"]),
            app("Files", "poly:Files", &["/bin/poly-files"]),
        ]);
        f.mark_removable(&["galculator.desktop".into(), "poly:Files".into()]);
        let by = |l: &str| f.by_label(l).unwrap();
        assert_eq!(by("Galculator").actions(), [Action::Open, Action::Uninstall]);
        assert_eq!(by("Chromium").actions(), [Action::Open]);
        assert_eq!(by("Files").actions(), [Action::Open], "Poly's own apps stay");
        assert_eq!(Action::Uninstall.label(), "Uninstall");
        assert_eq!(uninstall_request(by("Galculator")), "uninstall galculator.desktop\n");
        assert_eq!(
            Control::parse(&uninstall_request(by("Galculator"))),
            Some(Control::Uninstall { id: "galculator.desktop".into() })
        );
        // Marking again replaces the marks.
        f.mark_removable(&[]);
        assert_eq!(f.by_label("Galculator").unwrap().actions(), [Action::Open]);
    }

    #[test]
    fn search_is_by_name_without_regard_to_case() {
        let f = Folder::new(sample_apps());
        let names = |q: &str| -> Vec<&str> { f.matching(q).into_iter().map(|i| i.app.name.as_str()).collect() };
        assert_eq!(names("").len(), 6);
        assert_eq!(names("TER"), ["Terminal"]);
        assert_eq!(names("i"), ["Files", "Notifications", "Pixel Viewer", "Quill Editor", "Settings", "Terminal"]);
        assert_eq!(names("ill"), ["Quill Editor"]);
        assert!(names("nothing").is_empty());
    }

    #[test]
    fn listing_names_are_file_names_and_unique() {
        let f = Folder::new(vec![
            app("Input/Output", "io.desktop", &["io"]),
            app("Input\u{2215}Output", "io2.desktop", &["io"]),
            app("", "empty.desktop", &["e"]),
            app(".NET Tool", "net.desktop", &["n"]),
        ]);
        let labels: Vec<&str> = f.items().iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["empty.desktop", ".NET Tool", "Input\u{2215}Output", "Input\u{2215}Output (io2.desktop)"]);
        assert!(labels.iter().all(|l| !l.contains('/')));
        // The path and back.
        let p = path_of(labels[2]);
        assert_eq!(p, "applications:/Input\u{2215}Output");
        assert_eq!(f.by_path(&p).unwrap().app.id, "io.desktop");
        assert_eq!(f.by_path("applications:"), None);
        assert_eq!(f.by_path("/usr/share/applications/io.desktop"), None);
    }

    #[test]
    fn the_folders_location() {
        for t in ["applications:///", "applications:", "Applications", " applications ", "APPLICATIONS:/"] {
            assert!(is_location(t), "{t}");
        }
        for t in ["/Applications", "apps", "", "applications:/Files", "/usr/share/applications"] {
            assert!(!is_location(t), "{t}");
        }
        assert!(contains(DIR));
        assert!(contains("applications:/Files"));
        assert!(!contains("applications:x"));
        assert!(!contains("/applications:"));
        assert_eq!(label_of("applications:/"), None);
    }
}
