//! The Files window (ADR 0134), in the settings window's layout language
//! (ADR 0127): a 40-pixel header on the ground with back, forward and up,
//! the path bar (a field of breadcrumbs; Ctrl+L or a click on its empty
//! part edits it as text), the search field, the list/grid segmented
//! control and the hidden-files switch; the 224-pixel sidebar of places
//! (Home, Desktop, Documents, Downloads, ..., Applications, Computer, Trash)
//! and mounted disks; the page on surface 1 with the folder's title, a New
//! folder button, column headers that sort (Name, Size, Kind), the rows or
//! tiles, and a status line.
//!
//! Applications (ADR 0175) is the launcher's catalog as a folder
//! (`poly_shell::applications`, put over the file system by
//! [`crate::applications`]): the same rows and tiles with each app's own
//! icon, the kind of app in the Kind column, the selected app's details in
//! the status line; opening one asks the desktop to start it. Nothing in it
//! can be renamed, moved or removed, so those controls are not there.
//!
//! What the window decides lives in `poly-files` ([`Browser`], `ops`,
//! `trash`, `apps`), so the file picker of Phase 8 can reuse it; this
//! module only lays out, draws and maps input onto it.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use poly_app::ui::{DENSE, DENSE_MEDIUM, FIELD_RADIUS, HEADER, NAV_HEIGHT, SIDEBAR, SMALL, TITLE};
use poly_app::{App, AppEvent, CursorShape, Cx, KeyInput, LineEdit, Menu, MenuItem, Ui};
use poly_files::apps::{Handler, Open};
use poly_files::model::{Move, SortKey, SortOrder, View};
use poly_files::ops::{ClipOp, Clipboard, OpError};
use poly_files::places::{Place, PlaceIcon};
use poly_files::{Browser, Entry, FileIcon, Fs, Kind};
use poly_input::key::Key;
use poly_shell::appicon::{AppIcons, IconFiles};
use poly_shell::applications::{self as apps, Action, Item};
use poly_shell::draw::{Face, Style};
use poly_shell::icons::{Icon, lucide};
use poly_shell::theme::pal;
use poly_wayland::compose::{Framebuffer, Rect};

/// Rows of the list, the column header, the footer, tiles of the grid.
pub const ROW: i32 = 34;
pub const COLUMNS: i32 = 30;
pub const FOOTER: i32 = 30;
pub const TILE_W: i32 = 112;
pub const TILE_H: i32 = 104;
pub const TILE_GAP: i32 = 8;
pub const GUTTER: i32 = 20;
/// The search field's width.
pub const SEARCH_W: i32 = 200;
/// An app's icon in a row of the list and in a tile of the grid (a file's
/// glyph is 16 and 40: an app's own picture has the launcher's 28-pixel
/// box in the tile's place for it, and what a row's height leaves).
pub const APP_ICON_ROW: i32 = 20;
pub const APP_ICON_TILE: i32 = 40;

/// The icon of a kind of file.
pub fn file_icon(i: FileIcon) -> Icon {
    match i {
        FileIcon::Folder => lucide::FOLDER,
        FileIcon::File => lucide::FILE,
        FileIcon::Text | FileIcon::Pdf | FileIcon::Document => lucide::FILE_TEXT,
        FileIcon::Code | FileIcon::Script => lucide::FILE_CODE,
        FileIcon::Image => lucide::FILE_IMAGE,
        FileIcon::Audio => lucide::FILE_MUSIC,
        FileIcon::Video => lucide::FILE_VIDEO_CAMERA,
        FileIcon::Archive => lucide::FILE_ARCHIVE,
        FileIcon::Spreadsheet => lucide::FILE_SPREADSHEET,
        FileIcon::Presentation => lucide::LAYOUT_DASHBOARD,
        FileIcon::Executable => lucide::FILE_COG,
        FileIcon::DesktopEntry => lucide::APP_WINDOW,
        FileIcon::Font => lucide::FILE_TYPE,
        FileIcon::Disk => lucide::HARD_DRIVE,
        FileIcon::Symlink => lucide::LINK,
    }
}

/// The icon of a place.
pub fn place_icon(i: PlaceIcon) -> Icon {
    match i {
        PlaceIcon::Home => lucide::HOUSE,
        PlaceIcon::Desktop => lucide::MONITOR,
        PlaceIcon::Documents => lucide::FILE_TEXT,
        PlaceIcon::Downloads => lucide::DOWNLOAD,
        PlaceIcon::Music => lucide::FILE_MUSIC,
        PlaceIcon::Pictures => lucide::FILE_IMAGE,
        PlaceIcon::Videos => lucide::FILE_VIDEO_CAMERA,
        PlaceIcon::Computer | PlaceIcon::Drive | PlaceIcon::Removable => lucide::HARD_DRIVE,
        PlaceIcon::Applications => lucide::APP_WINDOW,
        PlaceIcon::Trash => lucide::TRASH_2,
    }
}

/// A size as people read it.
pub fn size_text(n: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return if n == 1 { "1 byte".into() } else { format!("{n} bytes") };
    }
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v < 10.0 {
        let tenths = (v * 10.0 + 0.5) as u64;
        format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[u])
    } else {
        format!("{} {}", (v + 0.5) as u64, UNITS[u])
    }
}

/// What is under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Back,
    Forward,
    Up,
    /// The path bar: crumb `i`, or its empty part (`None`).
    Crumb(Option<usize>),
    Search,
    ViewList,
    ViewGrid,
    Hidden,
    Place(usize),
    NewFolder,
    EmptyTrash,
    Sort(SortKey),
    Item(usize),
    /// The page's empty space.
    Background,
}

/// Where the keyboard goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    Search,
    Path,
    Rename,
}

/// A question before something that cannot be undone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub text: String,
    pub action: u32,
}

/// Menu and confirmation actions.
pub mod act {
    pub const OPEN: u32 = 1;
    pub const OPEN_TERMINAL: u32 = 2;
    pub const CUT: u32 = 3;
    pub const COPY: u32 = 4;
    pub const PASTE: u32 = 5;
    pub const RENAME: u32 = 6;
    pub const TRASH: u32 = 7;
    pub const DELETE: u32 = 8;
    pub const NEW_FOLDER: u32 = 9;
    pub const HIDDEN: u32 = 10;
    pub const SORT_NAME: u32 = 11;
    pub const SORT_SIZE: u32 = 12;
    pub const SORT_KIND: u32 = 13;
    pub const RESTORE: u32 = 14;
    pub const EMPTY_TRASH: u32 = 15;
    pub const COPY_PATH: u32 = 16;
    pub const REFRESH: u32 = 17;
    pub const UNINSTALL: u32 = 18;
}

/// The window's rectangles for a `w` × `h` window, logical.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub back: Rect,
    pub forward: Rect,
    pub up: Rect,
    pub path: Rect,
    pub search: Rect,
    pub segmented: Rect,
    pub hidden: Rect,
    pub page: Rect,
    pub title: Rect,
    pub new_folder: Rect,
    pub columns: Rect,
    pub list: Rect,
    pub footer: Rect,
    /// Whether the Kind column fits.
    pub kind_column: bool,
}

impl Layout {
    pub fn new(w: i32, h: i32) -> Layout {
        let b = |i: i32| Rect::new(12 + i * 32, 6, 28, 28);
        let hidden = Rect::new(w - 12 - 28, 6, 28, 28);
        let segmented = Rect::new(hidden.x - 8 - 64, 6, 64, 28);
        let search_w = SEARCH_W.min((w - 420).max(120));
        let search = Rect::new(segmented.x - 8 - search_w, 6, search_w, 28);
        let path = Rect::new(b(2).right() + 10, 6, (search.x - 10 - (b(2).right() + 10)).max(40), 28);
        let page = Rect::new(SIDEBAR, HEADER, (w - SIDEBAR).max(0), (h - HEADER).max(0));
        let title = Rect::new(page.x + GUTTER, page.y + 14, page.width - 2 * GUTTER, 36);
        let new_folder = Rect::new(title.right() - 30, title.y + 4, 30, 28);
        let columns = Rect::new(page.x + GUTTER, title.bottom() + 6, page.width - 2 * GUTTER, COLUMNS);
        let footer = Rect::new(page.x, page.bottom() - FOOTER, page.width, FOOTER);
        let list = Rect::new(columns.x, columns.bottom() + 2, columns.width, (footer.y - columns.bottom() - 2).max(0));
        Layout {
            back: b(0),
            forward: b(1),
            up: b(2),
            path,
            search,
            segmented,
            hidden,
            page,
            title,
            new_folder,
            columns,
            list,
            footer,
            kind_column: columns.width >= 460,
        }
    }

