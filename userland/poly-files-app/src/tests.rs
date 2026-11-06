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
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn setup() -> Setup {
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
    let fs = StdFs(root.clone());
    let exists = |p: &str| root.join(p.trim_start_matches('/')).is_dir();
    let places = poly_files::places::places("/home/m", None, "/dev/vda1 / ext2 rw 0 0\n", &exists);
    let mut app = FilesApp::new(
        Box::new(fs),
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
    assert!(app.go("/home/m"));
    let mut cx = Cx::default();
    app.event(AppEvent::Resized { width: 900, height: 580, scale: 1 }, &mut cx);
    Setup { app, root, launched }
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
