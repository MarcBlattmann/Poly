//! The Files window on a real temporary folder (std's file system behind
//! `poly-files`' trait): listing, keys, the menu's operations, the trash,
//! opening, drawing.

use crate::view::{FilesApp, Focus, Layout, Target, act, size_text};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use poly_app::{App, AppEvent, Cx, KeyInput, Mods, Ui};
use poly_files::{Fs, FsError, Kind, Meta, RawEntry};
use poly_input::key::Key;
use poly_shell::draw::Text;
use poly_wayland::compose::MemoryFramebuffer;
use std::path::PathBuf;

/// `std::fs` under a temporary root; paths are `/`-rooted inside it.
struct StdFs(PathBuf);

fn err(e: std::io::Error) -> FsError {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::NotFound => FsError::NotFound,
        K::AlreadyExists => FsError::AlreadyExists,
        K::PermissionDenied => FsError::PermissionDenied,
        K::DirectoryNotEmpty => FsError::NotEmpty,
        _ => FsError::Other,
    }
}

impl StdFs {
    fn p(&self, path: &str) -> PathBuf {
        self.0.join(path.trim_start_matches('/'))
    }

    fn meta(m: std::fs::Metadata) -> Meta {
        let kind = if m.is_dir() {
            Kind::Dir
        } else if m.file_type().is_symlink() {
            Kind::Symlink
        } else {
            Kind::File
        };
        Meta { kind, size: m.len(), mode: 0o644, device: 0, modified: None }
    }
}

impl Fs for StdFs {
    fn read_dir(&self, path: &str) -> Result<Vec<RawEntry>, FsError> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(self.p(path)).map_err(err)? {
            let e = e.map_err(err)?;
            let ft = e.file_type().map_err(err)?;
            let kind = if ft.is_dir() {
                Kind::Dir
            } else if ft.is_symlink() {
                Kind::Symlink
            } else {
                Kind::File
            };
            out.push(RawEntry { name: e.file_name().to_string_lossy().into_owned(), kind });
        }
        Ok(out)
    }
    fn stat(&self, path: &str) -> Result<Meta, FsError> {
        std::fs::metadata(self.p(path)).map(StdFs::meta).map_err(err)
    }
    fn lstat(&self, path: &str) -> Result<Meta, FsError> {
        std::fs::symlink_metadata(self.p(path)).map(StdFs::meta).map_err(err)
    }
    fn read_head(&self, path: &str, max: usize) -> Result<Vec<u8>, FsError> {
        let mut d = std::fs::read(self.p(path)).map_err(err)?;
        d.truncate(max);
        Ok(d)
    }
    fn write(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        std::fs::write(self.p(path), data).map_err(err)
    }
    fn create_new(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(self.p(path)).map_err(err)?;
        f.write_all(data).map_err(err)
    }
    fn create_dir(&self, path: &str) -> Result<(), FsError> {
        std::fs::create_dir(self.p(path)).map_err(err)
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        std::fs::rename(self.p(from), self.p(to)).map_err(err)
    }
    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        std::fs::remove_file(self.p(path)).map_err(err)
    }
    fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        std::fs::remove_dir(self.p(path)).map_err(err)
    }
}

fn temp() -> PathBuf {
    use core::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(std::format!(
        "poly-files-app-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Setup {
    app: FilesApp,
    root: PathBuf,
    launched: Rc<RefCell<Vec<Vec<String>>>>,
    /// The IDs of the apps the desktop was asked to start.
    opened: Rc<RefCell<Vec<String>>>,
    /// The IDs of the apps the desktop was asked to uninstall.
    uninstalled: Rc<RefCell<Vec<String>>>,
}

/// The desktop files under a test's root, as the launcher's reader sees
/// them.
struct RootApps(PathBuf);

impl poly_shell::apps::AppFiles for RootApps {
    fn list(&self, dir: &str) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.0.join(dir.trim_start_matches('/'))) else { return Vec::new() };
        entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(path.trim_start_matches('/'))).ok()
    }
}