    /// The Size and Kind columns' left edges.
    pub fn column_x(&self) -> (i32, i32) {
        let kind_w = if self.kind_column { 150 } else { 0 };
        let kind_x = self.columns.right() - kind_w;
        (kind_x - 90 - 12, kind_x)
    }

    /// Tiles in a grid row.
    pub fn grid_columns(&self) -> usize {
        (((self.list.width + TILE_GAP) / (TILE_W + TILE_GAP)).max(1)) as usize
    }
}

/// Starts a program (the app's own; a test records it).
pub type Launcher = Box<dyn FnMut(&[String]) -> Result<(), String>>;

/// Asks the desktop to start an installed app as its launcher does (the
/// control pipe's `open`; a test records it).
pub type AppOpener = Box<dyn FnMut(&Item) -> Result<(), String>>;

/// Asks the desktop to show the installer's window that uninstalls an
/// installed app (the control pipe's `uninstall`; a test records it).
pub type AppUninstaller = Box<dyn FnMut(&Item) -> Result<(), String>>;

/// The Files app.
pub struct FilesApp {
    pub fs: Box<dyn Fs>,
    pub browser: Browser,
    pub places: Vec<Place>,
    pub home: String,
    pub trash_dir: String,
    pub handlers: Vec<Handler>,
    pub defaults: Option<String>,
    pub launch: Launcher,
    /// The Applications folder as the file system last read it (empty and
    /// unreachable unless [`FilesApp::set_applications`] gave one).
    pub apps: crate::applications::Shared,
    pub open_app: AppOpener,
    pub uninstall_app: AppUninstaller,
    /// The apps' own icons (ADR 0146), loaded as they are drawn.
    icons: RefCell<AppIcons>,
    /// The wall clock, seconds (for the trash's dates).
    pub clock: Box<dyn Fn() -> u64>,
    size: (i32, i32),
    layout: Layout,
    pub focus: Focus,
    pub search: LineEdit,
    pub path_edit: Option<LineEdit>,
    /// The entry being renamed (its name) and the field.
    pub rename: Option<(String, LineEdit)>,
    pub clip: Option<Clipboard>,
    pub menu: Option<Menu>,
    pub confirm: Option<Confirm>,
    hover: Option<Target>,
    /// How far the list is scrolled, logical pixels.
    pub scroll: i32,
    pub status: String,
    now_ms: u64,
    /// The path bar's crumbs as last drawn: where each goes, its rectangle.
    crumb_cache: Vec<(String, Rect)>,
}

impl core::fmt::Debug for FilesApp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilesApp").field("cwd", &self.browser.cwd()).finish_non_exhaustive()
    }
}

impl FilesApp {
    pub fn new(fs: Box<dyn Fs>, home: &str, trash_dir: &str, places: Vec<Place>, launch: Launcher) -> FilesApp {
        FilesApp {
            fs,
            browser: Browser::new(),
            places,
            home: home.into(),
            trash_dir: trash_dir.into(),
            handlers: Vec::new(),
            defaults: None,
            launch,
            apps: crate::applications::Shared::default(),
            open_app: Box::new(|_| Err("The desktop cannot be reached".into())),
            uninstall_app: Box::new(|_| Err("The desktop cannot be reached".into())),
            icons: RefCell::new(AppIcons::default()),
            clock: Box::new(|| 0),
            size: (0, 0),
            layout: Layout::default(),
            focus: Focus::List,
            search: LineEdit::default(),
            path_edit: None,
            rename: None,
            clip: None,
            menu: None,
            confirm: None,
            hover: None,
            scroll: 0,
            status: String::new(),
            now_ms: 0,
            crumb_cache: Vec::new(),
        }
    }

    /// The trash's `files/` folder.
    pub fn trash_files(&self) -> String {
        poly_files::path::join(&self.trash_dir, "files")
    }

    pub fn in_trash(&self) -> bool {
        self.browser.cwd() == self.trash_files()
    }

    /// Show the Applications folder (ADR 0175): `folder` is what the file
    /// system's overlay ([`crate::applications::overlay`]) reads the
    /// installed apps into, `open` asks the desktop to start one. The
    /// sidebar gets its place.
    pub fn set_applications(&mut self, folder: crate::applications::Shared, open: AppOpener) {
        self.apps = folder;
        self.open_app = open;
        if !self.places.iter().any(|p| p.kind == poly_files::places::PlaceKind::Applications) {
            self.places = poly_files::places::with_applications(core::mem::take(&mut self.places));
        }
    }

    /// Where the apps' icons are read from (the icon theme on the disk).
    pub fn set_icon_files(&mut self, files: Box<dyn IconFiles>) {
        self.icons.get_mut().set_files(files);
    }

    /// Whether the Applications folder is shown.
    pub fn in_apps(&self) -> bool {
        self.browser.cwd() == apps::DIR
    }

    /// The app listed as entry `e`, in the Applications folder.
    fn app_of(&self, e: &Entry) -> Option<Item> {
        self.in_apps().then(|| self.apps.borrow().by_path(&e.path).cloned()).flatten()
    }

    /// The folder as the path bar's text.
    fn location(&self) -> &str {
        if self.in_apps() { apps::URI } else { self.browser.cwd() }
    }

    /// Show folder `path`.
    pub fn go(&mut self, path: &str) -> bool {
        // The sidebar's Trash is `trash:///`: its `files/` folder here.
        let path = if path == poly_files::places::TRASH_URI {
            self.trash_dir.clone()
        } else if apps::is_location(path) {
            // Applications, by its name, its URI or its place.
            apps::DIR.into()
        } else {
            path.into()
        };
        let path = path.as_str();
        if path == self.trash_files() || path == self.trash_dir {
            let _ = poly_files::trash::ensure(&*self.fs, &self.trash_dir);
        }
        let target = if path == self.trash_dir { self.trash_files() } else { path.into() };
        match self.browser.open_dir(&*self.fs, &target) {
            Ok(()) => {
                self.after_listing();
                true
            }
            Err(e) => {
                self.status = format!("{target}: {}", e.message());
                false
            }
        }
    }

    fn after_listing(&mut self) {
        self.scroll = 0;
        self.rename = None;
        self.status.clear();
        let names: Vec<&str> = self.browser.rows().take(40).map(|e| e.name.as_str()).collect();
        poly_rt::println!("poly-files: {} ({} items): {}", self.browser.cwd(), self.browser.len(), names.join(" "));
    }

    fn reload(&mut self) {
        if let Err(e) = self.browser.reload(&*self.fs) {
            self.status = format!("{}: {}", self.browser.cwd(), e.message());
        }
    }

    /// The folder's title.
    pub fn title(&self) -> String {
        if self.in_trash() {
            return "Trash".into();
        }
        if self.in_apps() {
            return apps::LABEL.into();
        }
        let cwd = self.browser.cwd();
        if cwd == "/" {
            return poly_files::path::ROOT_LABEL.into();
        }
        if cwd == self.home {
            return poly_files::path::HOME_LABEL.into();
        }
        poly_files::path::file_name(cwd).into()
    }

    /// How many rows there are, as the title's side and the status line
    /// say it.
    pub fn count_text(&self) -> String {
        match (self.browser.len(), self.in_apps()) {
            (1, false) => "1 item".to_string(),
            (n, false) => format!("{n} items"),
            (1, true) => "1 app".to_string(),
            (n, true) => format!("{n} apps"),
        }
    }

    /// The status line: what just happened, else the selection, else the
    /// folder. In Applications the selected app's details: what kind of
    /// app it is and where it comes from.
    pub fn status_line(&self) -> String {
        let (n, sel) = (self.browser.len(), self.browser.selected_rows().len());
        if !self.status.is_empty() {
            self.status.clone()
        } else if self.in_apps() {
            match self.browser.selected_entries().as_slice() {
                [one] => self.app_of(one).map(|i| i.details()).unwrap_or_else(|| one.name.clone()),
                [] => format!("{} installed", self.count_text()),
                _ => format!("{sel} of {n} selected"),
            }
        } else if sel > 0 {
            let size: u64 = self.browser.selected_entries().iter().filter(|e| !e.is_dir()).map(|e| e.size).sum();
            format!("{sel} of {n} selected, {}", size_text(size))
        } else {
            format!("{} in {}", self.count_text(), self.browser.cwd())
        }
    }

    /// The breadcrumbs and their rectangles in the path bar.
    fn crumbs(&self, ui: &Ui) -> Vec<(poly_files::path::Crumb, Rect)> {
        let r = self.layout.path;
        let crumbs = if self.in_trash() {
            alloc::vec![poly_files::path::Crumb { label: "Trash".into(), path: self.trash_files() }]
        } else if self.in_apps() {
            alloc::vec![poly_files::path::Crumb { label: apps::LABEL.into(), path: apps::DIR.into() }]
        } else {
            poly_files::path::crumbs(self.browser.cwd(), Some(&self.home))
        };
        let mut x = r.x + 10;
        let mut out = Vec::new();
        // From the right: the last crumbs that fit.
        let widths: Vec<i32> = crumbs.iter().map(|c| ui.measure(&c.label, DENSE_MEDIUM) + 8).collect();
        let total: i32 = widths.iter().map(|w| w + 16).sum();
        let mut skip = 0;
        let mut t = total;
        while t > r.width - 20 && skip + 1 < crumbs.len() {
            t -= widths[skip] + 16;
            skip += 1;
        }
        for (c, w) in crumbs.into_iter().zip(widths).skip(skip) {
            out.push((c, Rect::new(x, r.y + 3, w, r.height - 6)));
            x += w + 16;
        }
        out
    }

    /// Row `i`'s rectangle in the list (list view), or its tile (grid).
    pub fn item_rect(&self, i: usize) -> Rect {
        let l = &self.layout;
        match self.browser.view() {
            View::List => Rect::new(l.list.x, l.list.y + i as i32 * ROW - self.scroll, l.list.width, ROW),
            View::Grid => {
                let cols = l.grid_columns();
                let (c, r) = ((i % cols) as i32, (i / cols) as i32);
                Rect::new(
                    l.list.x + c * (TILE_W + TILE_GAP),
                    l.list.y + 4 + r * (TILE_H + TILE_GAP) - self.scroll,
                    TILE_W,
                    TILE_H,
                )
            }
        }
    }

    fn content_height(&self) -> i32 {
        let n = self.browser.len() as i32;
        match self.browser.view() {
            View::List => n * ROW,
            View::Grid => {
                let cols = self.layout.grid_columns() as i32;
                (n + cols - 1) / cols * (TILE_H + TILE_GAP) + 4
            }
        }
    }

    fn clamp_scroll(&mut self) {
        let max = (self.content_height() - self.layout.list.height).max(0);
        self.scroll = self.scroll.clamp(0, max);
    }

    /// Scroll so the cursor's row is in view.
    fn reveal_cursor(&mut self) {
        let Some(c) = self.browser.cursor() else { return };
        let r = self.item_rect(c);
        let l = self.layout.list;
        if r.y < l.y {
            self.scroll -= l.y - r.y;
        } else if r.bottom() > l.bottom() {
            self.scroll += r.bottom() - l.bottom();
        }
        self.clamp_scroll();
    }

    /// What is at (`x`, `y`).
    pub fn target_at(&self, x: i32, y: i32) -> Option<Target> {
        let l = &self.layout;
        let hit = |r: Rect| r.contains(x, y);
        if y < HEADER {
            return Some(if hit(l.back) {
                Target::Back
            } else if hit(l.forward) {
                Target::Forward
            } else if hit(l.up) {
                Target::Up
            } else if hit(l.search) {
                Target::Search
            } else if hit(l.hidden) {
                Target::Hidden
            } else if hit(l.segmented) {
                if x < l.segmented.x + l.segmented.width / 2 { Target::ViewList } else { Target::ViewGrid }
            } else if hit(l.path) {
                Target::Crumb(self.crumb_cache.iter().position(|(_, r)| r.contains(x, y)))
            } else {
                return None;
            });
        }
        if x < SIDEBAR {
            return self.place_rects().iter().position(|r| r.contains(x, y)).map(Target::Place);
        }
        if hit(l.new_folder) {
            // Nothing is made in Applications: no button there.
            if self.in_apps() {
                return None;
            }
            return Some(if self.in_trash() { Target::EmptyTrash } else { Target::NewFolder });
        }
        if self.browser.view() == View::List && hit(l.columns) {
            let (size_x, kind_x) = l.column_x();
            return Some(Target::Sort(if x >= kind_x && l.kind_column {
                SortKey::Kind
            } else if x >= size_x && !self.in_apps() {
                SortKey::Size
            } else {
                SortKey::Name
            }));
        }
        if hit(l.list) {
            for i in 0..self.browser.len() {
                if self.item_rect(i).contains(x, y) {
                    return Some(Target::Item(i));
                }
            }
            return Some(Target::Background);
        }
        None
    }

    /// The sidebar rows: the places, then the disks after their label.
    fn place_rects(&self) -> Vec<Rect> {
        let mut y = HEADER + 32;
        let mut out = Vec::new();
        let mut disks = false;
        for p in &self.places {
            if p.kind == poly_files::places::PlaceKind::Mount && !disks {
                disks = true;
                y += 30;
            }
            out.push(Rect::new(12, y, SIDEBAR - 24, NAV_HEIGHT));
            y += NAV_HEIGHT + 2;
        }
        out
    }

    // ---- actions -----------------------------------------------------------

    /// Open the cursor's entry (or the selected ones).
    pub fn open_selected(&mut self) {
        let paths = self.browser.selected_paths();
        let paths = if paths.is_empty() {
            self.browser.cursor_entry().map(|e| alloc::vec![e.path.clone()]).unwrap_or_default()
        } else {
            paths
        };
        for p in paths {
            self.open_path(&p);
        }
    }

    pub fn open_path(&mut self, path: &str) {
        if apps::contains(path) {
            return self.open_app_at(path);
        }
        match poly_files::apps::open(&*self.fs, path, &self.handlers, self.defaults.as_deref()) {
            Ok(Open::Folder(p)) => {
                self.go(&p);
            }
            Ok(Open::Launch(argv)) | Ok(Open::With(argv)) => self.run(&argv),
            Ok(Open::NoApp { mime }) if mime.starts_with("text/") || mime == "application/x-shellscript" => {
                // Text without an app for it: the terminal's editor.
                let argv: Vec<String> =
                    [poly_shell::apps::TERMINAL, "-e", "vi", path].iter().map(|s| s.to_string()).collect();
                self.run(&argv);
            }
            Ok(Open::NoApp { mime }) => {
                self.status = format!("No app opens {} files ({mime})", poly_files::mime::description(mime));
            }
            Err(e) => self.status = format!("{path}: {}", e.message()),
        }
    }

    /// Start the app at `path` of the Applications folder: the desktop
    /// does it, with the launcher's code (ADR 0175).
    fn open_app_at(&mut self, path: &str) {
        let Some(item) = self.apps.borrow().by_path(path).cloned() else {
            self.status = format!("{}: no such app", apps::label_of(path).unwrap_or(path));
            return;
        };
        match (self.open_app)(&item) {
            Ok(()) => {
                poly_rt::println!("poly-files: opened app {} ({})", item.app.name, item.app.id);
                self.status = format!("Opened {}", item.app.name);
            }
            Err(e) => self.status = format!("{}: {e}", item.app.name),
        }
    }