/// What the launcher offers on a test's disk.
fn launcher_catalog(root: &std::path::Path) -> Vec<poly_shell::App> {
    poly_shell::apps::installed(&RootApps(root.to_path_buf()), &mut |_, _| {})
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn setup() -> Setup {
    setup_with(false)
}

/// Files with the Applications folder over the test's disk.
fn setup_apps() -> Setup {
    setup_with(true)
}

fn setup_with(applications: bool) -> Setup {
    let root = temp();
    for d in ["home/m/Documents", "home/m/Desktop", "home/m/Downloads", "tmp", "usr/share/applications"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("home/m/notes.txt"), "hello").unwrap();
    std::fs::write(root.join("home/m/photo.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    std::fs::write(root.join("home/m/.hidden"), "").unwrap();
    std::fs::write(
        root.join("usr/share/applications/viewer.desktop"),
        "[Desktop Entry]\nType=Application\nName=Viewer\nExec=viewer %f\nMimeType=image/png;\n",
    )
    .unwrap();
    let launched = Rc::new(RefCell::new(Vec::new()));
    let l = Rc::clone(&launched);
    let fs: Box<dyn Fs> = Box::new(StdFs(root.clone()));
    let catalog_root = root.clone();
    let (fs, folder) = if applications {
        crate::applications::overlay(
            fs,
            Box::new(move || launcher_catalog(&catalog_root)),
            Box::new(|| std::vec!["viewer.desktop".to_string()]),
        )
    } else {
        (fs, crate::applications::Shared::default())
    };
    let exists = |p: &str| root.join(p.trim_start_matches('/')).is_dir();
    let places = poly_files::places::places("/home/m", None, "/dev/vda1 / ext2 rw 0 0\n", &exists);
    let mut app = FilesApp::new(
        fs,
        "/home/m",
        "/home/m/.local/share/Trash",
        places,
        Box::new(move |argv: &[String]| {
            l.borrow_mut().push(argv.to_vec());
            Ok(())
        }),
    );
    app.handlers = crate::load_handlers(&StdFs(root.clone()), &["/usr/share/applications".into()]);
    app.clock = Box::new(|| 1_793_491_200);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let uninstalled = Rc::new(RefCell::new(Vec::new()));
    if applications {
        let u = Rc::clone(&uninstalled);
        app.uninstall_app = Box::new(move |item| {
            let line = poly_shell::applications::uninstall_request(item);
            match poly_wayland::launch::Control::parse(&line) {
                Some(poly_wayland::launch::Control::Uninstall { id }) => u.borrow_mut().push(id),
                other => return Err(std::format!("not an uninstall request: {other:?}")),
            }
            Ok(())
        });
        let o = Rc::clone(&opened);
        app.set_applications(
            folder,
            Box::new(move |item| {
                // What goes to the compositor: the launcher's app, by ID.
                let line = poly_shell::applications::open_request(item);
                match poly_wayland::launch::Control::parse(&line) {
                    Some(poly_wayland::launch::Control::Open { id }) => o.borrow_mut().push(id),
                    other => return Err(std::format!("not an open request: {other:?}")),
                }
                Ok(())
            }),
        );
    }
    assert!(app.go("/home/m"));
    let mut cx = Cx::default();
    app.event(AppEvent::Resized { width: 900, height: 580, scale: 1 }, &mut cx);
    Setup { app, root, launched, opened, uninstalled }
}

fn names(app: &FilesApp) -> Vec<String> {
    app.browser.rows().map(|e| e.name.clone()).collect()
}

fn key(app: &mut FilesApp, k: Option<Key>, ch: Option<char>, mods: Mods) {
    let mut cx = Cx::default();
    app.event(AppEvent::Key(KeyInput { code: 0, key: k, ch, mods, pressed: true, repeat: false }), &mut cx);
}

fn ctrl(app: &mut FilesApp, c: char) {
    key(app, None, Some(c), Mods { ctrl: true, ..Mods::default() });
}

fn typed(app: &mut FilesApp, s: &str) {
    for c in s.chars() {
        key(app, None, Some(c), Mods::default());
    }
}

#[test]
fn lists_folders_first_and_hides_dot_files() {
    let s = setup();
    assert_eq!(names(&s.app), ["Desktop", "Documents", "Downloads", "notes.txt", "photo.png"]);
    assert_eq!(s.app.title(), "Home");
    let mut s = s;
    ctrl(&mut s.app, 'h');
    assert!(names(&s.app).contains(&".hidden".to_string()));
    let labels: Vec<&str> = s.app.places.iter().map(|p| p.label.as_str()).collect();
    assert!(labels.contains(&"Home") && labels.contains(&"Trash") && labels.contains(&"Computer"), "{labels:?}");
}

#[test]
fn keys_move_open_and_go_up() {
    let mut s = setup();
    key(&mut s.app, Some(Key::ArrowDown), None, Mods::default());
    key(&mut s.app, Some(Key::ArrowDown), None, Mods::default());
    assert_eq!(s.app.browser.cursor_entry().unwrap().name, "Documents");
    key(&mut s.app, Some(Key::Enter), None, Mods::default());
    assert_eq!(s.app.browser.cwd(), "/home/m/Documents");
    key(&mut s.app, Some(Key::Backspace), None, Mods::default());
    assert_eq!(s.app.browser.cwd(), "/home/m");
    key(&mut s.app, Some(Key::ArrowLeft), None, Mods { alt: true, ..Mods::default() });
    assert_eq!(s.app.browser.cwd(), "/home/m/Documents", "back");
    // Ctrl+L and a path.
    ctrl(&mut s.app, 'l');
    assert_eq!(s.app.focus, Focus::Path);
    key(&mut s.app, Some(Key::Backspace), None, Mods::default());
    typed(&mut s.app, "/tmp");
    key(&mut s.app, Some(Key::Enter), None, Mods::default());
    assert_eq!(s.app.browser.cwd(), "/tmp");
}

#[test]
fn search_filters_and_type_ahead_jumps() {
    let mut s = setup();
    ctrl(&mut s.app, 'f');
    typed(&mut s.app, "pho");
    assert_eq!(names(&s.app), ["photo.png"]);
    key(&mut s.app, Some(Key::Escape), None, Mods::default());
    assert_eq!(names(&s.app).len(), 5);
    assert_eq!(s.app.focus, Focus::List);
    typed(&mut s.app, "n");
    assert_eq!(s.app.browser.cursor_entry().unwrap().name, "notes.txt");
}

#[test]
fn new_folder_rename_copy_paste_and_trash() {
    let mut s = setup();
    let mut cx = Cx::default();
    s.app.act(act::NEW_FOLDER, &mut cx);
    assert!(s.root.join("home/m/New Folder").is_dir());
    assert_eq!(s.app.focus, Focus::Rename);
    typed(&mut s.app, "Projects");
    key(&mut s.app, Some(Key::Enter), None, Mods::default());
    assert!(s.root.join("home/m/Projects").is_dir());
    assert_eq!(s.app.browser.cursor_entry().unwrap().name, "Projects");
    // Copy notes.txt, paste twice into the same folder.
    s.app.browser.select_name("notes.txt");
    ctrl(&mut s.app, 'c');
    ctrl(&mut s.app, 'v');
    assert!(s.root.join("home/m/notes (copy).txt").is_file(), "{:?}", names(&s.app));
    // Cut photo.png into Projects.
    s.app.browser.select_name("photo.png");
    ctrl(&mut s.app, 'x');
    s.app.go("/home/m/Projects");
    ctrl(&mut s.app, 'v');
    assert!(s.root.join("home/m/Projects/photo.png").is_file());
    assert!(!s.root.join("home/m/photo.png").exists());
    // Delete moves to the trash, with its info file.
    s.app.browser.select_name("photo.png");
    key(&mut s.app, Some(Key::Delete), None, Mods::default());
    assert!(!s.root.join("home/m/Projects/photo.png").exists());
    let trash = s.root.join("home/m/.local/share/Trash");
    assert!(trash.join("files/photo.png").is_file());
    let info = std::fs::read_to_string(trash.join("info/photo.png.trashinfo")).unwrap_or_else(|e| {
        panic!(
            "{e}: {:?}",
            std::fs::read_dir(trash.join("info")).map(|d| d.map(|e| e.unwrap().file_name()).collect::<Vec<_>>())
        )
    });
    assert!(info.contains("Path=/home/m/Projects/photo.png"), "{info}");
    // In the trash (the sidebar's place): restore it.
    s.app.go(poly_files::places::TRASH_URI);
    assert!(s.app.in_trash());
    assert_eq!(s.app.title(), "Trash");
    s.app.browser.select_name("photo.png");
    s.app.act(act::RESTORE, &mut cx);
    assert!(s.root.join("home/m/Projects/photo.png").is_file());
}

#[test]
fn shift_delete_asks_first() {
    let mut s = setup();
    s.app.browser.select_name("notes.txt");
    key(&mut s.app, Some(Key::Delete), None, Mods { shift: true, ..Mods::default() });
    assert!(s.app.confirm.is_some());
    assert!(s.root.join("home/m/notes.txt").exists());
    key(&mut s.app, Some(Key::Enter), None, Mods::default());
    assert!(!s.root.join("home/m/notes.txt").exists());
}

#[test]
fn opening_files_uses_their_apps() {
    let mut s = setup();
    s.app.open_path("/home/m/photo.png");
    assert_eq!(s.launched.borrow().last().unwrap(), &["viewer", "/home/m/photo.png"]);
    s.app.open_path("/home/m/notes.txt");
    assert_eq!(s.launched.borrow().last().unwrap(), &["/bin/poly-terminal", "-e", "vi", "/home/m/notes.txt"]);
    let mut cx = Cx::default();
    s.app.browser.select_name("Documents");
    s.app.act(act::OPEN_TERMINAL, &mut cx);
    assert_eq!(s.launched.borrow().last().unwrap(), &["/bin/poly-terminal", "--dir", "/home/m/Documents"]);
}

/// The `mimeapps.list` files of the spec's locations are read, the user's
/// before the system's (`$XDG_CONFIG_DIRS`, where Chromium's desktop names
/// its browser the default).
#[test]
fn the_default_browser_comes_from_the_system_list_and_the_user_can_change_it() {
    let mut s = setup();
    let app = |name: &str, exec: &str, mime: &str| {
        std::fs::write(
            s.root.join(std::format!("usr/share/applications/{name}.desktop")),
            std::format!(
                "[Desktop Entry]
Type=Application
Name={name}
Exec={exec}
MimeType={mime}
"
            ),
        )
        .unwrap();
    };
    app("browser", "/bin/bash /usr/local/bin/browser --window %U", "text/html;x-scheme-handler/https;");
    app("aaa-editor", "editor %F", "text/html;");
    std::fs::write(s.root.join("home/m/page.html"), "<!DOCTYPE html><title>x</title>").unwrap();
    let reload = |s: &mut Setup| {
        let fs = StdFs(s.root.clone());
        s.app.handlers = crate::load_handlers(&fs, &["/usr/share/applications".into()]);
        let lists = crate::mimeapps_paths("/home/m/.config", None, "/home/m/.local/share", None);
        s.app.defaults = crate::load_mimeapps(&fs, &lists);
    };
    // No list: the first handler by id.
    reload(&mut s);
    assert_eq!(s.app.defaults, None);
    s.app.open_path("/home/m/page.html");
    assert_eq!(s.launched.borrow().last().unwrap(), &["editor", "/home/m/page.html"]);
    // The system's list (/etc/xdg) names the browser.
    std::fs::create_dir_all(s.root.join("etc/xdg")).unwrap();
    std::fs::write(
        s.root.join("etc/xdg/mimeapps.list"),
        "[Default Applications]
text/html=browser.desktop
x-scheme-handler/https=browser.desktop
",
    )
    .unwrap();
    reload(&mut s);
    s.app.open_path("/home/m/page.html");
    assert_eq!(
        s.launched.borrow().last().unwrap(),
        &["/bin/bash", "/usr/local/bin/browser", "--window", "/home/m/page.html"]
    );
    // The user's own list wins over it.
    std::fs::create_dir_all(s.root.join("home/m/.config")).unwrap();
    std::fs::write(
        s.root.join("home/m/.config/mimeapps.list"),
        "[Default Applications]
text/html=aaa-editor.desktop
",
    )
    .unwrap();
    reload(&mut s);
    s.app.open_path("/home/m/page.html");
    assert_eq!(s.launched.borrow().last().unwrap(), &["editor", "/home/m/page.html"]);
}

#[test]
fn mimeapps_lists_are_looked_up_in_the_specs_order() {
    assert_eq!(
        crate::mimeapps_paths("/home/m/.config", None, "/home/m/.local/share", None),
        [
            "/home/m/.config/mimeapps.list",
            "/etc/xdg/mimeapps.list",
            "/home/m/.local/share/applications/mimeapps.list",
            "/usr/local/share/applications/mimeapps.list",
            "/usr/share/applications/mimeapps.list",
        ]
    );
    // Variables with several directories (relative ones ignored), in order.
    assert_eq!(
        crate::mimeapps_paths("/c", Some("/a:rel:/b"), "/d", Some("/x")),
        [
            "/c/mimeapps.list",
            "/a/mimeapps.list",
            "/b/mimeapps.list",
            "/d/applications/mimeapps.list",
            "/x/applications/mimeapps.list"
        ]
    );
    // An empty variable is the default.
    assert_eq!(crate::mimeapps_paths("/c", Some(""), "/d", Some(""))[1], "/etc/xdg/mimeapps.list");
}

#[test]
fn clicks_select_and_double_clicks_open() {
    let mut s = setup();
    let mut cx = Cx::default();
    let r = s.app.item_rect(1);
    let (x, y) = (r.x + 50, r.y + 10);
    assert_eq!(s.app.target_at(x, y), Some(Target::Item(1)));
    let click =
        |clicks| AppEvent::Button { button: poly_app::BTN_LEFT, pressed: true, x, y, clicks, mods: Mods::default() };
    s.app.event(click(1), &mut cx);
    assert_eq!(s.app.browser.selected_rows(), [1]);
    s.app.event(click(2), &mut cx);
    assert_eq!(s.app.browser.cwd(), "/home/m/Documents");
    // The sidebar's Home.
    let home = s.app.places.iter().position(|p| p.label == "Home").unwrap();
    let mut y = 0;
    while s.app.target_at(30, y) != Some(Target::Place(home)) && y < 580 {
        y += 1;
    }
    s.app.event(
        AppEvent::Button { button: poly_app::BTN_LEFT, pressed: true, x: 30, y, clicks: 1, mods: Mods::default() },
        &mut cx,
    );
    assert_eq!(s.app.browser.cwd(), "/home/m");
    // Right click on the background: the folder's menu.
    let l = Layout::new(900, 580);
    s.app.event(
        AppEvent::Button {
            button: poly_app::BTN_RIGHT,
            pressed: true,
            x: l.list.x + 10,
            y: l.list.bottom() - 5,
            clicks: 1,
            mods: Mods::default(),
        },
        &mut cx,
    );
    let m = s.app.menu.clone().expect("a menu");
    assert_eq!(m.items[0].label, "New folder");
}

#[test]
fn draws_in_both_views_and_themes() {
    let mut s = setup();
    let ui = Ui { text: Text::new(None), scale: 1 };
    for light in [false, true] {
        poly_shell::theme::set_light(light);
        let mut fb = MemoryFramebuffer::new(900, 580);
        s.app.draw(&mut fb, &ui, true);
        let p = poly_shell::theme::pal();
        assert_eq!(fb.pixel(100, 560), Some(p.s0), "the sidebar's ground");
        assert_eq!(fb.pixel(600, 300), Some(p.s1), "the page");
        ctrl(&mut s.app, '2');
        s.app.draw(&mut fb, &ui, true);
        ctrl(&mut s.app, '1');
    }
    poly_shell::theme::set_light(false);
}

#[test]
fn sizes_read_well() {
    assert_eq!(size_text(1), "1 byte");
    assert_eq!(size_text(900), "900 bytes");
    assert_eq!(size_text(1536), "1.5 KB");
    assert_eq!(size_text(20 * 1024 * 1024), "20 MB");
}

#[test]
fn programs_are_found_on_the_path() {
    let there = |p: &str| p == "/usr/bin/viewer";
    assert_eq!(crate::resolve("viewer", Some("/bin:/usr/bin"), &there), "/usr/bin/viewer");
    assert_eq!(crate::resolve("/x/y", None, &there), "/x/y");
    assert_eq!(crate::resolve("nope", None, &there), "nope");
}

#[test]
fn the_path_bar_edits_and_draws_with_the_bitmap_font() {
    let mut s = setup();
    let ui = Ui { text: Text::new(None), scale: 1 };
    let mut fb = MemoryFramebuffer::new(900, 580);
    s.app.draw(&mut fb, &ui, true);
    let mut cx = Cx::default();
    s.app.event(
        AppEvent::Button {
            button: poly_app::BTN_LEFT,
            pressed: true,
            x: 600,
            y: 300,
            clicks: 1,
            mods: Mods::default(),
        },
        &mut cx,
    );
    s.app.draw(&mut fb, &ui, true);
    ctrl(&mut s.app, 'l');
    s.app.draw(&mut fb, &ui, true);
    for c in "/tmp".chars() {
        key(&mut s.app, None, Some(c), Mods::default());
        s.app.draw(&mut fb, &ui, true);
    }
    key(&mut s.app, Some(Key::Enter), None, Mods::default());
    s.app.draw(&mut fb, &ui, true);
    assert_eq!(s.app.browser.cwd(), "/tmp");
}

// ---- the Applications folder (ADR 0175) --------------------------------------

const EDITOR: &str = "[Desktop Entry]\nType=Application\nName=Editor\nComment=Write text\nExec=/usr/bin/editor %F\n";
const HELPER: &str = "[Desktop Entry]\nType=Application\nName=Helper\nExec=h\nNoDisplay=true\n";

fn button(button: u32, x: i32, y: i32, clicks: u32) -> AppEvent {
    AppEvent::Button { button, pressed: true, x, y, clicks, mods: Mods::default() }
}

#[test]
fn applications_lists_exactly_what_the_launcher_offers() {
    let mut s = setup_apps();
    let apps = s.root.join("usr/share/applications");
    std::fs::write(apps.join("helper.desktop"), HELPER).unwrap();
    assert!(s.app.go("applications:///"));
    assert!(s.app.in_apps());
    assert_eq!(s.app.title(), "Applications");
    assert_eq!(names(&s.app), ["Files", "Notifications", "Settings", "Terminal", "Viewer"]);
    let launcher: Vec<String> = launcher_catalog(&s.root).into_iter().map(|a| a.name).collect();
    assert_eq!(names(&s.app), launcher, "the launcher's own list, hidden entries left out by its rule");
    assert_eq!(s.app.count_text(), "5 apps");
    assert_eq!(s.app.status_line(), "5 apps installed");
    // An app is installed, another removed: the next look shows it.
    std::fs::write(apps.join("editor.desktop"), EDITOR).unwrap();
    std::fs::remove_file(apps.join("viewer.desktop")).unwrap();
    key(&mut s.app, Some(Key::F5), None, Mods::default());
    assert_eq!(names(&s.app), ["Editor", "Files", "Notifications", "Settings", "Terminal"]);
    assert_eq!(names(&s.app), launcher_catalog(&s.root).into_iter().map(|a| a.name).collect::<Vec<_>>());
    // Without the overlay there is no such folder.
    let mut plain = setup();
    assert!(!plain.app.go("applications:///"));
    assert!(!plain.app.places.iter().any(|p| p.label == "Applications"));
}

#[test]
fn applications_is_a_place_and_a_location() {
    let mut s = setup_apps();
    let labels: Vec<&str> = s.app.places.iter().map(|p| p.label.as_str()).collect();
    let at = labels.iter().position(|l| *l == "Applications").expect("a place");
    assert_eq!(labels[at + 1], "Computer", "{labels:?}");
    // A click on the place.
    let mut cx = Cx::default();
    let mut y = 0;
    while s.app.target_at(30, y) != Some(Target::Place(at)) && y < 580 {
        y += 1;
    }
    s.app.event(button(poly_app::BTN_LEFT, 30, y, 1), &mut cx);
    assert!(s.app.in_apps());
    // The path bar shows and takes its location; it has no parent.
    ctrl(&mut s.app, 'l');
    assert_eq!(s.app.path_edit.as_ref().map(|e| e.text.as_str()), Some("applications:///"));
    key(&mut s.app, Some(Key::Escape), None, Mods::default());
    key(&mut s.app, Some(Key::Backspace), None, Mods::default());
    assert!(s.app.in_apps(), "up goes nowhere");
    assert!(!s.app.browser.can_go_up());
    // Back to where we were and forward again.
    key(&mut s.app, Some(Key::ArrowLeft), None, Mods { alt: true, ..Mods::default() });
    assert_eq!(s.app.browser.cwd(), "/home/m");
    key(&mut s.app, Some(Key::ArrowRight), None, Mods { alt: true, ..Mods::default() });
    assert!(s.app.in_apps());
    // Its name typed in the path bar, in any case.
    s.app.go("/tmp");
    ctrl(&mut s.app, 'l');
    key(&mut s.app, Some(Key::Backspace), None, Mods::default());
    typed(&mut s.app, "applications");
    key(&mut s.app, Some(Key::Enter), None, Mods::default());
    assert!(s.app.in_apps());
    assert_eq!(s.app.browser.len(), 5);
}

#[test]
fn opening_an_app_asks_the_desktop_to_start_it() {
    let mut s = setup_apps();
    s.app.go("applications:///");
    // Type-ahead to Settings (the shell's own window: no program Files
    // could start), Enter.
    typed(&mut s.app, "se");
    assert_eq!(s.app.browser.cursor_entry().unwrap().name, "Settings");
    assert_eq!(
        s.app.status_line(),
        "Settings: Poly app, built into Poly. Display, keyboard, wallpaper, sound, time zone"
    );
    key(&mut s.app, Some(Key::Enter), None, Mods::default());
    assert_eq!(*s.opened.borrow(), ["poly:Settings"]);
    assert_eq!(s.app.status_line(), "Opened Settings");
    // Enter held down (the key repeats) does not start it again.
    let held =
        KeyInput { code: 0, key: Some(Key::Enter), ch: None, mods: Mods::default(), pressed: true, repeat: true };
    s.app.event(AppEvent::Key(held), &mut Cx::default());
    assert_eq!(s.opened.borrow().len(), 1);
    // A double click on a desktop file's app.
    let row = names(&s.app).iter().position(|n| n == "Viewer").unwrap();
    let r = s.app.item_rect(row);
    let mut cx = Cx::default();
    for clicks in [1, 2] {
        s.app.event(button(poly_app::BTN_LEFT, r.x + 50, r.y + 10, clicks), &mut cx);
    }
    assert_eq!(*s.opened.borrow(), ["poly:Settings", "viewer.desktop"]);
    assert!(s.launched.borrow().is_empty(), "Files itself starts nothing: the desktop does");
    // The details of the selected app: its kind and where it comes from.
    key(&mut s.app, Some(Key::ArrowUp), None, Mods::default());
    assert_eq!(s.app.status_line(), "Terminal: Poly app, built into Poly. A shell in a window");
    s.app.browser.select_name("Viewer");
    assert_eq!(s.app.status_line(), "Viewer: Linux app, from /usr/share/applications/viewer.desktop");
    // An item that is gone since the listing.
    s.app.open_path("applications:/Gone");
    assert_eq!(s.app.status_line(), "Gone: no such app");
    assert_eq!(s.opened.borrow().len(), 2);
}

#[test]
fn search_works_on_applications() {
    let mut s = setup_apps();
    s.app.go("applications:///");
    ctrl(&mut s.app, 'f');
    typed(&mut s.app, "TI");
    assert_eq!(names(&s.app), ["Notifications", "Settings"]);
    let model: Vec<String> = s.app.apps.borrow().matching("TI").into_iter().map(|i| i.label.clone()).collect();
    assert_eq!(names(&s.app), model, "the model's search rule");
    // Enter in the search field opens the first match.
    typed(&mut s.app, "n");
    assert_eq!(names(&s.app), ["Settings"]);
    key(&mut s.app, Some(Key::Enter), None, Mods::default());
    assert_eq!(*s.opened.borrow(), ["poly:Settings"]);
}

#[test]
fn nothing_in_applications_can_be_changed() {
    let mut s = setup_apps();
    s.app.go("applications:///");
    let mut cx = Cx::default();
    s.app.browser.select_name("Viewer");
    key(&mut s.app, Some(Key::F2), None, Mods::default());
    assert!(s.app.rename.is_none());
    assert_eq!(s.app.focus, Focus::List);
    key(&mut s.app, Some(Key::Delete), None, Mods::default());
    key(&mut s.app, Some(Key::Delete), None, Mods { shift: true, ..Mods::default() });
    assert!(s.app.confirm.is_none(), "nothing asks to delete an app");
    ctrl(&mut s.app, 'c');
    ctrl(&mut s.app, 'x');
    assert!(s.app.clip.is_none());
    for a in [act::NEW_FOLDER, act::TRASH, act::DELETE, act::RENAME, act::CUT, act::PASTE, act::OPEN_TERMINAL] {
        s.app.act(a, &mut cx);
    }
    assert!(s.root.join("usr/share/applications/viewer.desktop").is_file());
    assert_eq!(names(&s.app), ["Files", "Notifications", "Settings", "Terminal", "Viewer"]);
    assert!(s.launched.borrow().is_empty() && s.opened.borrow().is_empty() && s.uninstalled.borrow().is_empty());
    // The New folder button is not there.
    let l = Layout::new(900, 580);
    assert_eq!(s.app.target_at(l.new_folder.x + 5, l.new_folder.y + 5), None);
    // An item's menu: what the model offers, nothing dead. Poly's own apps
    // only open.
    let r = s.app.item_rect(0);
    s.app.event(button(poly_app::BTN_RIGHT, r.x + 50, r.y + 10, 1), &mut cx);
    let labels: Vec<String> = s.app.menu.take().expect("a menu").items.into_iter().map(|i| i.label).collect();
    assert_eq!(labels, ["Open"]);
    s.app.act(act::UNINSTALL, &mut cx);
    assert!(s.uninstalled.borrow().is_empty(), "Poly's own apps cannot be uninstalled");
    // The folder's menu.
    s.app.event(button(poly_app::BTN_RIGHT, l.list.x + 10, l.list.bottom() - 5, 1), &mut cx);
    let labels: Vec<String> = s.app.menu.take().expect("a menu").items.into_iter().map(|i| i.label).collect();
    assert_eq!(labels, ["Refresh"]);
    // The file system refuses too, whoever asks.
    assert_eq!(s.app.fs.remove_file("applications:/Viewer"), Err(FsError::Unsupported));
    assert_eq!(s.app.fs.create_dir("applications:/New"), Err(FsError::Unsupported));
    assert_eq!(s.app.fs.rename("applications:/Viewer", "/tmp/v"), Err(FsError::Unsupported));
    assert_eq!(s.app.fs.copy_file("/home/m/notes.txt", "applications:/notes.txt"), Err(FsError::Unsupported));
    assert_eq!(s.app.fs.stat("applications:/Viewer").map(|m| m.kind), Ok(Kind::File));
    assert_eq!(s.app.fs.stat("applications:").map(|m| m.kind), Ok(Kind::Dir));
    assert_eq!(s.app.fs.stat("applications:/Gone"), Err(FsError::NotFound));
    assert!(s.app.fs.exists("applications:/Files") && !s.app.fs.exists("applications:/Gone"));
    assert!(s.app.fs.exists("/home/m/notes.txt"), "the disk underneath is untouched");
}

#[test]
fn an_app_the_installer_owns_can_be_uninstalled_from_its_menu() {
    let mut s = setup_apps();
    s.app.go("applications:///");
    let mut cx = Cx::default();
    let r = s.app.item_rect(4);
    s.app.event(button(poly_app::BTN_RIGHT, r.x + 50, r.y + 10, 1), &mut cx);
    let menu = s.app.menu.clone().expect("a menu");
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Open", "Uninstall"]);
    assert!(menu.items.iter().all(|i| i.enabled));
    // The row asks the desktop to show the installer's window; nothing is
    // removed from here.
    s.app.act(act::UNINSTALL, &mut cx);
    assert_eq!(*s.uninstalled.borrow(), ["viewer.desktop"]);
    assert_eq!(s.app.status, "Asked to uninstall Viewer");
    assert!(s.root.join("usr/share/applications/viewer.desktop").is_file());
    assert!(s.opened.borrow().is_empty());
    // Without a desktop to ask the status says so and nothing happens.
    s.app.uninstall_app = Box::new(|_| Err("the desktop cannot be reached".into()));
    s.app.act(act::UNINSTALL, &mut cx);
    assert_eq!(s.app.status, "Viewer: the desktop cannot be reached");
}

#[test]
fn applications_draw_each_apps_own_icon_in_both_views() {
    let mut s = setup_apps();
    s.app.go("applications:///");
    let ui = Ui { text: Text::new(None), scale: 1 };
    let mut fb = MemoryFramebuffer::new(900, 580);
    // Poly's own artwork, as the launcher draws it, at each view's size.
    let mut icons = poly_shell::appicon::AppIcons::default();
    let files = names(&s.app).iter().position(|n| n == "Files").unwrap();
    for (view, size) in [('1', crate::view::APP_ICON_ROW), ('2', crate::view::APP_ICON_TILE)] {
        ctrl(&mut s.app, view);
        s.app.draw(&mut fb, &ui, true);
        let art = icons.get("poly-files", size as u32).expect("Files' icon");
        let r = s.app.item_rect(files);
        let (x, y) = if view == '1' {
            (r.x + 12 - (size - 16) / 2, r.y + (r.height - size) / 2)
        } else {
            (r.x + (r.width - size) / 2, r.y + 12)
        };
        let (cx, cy) = (art.width / 2, art.height / 2);
        let want = art.pixels[(cy * art.width + cx) as usize];
        assert_eq!(want >> 24, 255, "the artwork is opaque at its centre");
        let (ox, oy) = (x + (size - art.width as i32) / 2, y + (size - art.height as i32) / 2);
        assert_eq!(fb.pixel((ox + cx as i32) as u32, (oy + cy as i32) as u32), Some(want & 0xff_ffff), "view {view}");
    }
    // At twice the scale too, and with every row selected.
    let ui2 = Ui { text: Text::new(None), scale: 2 };
    let mut fb2 = MemoryFramebuffer::new(1800, 1160);
    ctrl(&mut s.app, 'a');
    s.app.draw(&mut fb2, &ui2, true);
    assert_eq!(s.app.status_line(), "5 of 5 selected");
}