    /// Ask the desktop to show the installer's window for uninstalling the
    /// app at the cursor of the Applications folder (ADR 0180). The window
    /// is the confirmation; nothing is removed here.
    fn uninstall_selected(&mut self) {
        let Some(item) = self.browser.cursor_entry().and_then(|e| self.app_of(e)).filter(|i| i.removable) else {
            self.status = "This app cannot be uninstalled".into();
            return;
        };
        match (self.uninstall_app)(&item) {
            Ok(()) => {
                poly_rt::println!("poly-files: asked to uninstall {} ({})", item.app.name, item.app.id);
                self.status = format!("Asked to uninstall {}", item.app.name);
            }
            Err(e) => self.status = format!("{}: {e}", item.app.name),
        }
    }

    fn run(&mut self, argv: &[String]) {
        match (self.launch)(argv) {
            Ok(()) => {
                poly_rt::println!("poly-files: opened {}", argv.join(" "));
                self.status = format!("Opened {}", poly_files::path::file_name(&argv[0]));
            }
            Err(e) => self.status = e,
        }
    }

    fn open_terminal(&mut self) {
        if self.in_apps() {
            return;
        }
        let dir = match self.browser.cursor_entry().filter(|e| e.is_dir() && self.browser.selected_rows().len() == 1) {
            Some(e) => e.path.clone(),
            None => self.browser.cwd().into(),
        };
        let argv: Vec<String> = alloc::vec![poly_shell::apps::TERMINAL.into(), "--dir".into(), dir];
        self.run(&argv);
    }

    fn set_clip(&mut self, op: ClipOp, cx: &mut Cx) {
        let paths = self.browser.selected_paths();
        if paths.is_empty() || self.in_apps() {
            return;
        }
        // The paths go on the desktop's clipboard as text too.
        cx.copy(paths.join("\n"));
        self.status = format!(
            "{} {} item{}",
            if op == ClipOp::Cut { "Cut" } else { "Copied" },
            paths.len(),
            if paths.len() == 1 { "" } else { "s" }
        );
        self.clip = Some(Clipboard { op, paths });
    }

    fn paste(&mut self) {
        let Some(clip) = self.clip.clone() else { return };
        if self.in_trash() || self.in_apps() {
            return;
        }
        let dest = String::from(self.browser.cwd());
        match poly_files::ops::paste(&*self.fs, &clip, &dest) {
            Ok(report) => {
                if clip.op == ClipOp::Cut {
                    self.clip = None;
                }
                self.reload();
                for p in &report.done {
                    self.browser.select_name(poly_files::path::file_name(p));
                }
                self.status = match report.failed.first() {
                    Some((p, e)) => format!("{}: {}", poly_files::path::file_name(p), e.message()),
                    None => {
                        format!("Pasted {} item{}", report.done.len(), if report.done.len() == 1 { "" } else { "s" })
                    }
                };
            }
            Err(OpError::IntoItself(p)) => self.status = format!("Cannot paste {p} into itself"),
            Err(OpError::Fs(e)) => self.status = e.message().into(),
            Err(OpError::Name(e)) => self.status = e.message().into(),
        }
    }

    pub fn new_folder(&mut self) {
        if self.in_trash() || self.in_apps() {
            return;
        }
        let dir = String::from(self.browser.cwd());
        match poly_files::ops::new_folder(&*self.fs, &dir) {
            Ok(name) => {
                self.reload();
                self.browser.select_name(&name);
                self.reveal_cursor();
                self.start_rename();
            }
            Err(e) => self.status = e.message().into(),
        }
    }

    pub fn start_rename(&mut self) {
        let Some(e) = self.browser.cursor_entry() else { return };
        if self.in_trash() || self.in_apps() {
            return;
        }
        let name = e.name.clone();
        let stem = if e.is_dir() { name.len() } else { poly_files::path::split_ext(&name).0.len() };
        self.rename = Some((name.clone(), LineEdit::selecting(&name, stem)));
        self.focus = Focus::Rename;
    }

    fn finish_rename(&mut self) {
        let Some((old, edit)) = self.rename.take() else { return };
        self.focus = Focus::List;
        if edit.text == old {
            return;
        }
        let dir = String::from(self.browser.cwd());
        match poly_files::ops::rename(&*self.fs, &dir, &old, &edit.text) {
            Ok(new) => {
                // The new path (its name, after validation).
                let name = String::from(poly_files::path::file_name(&new));
                self.reload();
                self.browser.select_name(&name);
                self.status = format!("Renamed to {name}");
            }
            Err(OpError::Name(e)) => self.status = e.message().into(),
            Err(OpError::Fs(e)) => self.status = e.message().into(),
            Err(OpError::IntoItself(_)) => {}
        }
    }

    /// Move the selection to the trash (in the trash: ask to delete).
    pub fn trash_selected(&mut self) {
        let paths = self.browser.selected_paths();
        // An app is not removed by deleting its row: uninstalling comes
        // with the installer (ADR 0175).
        if paths.is_empty() || self.in_apps() {
            return;
        }
        if self.in_trash() {
            return self.ask_delete();
        }
        let now = (self.clock)();
        let mut done = 0;
        for p in &paths {
            match poly_files::trash::trash(&*self.fs, &self.trash_dir, p, now) {
                Ok(_) => done += 1,
                Err(e) => self.status = format!("{}: {}", poly_files::path::file_name(p), e.message()),
            }
        }
        self.reload();
        if done > 0 {
            self.status = format!("Moved {done} item{} to the Trash", if done == 1 { "" } else { "s" });
        }
    }

    fn ask_delete(&mut self) {
        let n = self.browser.selected_paths().len();
        if n == 0 || self.in_apps() {
            return;
        }
        let what = if n == 1 {
            format!("\u{201c}{}\u{201d}", self.browser.selected_entries()[0].name)
        } else {
            format!("{n} items")
        };
        self.confirm = Some(Confirm { text: format!("Delete {what} for ever?"), action: act::DELETE });
    }

    fn delete_selected(&mut self) {
        if self.in_apps() {
            return;
        }
        let in_trash = self.in_trash();
        for e in
            self.browser.selected_entries().into_iter().map(|e| (e.name.clone(), e.path.clone())).collect::<Vec<_>>()
        {
            let r = if in_trash {
                poly_files::trash::delete(&*self.fs, &self.trash_dir, &e.0)
            } else {
                poly_files::ops::delete_permanently(&*self.fs, &e.1)
            };
            if let Err(err) = r {
                self.status = format!("{}: {}", e.0, err.message());
            }
        }
        self.reload();
    }

    fn restore_selected(&mut self) {
        for name in self.browser.selected_entries().into_iter().map(|e| e.name.clone()).collect::<Vec<_>>() {
            match poly_files::trash::restore(&*self.fs, &self.trash_dir, &name) {
                Ok(p) => self.status = format!("Restored {p}"),
                Err(e) => self.status = format!("{name}: {}", e.message()),
            }
        }
        self.reload();
    }

    fn empty_trash(&mut self) {
        if let Err(e) = poly_files::trash::empty(&*self.fs, &self.trash_dir) {
            self.status = e.message().into();
        }
        if self.in_trash() {
            self.reload();
        }
    }

    pub fn act(&mut self, id: u32, cx: &mut Cx) {
        match id {
            act::OPEN => self.open_selected(),
            act::UNINSTALL => self.uninstall_selected(),
            act::OPEN_TERMINAL => self.open_terminal(),
            act::CUT => self.set_clip(ClipOp::Cut, cx),
            act::COPY => self.set_clip(ClipOp::Copy, cx),
            act::PASTE => self.paste(),
            act::RENAME => self.start_rename(),
            act::TRASH => self.trash_selected(),
            act::DELETE => self.delete_selected(),
            act::NEW_FOLDER => self.new_folder(),
            act::HIDDEN => self.browser.set_show_hidden(!self.browser.show_hidden()),
            act::SORT_NAME => self.sort_by(SortKey::Name),
            act::SORT_SIZE => self.sort_by(SortKey::Size),
            act::SORT_KIND => self.sort_by(SortKey::Kind),
            act::RESTORE => self.restore_selected(),
            act::EMPTY_TRASH => {
                self.confirm = Some(Confirm {
                    text: "Delete everything in the Trash for ever?".into(),
                    action: act::EMPTY_TRASH + 100,
                })
            }
            x if x == act::EMPTY_TRASH + 100 => self.empty_trash(),
            act::REFRESH => self.reload(),
            act::COPY_PATH => {
                let paths = self.browser.selected_paths();
                if !paths.is_empty() && !self.in_apps() {
                    cx.copy(paths.join("\n"));
                }
            }
            _ => {}
        }
        self.clamp_scroll();
        cx.redraw();
    }

    fn sort_by(&mut self, key: SortKey) {
        let (k, o) = self.browser.sort();
        let order = if k == key && o == SortOrder::Ascending { SortOrder::Descending } else { SortOrder::Ascending };
        self.browser.set_sort(key, order);
    }

    fn context_menu(&mut self, x: i32, y: i32, on_item: bool) {
        let n = self.browser.selected_paths().len();
        let paste = self.clip.is_some() && !self.in_trash();
        let items = if self.in_apps() {
            // What the model says can be done with an app (today: open it).
            let actions = if on_item {
                self.browser.cursor_entry().and_then(|e| self.app_of(e)).map(|i| i.actions()).unwrap_or_default()
            } else {
                Vec::new()
            };
            let mut items: Vec<MenuItem> = actions
                .into_iter()
                .map(|a| match a {
                    Action::Open => MenuItem::new(act::OPEN, a.label(), "Enter", true),
                    Action::Uninstall => MenuItem::new(act::UNINSTALL, a.label(), "", true),
                })
                .collect();
            if items.is_empty() {
                items.push(MenuItem::new(act::REFRESH, "Refresh", "F5", true));
            }
            items
        } else if self.in_trash() {
            alloc::vec![
                MenuItem::new(act::RESTORE, "Restore", "", on_item && n > 0),
                MenuItem::new(act::DELETE, "Delete for ever", "Delete", on_item && n > 0),
                MenuItem::separator(),
                MenuItem::new(act::EMPTY_TRASH, "Empty Trash", "", !self.browser.is_empty()),
            ]
        } else if on_item {
            alloc::vec![
                MenuItem::new(act::OPEN, "Open", "Enter", true),
                MenuItem::new(act::OPEN_TERMINAL, "Open in Terminal", "", true),
                MenuItem::separator(),
                MenuItem::new(act::CUT, "Cut", "Ctrl+X", true),
                MenuItem::new(act::COPY, "Copy", "Ctrl+C", true),
                MenuItem::new(act::PASTE, "Paste", "Ctrl+V", paste),
                MenuItem::new(act::COPY_PATH, "Copy path", "", true),
                MenuItem::separator(),
                MenuItem::new(act::RENAME, "Rename", "F2", n == 1),
                MenuItem::new(act::TRASH, "Move to Trash", "Delete", true),
            ]
        } else {
            alloc::vec![
                MenuItem::new(act::NEW_FOLDER, "New folder", "Ctrl+Shift+N", true),
                MenuItem::new(act::PASTE, "Paste", "Ctrl+V", paste),
                MenuItem::new(act::OPEN_TERMINAL, "Open Terminal here", "", true),
                MenuItem::separator(),
                MenuItem::new(
                    act::HIDDEN,
                    if self.browser.show_hidden() { "Hide hidden files" } else { "Show hidden files" },
                    "Ctrl+H",
                    true
                ),
                MenuItem::new(act::SORT_NAME, "Sort by name", "", true),
                MenuItem::new(act::SORT_SIZE, "Sort by size", "", true),
                MenuItem::new(act::SORT_KIND, "Sort by kind", "", true),
            ]
        };
        self.menu = Some(Menu::new(items, x, y, self.size.0, self.size.1));
    }

    // ---- input ---------------------------------------------------------------

    fn key(&mut self, k: KeyInput, cx: &mut Cx) {
        if !k.pressed {
            return;
        }
        cx.redraw();
        if self.confirm.is_some() {
            match k.key {
                Some(Key::Enter | Key::NumpadEnter) => {
                    let c = self.confirm.take().map(|c| c.action).unwrap_or(0);
                    self.act(c, cx);
                }
                Some(Key::Escape) => self.confirm = None,
                _ => {}
            }
            return;
        }
        if self.menu.is_some() {
            if k.key == Some(Key::Escape) {
                self.menu = None;
            }
            return;
        }
        let m = k.mods;
        match self.focus {
            Focus::Rename => {
                let Some((_, edit)) = &mut self.rename else {
                    self.focus = Focus::List;
                    return;
                };
                match edit.key(&k) {
                    poly_app::ui::Edit::Enter => self.finish_rename(),
                    poly_app::ui::Edit::Escape => {
                        self.rename = None;
                        self.focus = Focus::List;
                    }
                    _ => {}
                }
                return;
            }
            Focus::Path => {
                let Some(edit) = &mut self.path_edit else {
                    self.focus = Focus::List;
                    return;
                };
                match edit.key(&k) {
                    poly_app::ui::Edit::Enter => {
                        let target = if apps::is_location(&edit.text) {
                            apps::DIR.into()
                        } else {
                            poly_files::path::normalize(&edit.text.replace('~', &self.home))
                        };
                        self.path_edit = None;
                        self.focus = Focus::List;
                        self.go(&target);
                    }
                    poly_app::ui::Edit::Escape => {
                        self.path_edit = None;
                        self.focus = Focus::List;
                    }
                    _ => {}
                }
                return;
            }
            Focus::Search => {
                if let Some(Key::ArrowDown | Key::Enter | Key::NumpadEnter) = k.key {
                    self.focus = Focus::List;
                    if self.browser.cursor().is_none() {
                        self.browser.move_cursor(Move::Home, false, 1);
                    }
                    if matches!(k.key, Some(Key::Enter | Key::NumpadEnter)) && !k.repeat {
                        self.open_selected();
                    }
                    return;
                }
                match self.search.key(&k) {
                    poly_app::ui::Edit::Changed => {
                        self.browser.set_filter(&self.search.text.clone());
                        self.scroll = 0;
                    }
                    poly_app::ui::Edit::Escape => {
                        self.search = LineEdit::default();
                        self.browser.set_filter("");
                        self.focus = Focus::List;
                    }
                    _ => {}
                }
                return;
            }
            Focus::List => {}
        }
        let cols = if self.browser.view() == View::Grid { self.layout.grid_columns() } else { 1 };
        let page = ((self.layout.list.height / ROW).max(1)) as usize;
        let mv = |mv: Move, b: &mut Browser| b.move_cursor(mv, m.shift, cols);
        let ch = k.ch.map(|c| c.to_ascii_lowercase());
        match (k.key, m.ctrl, m.alt) {
            (Some(Key::ArrowLeft), false, true) => {
                let _ = self.browser.back(&*self.fs);
                self.after_listing();
            }
            (Some(Key::ArrowRight), false, true) => {
                let _ = self.browser.forward(&*self.fs);
                self.after_listing();
            }
            (Some(Key::ArrowUp), false, true) | (Some(Key::Backspace), false, false) => {
                if self.browser.up(&*self.fs).unwrap_or(false) {
                    self.after_listing();
                }
            }
            (Some(Key::ArrowUp), false, false) => {
                mv(Move::Up, &mut self.browser);
            }
            (Some(Key::ArrowDown), false, false) => {
                mv(Move::Down, &mut self.browser);
            }
            (Some(Key::ArrowLeft), false, false) => {
                mv(Move::Left, &mut self.browser);
            }
            (Some(Key::ArrowRight), false, false) => {
                mv(Move::Right, &mut self.browser);
            }
            (Some(Key::Home), _, _) => {
                mv(Move::Home, &mut self.browser);
            }
            (Some(Key::End), _, _) => {
                mv(Move::End, &mut self.browser);
            }
            (Some(Key::PageUp), _, _) => {
                mv(Move::PageUp(page), &mut self.browser);
            }
            (Some(Key::PageDown), _, _) => {
                mv(Move::PageDown(page), &mut self.browser);
            }
            // A held Enter opens once: a repeat would start an app again
            // and again, or run down a chain of folders.
            (Some(Key::Enter | Key::NumpadEnter), _, _) if k.repeat => {}
            (Some(Key::Enter | Key::NumpadEnter), _, _) => self.open_selected(),
            (Some(Key::F2), _, _) => self.start_rename(),
            (Some(Key::F5), _, _) => self.reload(),
            (Some(Key::Delete), _, _) if m.shift || self.in_trash() => self.ask_delete(),
            (Some(Key::Delete), _, _) => self.trash_selected(),
            (Some(Key::Escape), _, _) => {
                if !self.search.text.is_empty() {
                    self.search = LineEdit::default();
                    self.browser.set_filter("");
                } else {
                    self.browser.select_none();
                }
            }
            (_, true, false) => match ch {
                Some('a') => self.browser.select_all(),
                Some('c') => self.set_clip(ClipOp::Copy, cx),
                Some('x') => self.set_clip(ClipOp::Cut, cx),
                Some('v') => self.paste(),
                Some('h') => self.browser.set_show_hidden(!self.browser.show_hidden()),
                Some('n') if m.shift => self.new_folder(),
                Some('f') => self.focus = Focus::Search,
                Some('l') => {
                    self.path_edit = Some(LineEdit::selecting(self.location(), self.location().len()));
                    self.focus = Focus::Path;
                }
                Some('1') => self.browser.set_view(View::List),
                Some('2') => self.browser.set_view(View::Grid),
                Some('t') => self.open_terminal(),
                _ => {}
            },
            _ if m.plain() => {
                if let Some(c) = k.ch.filter(|c| !c.is_whitespace() || *c == ' ') {
                    if c == '/' || c == '~' {
                        self.path_edit = Some(LineEdit::new(&c.to_string()));
                        self.focus = Focus::Path;
                    } else {
                        self.browser.type_ahead(c, self.now_ms);
                    }
                }
            }
            _ => {}
        }
        self.reveal_cursor();
    }

    #[allow(clippy::too_many_arguments)]
    fn button(&mut self, button: u32, pressed: bool, x: i32, y: i32, clicks: u32, m: poly_app::Mods, cx: &mut Cx) {
        if !pressed {
            return;
        }
        cx.redraw();
        if let Some(c) = self.confirm.take() {
            let (cancel, ok) = confirm_buttons(self.size);
            if ok.contains(x, y) {
                self.act(c.action, cx);
            } else if !cancel.contains(x, y) && !confirm_rect(self.size).contains(x, y) {
                // A click outside dismisses.
            } else if !cancel.contains(x, y) {
                self.confirm = Some(c);
            }
            return;
        }
        if let Some(menu) = self.menu.take() {
            if let Some(i) = menu.item_at(x, y) {
                let id = menu.items[i].id;
                self.act(id, cx);
            }
            return;
        }
        // A click elsewhere ends a rename (keeping the name typed).
        if self.rename.is_some() && !self.target_at(x, y).is_some_and(|t| matches!(t, Target::Item(_))) {
            self.finish_rename();
        }
        if self.path_edit.is_some() && !self.layout.path.contains(x, y) {
            self.path_edit = None;
            self.focus = Focus::List;
        }
        let target = self.target_at(x, y);
        if button == poly_app::BTN_RIGHT {
            match target {
                Some(Target::Item(i)) => {
                    if !self.browser.is_selected(i) {
                        self.browser.click(i, false, false);
                    }
                    self.context_menu(x, y, true);
                }
                Some(Target::Background) => {
                    self.browser.select_none();
                    self.context_menu(x, y, false);
                }
                _ => {}
            }
            return;
        }
        if button != poly_app::BTN_LEFT {
            return;
        }
        if !matches!(target, Some(Target::Search)) && self.focus == Focus::Search {
            self.focus = Focus::List;
        }
        match target {
            Some(Target::Back) => {
                if self.browser.back(&*self.fs).unwrap_or(false) {
                    self.after_listing();
                }
            }
            Some(Target::Forward) => {
                if self.browser.forward(&*self.fs).unwrap_or(false) {
                    self.after_listing();
                }
            }
            Some(Target::Up) => {
                if self.browser.up(&*self.fs).unwrap_or(false) {
                    self.after_listing();
                }
            }
            Some(Target::Crumb(Some(i))) => {
                if let Some((path, _)) = self.crumb_cache.get(i).cloned() {
                    self.go(&path);
                }
            }
            Some(Target::Crumb(None)) => {
                self.path_edit = Some(LineEdit::selecting(self.location(), self.location().len()));
                self.focus = Focus::Path;
            }
            Some(Target::Search) => self.focus = Focus::Search,
            Some(Target::ViewList) => self.browser.set_view(View::List),
            Some(Target::ViewGrid) => self.browser.set_view(View::Grid),
            Some(Target::Hidden) => self.browser.set_show_hidden(!self.browser.show_hidden()),
            Some(Target::Place(i)) => {
                if let Some(p) = self.places.get(i).map(|p| p.path.clone()) {
                    self.go(&p);
                }
            }
            Some(Target::NewFolder) => self.new_folder(),
            Some(Target::EmptyTrash) => self.act(act::EMPTY_TRASH, cx),
            Some(Target::Sort(k)) => self.sort_by(k),
            Some(Target::Item(i)) => {
                self.focus = Focus::List;
                if clicks >= 2 && self.browser.is_selected(i) {
                    self.open_selected();
                } else {
                    self.browser.click(i, m.ctrl, m.shift);
                }
            }
            Some(Target::Background) => self.browser.select_none(),
            None => {}
        }
        self.clamp_scroll();
    }

    // ---- drawing -------------------------------------------------------------

    fn draw_all(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect) {
        let (w, h) = self.size;
        let l = &self.layout;
        ui.ground(fb, clip, w, h, l.page);
        self.draw_header(ui, fb, clip);
        self.draw_sidebar(ui, fb, clip);
        self.draw_page(ui, fb, clip);
        if let Some(m) = &self.menu {
            m.draw(ui, fb, clip);
        }
        if let Some(c) = &self.confirm {
            self.draw_confirm(ui, fb, clip, c);
        }
    }

    fn draw_header(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect) {
        let l = &self.layout;
        let hv = |t: Target| self.hover == Some(t);
        ui.icon_button(fb, clip, l.back, lucide::ARROW_LEFT, 16, hv(Target::Back), self.browser.can_go_back());
        ui.icon_button(
            fb,
            clip,
            l.forward,
            lucide::ARROW_RIGHT,
            16,
            hv(Target::Forward),
            self.browser.can_go_forward(),
        );
        ui.icon_button(fb, clip, l.up, lucide::ARROW_UP, 16, hv(Target::Up), self.browser.can_go_up());
        // The path bar.
        ui.field(fb, clip, l.path, self.focus == Focus::Path);
        if let Some(edit) = &self.path_edit {
            let r = Rect::new(l.path.x + 10, l.path.y, l.path.width - 20, l.path.height);
            edit.draw(ui, fb, clip, r, DENSE, "", self.focus == Focus::Path);
        } else {
            let crumbs = self.crumbs(ui);
            let n = crumbs.len();
            for (i, (c, r)) in crumbs.iter().enumerate() {
                let last = i + 1 == n;
                if self.hover == Some(Target::Crumb(Some(i))) && !last {
                    ui.round(fb, clip, *r, 6, pal().ink, pal().ghost);
                }
                let (st, ink) = if last { (DENSE_MEDIUM, pal().ink_primary) } else { (DENSE, pal().quiet) };
                ui.text(fb, clip, r.x + 4, r.y, r.height, l.path.right() - 8, &c.label, st, ink);
                if !last {
                    ui.icon(
                        fb,
                        clip,
                        lucide::CHEVRON_RIGHT,
                        r.right() + 2,
                        r.y + (r.height - 12) / 2,
                        12,
                        pal().faint,
                        255,
                    );
                }
            }
        }
        // The search field.
        ui.field(fb, clip, l.search, self.focus == Focus::Search);
        ui.icon(fb, clip, lucide::SEARCH, l.search.x + 10, l.search.y + 7, 14, pal().faint, 255);
        let sr = Rect::new(l.search.x + 32, l.search.y, l.search.width - 40, l.search.height);
        self.search.draw(ui, fb, clip, sr, DENSE, "Search", self.focus == Focus::Search);
        // List or grid; hidden files.
        let list = self.browser.view() == View::List;
        ui.segmented(fb, clip, l.segmented, &[(lucide::LIST, list), (lucide::LAYOUT_GRID, !list)]);
        let eye = if self.browser.show_hidden() { lucide::EYE } else { lucide::EYE_OFF };
        if self.browser.show_hidden() {
            ui.round(fb, clip, l.hidden, FIELD_RADIUS, pal().s3, 255);
        }
        ui.icon_button(fb, clip, l.hidden, eye, 15, hv(Target::Hidden), true);
    }

    fn draw_sidebar(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect) {
        ui.eyebrow(fb, clip, 20, HEADER + 8, "Places");
        let cwd = self.browser.cwd();
        let rects = self.place_rects();
        let mut disks = false;
        for (i, (p, r)) in self.places.iter().zip(rects).enumerate() {
            if p.kind == poly_files::places::PlaceKind::Mount && !disks {
                disks = true;
                ui.eyebrow(fb, clip, 20, r.y - 26, "Disks");
            }
            let active = cwd == p.path
                || (p.kind == poly_files::places::PlaceKind::Trash && self.in_trash())
                || (p.kind == poly_files::places::PlaceKind::Applications && self.in_apps());
            ui.nav_row(fb, clip, r, place_icon(p.icon), &p.label, active, self.hover == Some(Target::Place(i)));
        }
    }

    fn draw_page(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect) {
        let l = &self.layout;
        let page_clip = clip.intersect(&ui.px(l.page));
        // The title and the count.
        let title = self.title();
        let end = ui.text(
            fb,
            page_clip,
            l.title.x,
            l.title.y,
            l.title.height,
            l.new_folder.x - 120,
            &title,
            TITLE,
            pal().ink_primary,
        );
        let n = self.browser.len();
        let in_apps = self.in_apps();
        let count = self.count_text();
        ui.text(fb, page_clip, end + 12, l.title.y + 4, l.title.height, l.new_folder.x - 8, &count, SMALL, pal().faint);
        let (icon, hv) = if self.in_trash() {
            (lucide::TRASH_2, self.hover == Some(Target::EmptyTrash))
        } else {
            (lucide::FOLDER_PLUS, self.hover == Some(Target::NewFolder))
        };
        if !in_apps {
            ui.icon_button(fb, page_clip, l.new_folder, icon, 16, hv, !self.in_trash() || n > 0);
        }
        // Column headers.
        if self.browser.view() == View::List {
            let (size_x, kind_x) = l.column_x();
            let (key, order) = self.browser.sort();
            let col = |label: &str, k: SortKey, x: i32, right: i32| (label.to_string(), k, x, right);
            let mut cols = alloc::vec![col("Name", SortKey::Name, l.columns.x + 38, size_x - 8)];
            // An app has no size to show.
            if !in_apps {
                cols.push(col("Size", SortKey::Size, size_x, size_x + 90));
            }
            if l.kind_column {
                cols.push(col("Kind", SortKey::Kind, kind_x, l.columns.right()));
            }
            let st = Style::new(Face::Medium, 11.0);
            for (label, k, x, right) in cols {
                let ink = if k == key { pal().ink_primary } else { pal().faint };
                let end = ui.text(fb, page_clip, x, l.columns.y, l.columns.height, right, &label, st, ink);
                if k == key {
                    let i = if order == SortOrder::Ascending { lucide::CHEVRON_DOWN } else { lucide::CHEVRON_RIGHT };
                    ui.icon(fb, page_clip, i, end + 4, l.columns.y + (l.columns.height - 11) / 2, 11, ink, 255);
                }
            }
            ui.fill(
                fb,
                page_clip,
                Rect::new(l.columns.x, l.columns.bottom(), l.columns.width, 1),
                pal().ink,
                pal().hairline,
            );
        }
        // The rows or tiles.
        let list_clip = page_clip.intersect(&ui.px(l.list));
        if self.browser.is_empty() {
            let msg = if !self.browser.filter().is_empty() {
                "Nothing matches the search"
            } else if in_apps {
                "No apps are installed"
            } else if self.in_trash() {
                "The Trash is empty"
            } else {
                "This folder is empty"
            };
            let w = ui.measure(msg, DENSE);
            ui.text(
                fb,
                list_clip,
                l.list.x + (l.list.width - w) / 2,
                l.list.y + 40,
                24,
                l.list.right(),
                msg,
                DENSE,
                pal().faint,
            );
        }
        for (i, e) in self.browser.rows().enumerate() {
            let r = self.item_rect(i);
            if r.bottom() < l.list.y || r.y > l.list.bottom() {
                continue;
            }
            match self.browser.view() {
                View::List => self.draw_row(ui, fb, list_clip, i, e, r),
                View::Grid => self.draw_tile(ui, fb, list_clip, i, e, r),
            }
        }
        // The status line.
        ui.fill(fb, page_clip, Rect::new(l.footer.x, l.footer.y, l.footer.width, 1), pal().ink, pal().hairline);
        let text = self.status_line();
        let fit = ui.fit(&text, SMALL, l.footer.width - 2 * GUTTER);
        ui.text(
            fb,
            page_clip,
            l.footer.x + GUTTER,
            l.footer.y,
            l.footer.height,
            l.footer.right() - GUTTER,
            &fit,
            SMALL,
            pal().quiet,
        );
    }

    fn row_fill(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect, i: usize, r: Rect) {
        if self.browser.is_selected(i) {
            ui.round(fb, clip, r, 6, pal().s3, 255);
            if self.browser.cursor() == Some(i) && self.focus == Focus::List {
                ui.border(fb, clip, r, 6, pal().line, 255);
            }
        } else if self.hover == Some(Target::Item(i)) {
            ui.round(fb, clip, r, 6, pal().ink, pal().ghost);
        } else if self.browser.cursor() == Some(i) && self.focus == Focus::List {
            ui.border(fb, clip, r, 6, pal().line, 255);
        }
    }

    fn cut(&self, e: &Entry) -> bool {
        self.clip.as_ref().is_some_and(|c| c.op == ClipOp::Cut && c.paths.contains(&e.path))
    }

    /// An app's icon in the `size` box at (`x`, `y`): its own from the icon
    /// theme (Poly's own apps' artwork), else its glyph as files have
    /// theirs.
    #[allow(clippy::too_many_arguments)]
    fn draw_app_icon(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect, item: &Item, x: i32, y: i32, size: i32) {
        let s = ui.scale;
        let own = item.app.icon.as_deref().and_then(|name| self.icons.borrow_mut().get(name, (size * s) as u32));
        match own {
            Some(bitmap) => bitmap.draw(fb, clip, x * s, y * s, size * s),
            None => {
                // The glyph at a file glyph's weight in the box.
                let g = size * 4 / 5;
                ui.icon(fb, clip, item.app.glyph(), x + (size - g) / 2, y + (size - g) / 2, g, pal().quiet, 255);
            }
        }
    }

    /// A row of the Applications folder: the app's icon and name, its kind.
    fn draw_app_row(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect, item: &Item, r: Rect) {
        let l = &self.layout;
        let (_, kind_x) = l.column_x();
        let icon_x = r.x + 12 - (APP_ICON_ROW - 16) / 2;
        self.draw_app_icon(ui, fb, clip, item, icon_x, r.y + (r.height - APP_ICON_ROW) / 2, APP_ICON_ROW);
        let name_x = r.x + 38;
        let right = if l.kind_column { kind_x } else { l.columns.right() };
        let name = ui.fit(&item.app.name, DENSE, right - name_x - 12);
        ui.text(fb, clip, name_x, r.y, r.height, right - 8, &name, DENSE, pal().ink_primary);
        if l.kind_column {
            ui.text(fb, clip, kind_x, r.y, r.height, l.columns.right(), item.kind.label(), DENSE, pal().quiet);
        }
    }

    /// A tile of the Applications folder: the app's icon over its name.
    fn draw_app_tile(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect, item: &Item, r: Rect) {
        self.draw_app_icon(ui, fb, clip, item, r.x + (r.width - APP_ICON_TILE) / 2, r.y + 12, APP_ICON_TILE);
        let st = Style::new(Face::Sans, 12.0);
        let lines = ui.text.wrap_style(&item.app.name, st.scaled(ui.scale), ((r.width - 12) * ui.scale) as f32, 2);
        for (n, line) in lines.iter().enumerate() {
            let w = ui.measure(line, st);
            let y = r.y + 58 + n as i32 * 16;
            ui.text(fb, clip, r.x + (r.width - w) / 2, y, 16, r.right() - 4, line, st, pal().ink_primary);
        }
    }

    fn draw_row(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect, i: usize, e: &Entry, r: Rect) {
        let l = &self.layout;
        self.row_fill(ui, fb, clip, i, r);
        if let Some(item) = self.app_of(e) {
            return self.draw_app_row(ui, fb, clip, &item, r);
        }
        let faded = e.hidden || self.cut(e);
        let ink = if faded { pal().quiet } else { pal().ink_primary };
        let icon_ink = if e.is_dir() { pal().ink_primary } else { pal().quiet };
        ui.icon(
            fb,
            clip,
            file_icon(e.icon),
            r.x + 12,
            r.y + (r.height - 16) / 2,
            16,
            icon_ink,
            if faded { 150 } else { 255 },
        );
        let (size_x, kind_x) = l.column_x();
        let name_x = r.x + 38;
        if let Some((old, edit)) = &self.rename
            && *old == e.name
        {
            let f = Rect::new(name_x - 6, r.y + 3, size_x - name_x - 8, r.height - 6);
            ui.field(fb, clip, f, true);
            edit.draw(ui, fb, clip, Rect::new(f.x + 6, f.y, f.width - 12, f.height), DENSE, "", true);
        } else {
            let name = ui.fit(&e.name, DENSE, size_x - name_x - 12);
            ui.text(fb, clip, name_x, r.y, r.height, size_x - 8, &name, DENSE, ink);
        }
        let size = if e.is_dir() { "—".to_string() } else { size_text(e.size) };
        ui.text(fb, clip, size_x, r.y, r.height, size_x + 90, &size, DENSE, pal().quiet);
        if l.kind_column {
            let kind = if e.symlink && e.kind == Kind::Symlink {
                "Broken link"
            } else {
                poly_files::mime::description(e.mime)
            };
            let kind = ui.fit(kind, DENSE, l.columns.right() - kind_x - 8);
            ui.text(fb, clip, kind_x, r.y, r.height, l.columns.right(), &kind, DENSE, pal().quiet);
        }
    }

    fn draw_tile(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect, i: usize, e: &Entry, r: Rect) {
        self.row_fill(ui, fb, clip, i, r);
        if let Some(item) = self.app_of(e) {
            return self.draw_app_tile(ui, fb, clip, &item, r);
        }
        let faded = e.hidden || self.cut(e);
        let ink = if e.is_dir() { pal().ink_primary } else { pal().quiet };
        ui.icon(
            fb,
            clip,
            file_icon(e.icon),
            r.x + (r.width - 40) / 2,
            r.y + 12,
            40,
            ink,
            if faded { 150 } else { 230 },
        );
        let st = Style::new(Face::Sans, 12.0);
        if let Some((old, edit)) = &self.rename
            && *old == e.name
        {
            let f = Rect::new(r.x + 2, r.y + 60, r.width - 4, 26);
            ui.field(fb, clip, f, true);
            edit.draw(ui, fb, clip, Rect::new(f.x + 4, f.y, f.width - 8, f.height), st, "", true);
            return;
        }
        let lines = ui.text.wrap_style(&e.name, st.scaled(ui.scale), ((r.width - 12) * ui.scale) as f32, 2);
        for (n, line) in lines.iter().enumerate() {
            let w = ui.measure(line, st);
            let ink = if faded { pal().quiet } else { pal().ink_primary };
            ui.text(fb, clip, r.x + (r.width - w) / 2, r.y + 58 + n as i32 * 16, 16, r.right() - 4, line, st, ink);
        }
    }

    fn draw_confirm(&self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect, c: &Confirm) {
        let r = confirm_rect(self.size);
        ui.popover(fb, clip, r);
        let text = ui.fit(&c.text, DENSE_MEDIUM, r.width - 40);
        ui.text(fb, clip, r.x + 20, r.y + 16, 24, r.right() - 20, &text, DENSE_MEDIUM, pal().ink_primary);
        ui.text(fb, clip, r.x + 20, r.y + 40, 20, r.right() - 20, "This cannot be undone.", SMALL, pal().quiet);
        let (cancel, ok) = confirm_buttons(self.size);
        ui.round(fb, clip, cancel, FIELD_RADIUS, pal().ink, pal().ghost);
        ui.round(fb, clip, ok, FIELD_RADIUS, pal().danger, 255);
        for (b, label, ink) in [(cancel, "Cancel", pal().ink_primary), (ok, "Delete", 0xff_ffff)] {
            let w = ui.measure(label, DENSE_MEDIUM);
            ui.text(fb, clip, b.x + (b.width - w) / 2, b.y, b.height, b.right(), label, DENSE_MEDIUM, ink);
        }
    }
}

/// The confirmation's box, centred.
fn confirm_rect((w, h): (i32, i32)) -> Rect {
    Rect::new((w - 380) / 2, (h - 128) / 2, 380, 128)
}

fn confirm_buttons(size: (i32, i32)) -> (Rect, Rect) {
    let r = confirm_rect(size);
    let ok = Rect::new(r.right() - 20 - 90, r.bottom() - 16 - 30, 90, 30);
    (Rect::new(ok.x - 8 - 90, ok.y, 90, 30), ok)
}

impl App for FilesApp {
    fn event(&mut self, ev: AppEvent, cx: &mut Cx) {
        self.now_ms = cx.now_ms;
        match ev {
            AppEvent::Resized { width, height, .. } => {
                self.size = (width, height);
                self.layout = Layout::new(width, height);
                self.clamp_scroll();
                cx.redraw();
            }
            AppEvent::Key(k) => {
                if k.pressed {
                    self.status.clear();
                }
                self.key(k, cx);
            }
            AppEvent::Motion { x, y } => {
                let t = self.target_at(x, y);
                if let Some(m) = &mut self.menu {
                    let h = m.item_at(x, y);
                    if h != m.hover {
                        m.hover = h;
                        cx.redraw();
                    }
                    return;
                }
                let t = t.filter(|t| !matches!(t, Target::Background | Target::Search));
                if t != self.hover {
                    self.hover = t;
                    cx.redraw();
                }
                cx.cursor(match t {
                    Some(Target::Crumb(None)) => CursorShape::Text,
                    _ if self.layout.search.contains(x, y) => CursorShape::Text,
                    _ => CursorShape::Default,
                });
            }
            AppEvent::Leave => {
                if self.hover.take().is_some() {
                    cx.redraw();
                }
            }
            AppEvent::Button { button, pressed, x, y, clicks, mods } => {
                self.button(button, pressed, x, y, clicks, mods, cx);
            }
            AppEvent::Scroll { dy } => {
                self.scroll += (dy * 2.0) as i32;
                self.clamp_scroll();
                cx.redraw();
            }
            AppEvent::Paste(text) => {
                // Text pasted into a field.
                match self.focus {
                    Focus::Search => {
                        self.search.insert(text.lines().next().unwrap_or(""));
                        self.browser.set_filter(&self.search.text.clone());
                    }
                    Focus::Path => {
                        if let Some(e) = &mut self.path_edit {
                            e.insert(text.lines().next().unwrap_or(""));
                        }
                    }
                    Focus::Rename => {
                        if let Some((_, e)) = &mut self.rename {
                            e.insert(text.lines().next().unwrap_or(""));
                        }
                    }
                    Focus::List => {}
                }
                cx.redraw();
            }
            AppEvent::Focus(_) | AppEvent::Theme => cx.redraw(),
        }
    }

    fn draw(&mut self, fb: &mut dyn Framebuffer, ui: &Ui, _full: bool) -> Option<Vec<Rect>> {
        let all = Rect::new(0, 0, self.size.0 * ui.scale, self.size.1 * ui.scale);
        self.draw_all(ui, fb, all);
        self.crumb_cache = self.crumbs(ui).into_iter().map(|(c, r)| (c.path, r)).collect();
        None
    }
}
