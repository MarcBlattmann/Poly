//! The installer's window with a made-up system behind it: what it shows
//! for a package, the clicks and keys, the commands it runs one after the
//! other, the end.

use crate::view::{Host, InstallerApp, Pumped, SIZE, wrap};
use crate::{Request, absolute, parse_args, take_lines};
use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use poly_app::{App, AppEvent, Cx, KeyInput, Mods, Ui};
use poly_input::key::Key;
use poly_pkg::installer::{Action, Installer, Phase, Plan, Removal, Step};
use poly_pkg::{ar, deb};
use poly_shell::draw::Text;
use poly_wayland::compose::MemoryFramebuffer;
use poly_wayland::compose::Scale;

/// The colour theme is one setting for the whole process: the tests that
/// switch it take turns, or one sees the other's colours.
static THEME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the window did to the system, and what the system answers.
#[derive(Default)]
struct Fake {
    started: Vec<Vec<String>>,
    /// What the next looks at the running command give.
    script: Vec<Pumped>,
    changed: usize,
    opened: Vec<String>,
    reports: Vec<String>,
    refuse_start: bool,
    /// The AppImages whose files the window was asked to remove.
    deleted: Vec<String>,
    refuse_delete: bool,
    /// The AppImages installed (path, name) and run.
    placed: Vec<(String, String)>,
    ran: Vec<String>,
    refuse_place: bool,
    refuse_run: bool,
}

struct FakeHost(Rc<RefCell<Fake>>);

impl Host for FakeHost {
    fn start(&mut self, argv: &[String]) -> Result<(), String> {
        if self.0.borrow().refuse_start {
            return Err("/usr/bin/apt-get: not found".into());
        }
        self.0.borrow_mut().started.push(argv.to_vec());
        Ok(())
    }

    fn handles(&self) -> Vec<(u32, u16)> {
        alloc::vec![(7, 1)]
    }

    fn pump(&mut self) -> Pumped {
        let mut f = self.0.borrow_mut();
        if f.script.is_empty() { Pumped::default() } else { f.script.remove(0) }
    }

    fn apps_of(&mut self, name: &str) -> Vec<(String, String, Option<String>)> {
        assert_eq!(name, "galculator");
        alloc::vec![("galculator.desktop".into(), "Galculator".into(), Some("galculator".into()))]
    }

    fn place_image(&mut self, path: &str, image: &poly_pkg::appimage::Image) -> Result<crate::view::Placed, String> {
        let mut f = self.0.borrow_mut();
        f.placed.push((path.into(), image.name.clone()));
        if f.refuse_place {
            Err("no space left".into())
        } else {
            Ok(crate::view::Placed {
                id: "poly-appimage-hello-pixels.desktop".into(),
                icon: Some("/h/icon.png".into()),
            })
        }
    }

    fn run_image(&mut self, path: &str) -> Result<(), String> {
        let mut f = self.0.borrow_mut();
        f.ran.push(path.into());
        if f.refuse_run { Err("exec format error".into()) } else { Ok(()) }
    }

    fn delete_image(&mut self, slug: &str) -> Result<(), String> {
        let mut f = self.0.borrow_mut();
        f.deleted.push(slug.into());
        if f.refuse_delete { Err("permission denied".into()) } else { Ok(()) }
    }

    fn apps_changed(&mut self) {
        self.0.borrow_mut().changed += 1;
    }

    fn open(&mut self, id: &str) {
        self.0.borrow_mut().opened.push(id.into());
    }

    fn report(&mut self, line: &str) {
        self.0.borrow_mut().reports.push(line.into());
    }
}

const CONTROL: &[u8] = b"Package: galculator\nVersion: 2.1.4-2\nArchitecture: amd64\n\
Maintainer: Debian QA Group <packages@qa.debian.org>\nInstalled-Size: 1297\n\
Depends: libc6 (>= 2.34), libgtk-3-0t64, libquadmath0\n\
Description: scientific calculator\n A GTK 3 calculator.\n";

fn app(unconfigured: usize, have_lists: bool) -> (InstallerApp, Rc<RefCell<Fake>>) {
    let deb = deb::build(&[("control", CONTROL)], &[("usr/bin/galculator", b"\x7fELF")]);
    let member = deb::control_member(&deb).unwrap();
    let control = ar::data(&deb, &member).unwrap();
    let mut model = Installer::new("/home/poly/Downloads/galculator_2.1.4-2_amd64.deb", Ok(&deb), Some(control));
    model.base_root(None, true, have_lists, unconfigured);
    let fake = Rc::new(RefCell::new(Fake::default()));
    (InstallerApp::new(model, Box::new(FakeHost(Rc::clone(&fake)))), fake)
}

fn ui() -> Ui {
    Ui { text: Text::new(None), scale: Scale::from_int(1) }
}

/// Draw the window (which lays the buttons out) and click the one for
/// `action`.
fn click(app: &mut InstallerApp, action: Action) -> Cx {
    let mut fb = MemoryFramebuffer::new(SIZE.0 as u32, SIZE.1 as u32);
    app.draw(&mut fb, &ui(), true);
    let (r, _) = app.button_rects(&ui()).into_iter().find(|(_, a)| *a == action).expect("the button is there");
    let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
    let mut cx = Cx::default();
    for pressed in [true, false] {
        let ev = AppEvent::Button { button: poly_app::BTN_LEFT, pressed, x, y, clicks: 1, mods: Mods::default() };
        app.event(ev, &mut cx);
    }
    cx
}

fn key(app: &mut InstallerApp, key: Key) -> Cx {
    let mut cx = Cx::default();
    let k = KeyInput { code: 0, key: Some(key), ch: None, mods: Mods::default(), pressed: true, repeat: false };
    app.event(AppEvent::Key(k), &mut cx);
    cx
}

fn ended(ok: bool, output: &[&str]) -> Pumped {
    Pumped { output: output.iter().map(ToString::to_string).collect(), status: Vec::new(), ended: Some(ok) }
}

#[test]
fn one_click_installs_and_the_desktop_hears_of_the_new_app() {
    let (mut app, fake) = app(0, true);
    assert_eq!(app.title(), "Install galculator");
    assert_eq!(app.model.phase, Phase::Ready);
    assert!(app.handles().is_empty(), "nothing runs before the click");
    click(&mut app, Action::Install);
    assert_eq!(app.model.phase, Phase::Working(Step::Install));
    assert_eq!(fake.borrow().started.len(), 1);
    assert_eq!(fake.borrow().started[0].last().unwrap(), "/home/poly/Downloads/galculator_2.1.4-2_amd64.deb");
    assert_eq!(app.handles(), [(7, 1)]);
    assert!(app.deadline().is_some());

    // The package tools report, then end well.
    fake.borrow_mut().script = alloc::vec![
        Pumped {
            output: alloc::vec!["Unpacking galculator (2.1.4-2) ...".into()],
            status: alloc::vec!["pmstatus:galculator:50.0:Unpacking galculator (amd64)".into()],
            ended: None,
        },
        ended(true, &["Setting up galculator (2.1.4-2) ..."]),
    ];
    let mut cx = Cx::default();
    app.ready(7, 1, &mut cx);
    assert_eq!(app.model.percent, Some(66));
    assert_eq!(fake.borrow().changed, 0);
    app.tick(&mut cx);
    assert_eq!(app.model.phase, Phase::Done);
    assert_eq!(fake.borrow().changed, 1, "the desktop reads its apps again");
    assert_eq!(
        fake.borrow().reports,
        ["running /usr/bin/apt-get install", "installed galculator 2.1.4-2: Galculator (galculator.desktop)"]
    );
    assert_eq!(app.model.message(), "Galculator is installed. It is in the launcher and in Applications.");
    assert!(app.handles().is_empty() && app.deadline().is_none());

    // Open starts the app through the desktop and the window goes.
    click(&mut app, Action::Open);
    assert_eq!(fake.borrow().opened, ["galculator.desktop"]);
}

#[test]
fn a_new_disk_runs_three_commands_in_order() {
    let (mut app, fake) = app(251, false);
    // Enter is the main button.
    key(&mut app, Key::Enter);
    assert_eq!(app.model.phase, Phase::Working(Step::Configure));
    let mut cx = Cx::default();
    for _ in 0..3 {
        fake.borrow_mut().script = alloc::vec![ended(true, &[])];
        app.tick(&mut cx);
    }
    assert_eq!(app.model.phase, Phase::Done);
    let started: Vec<String> = fake.borrow().started.iter().map(|argv| argv.join(" ")).collect();
    assert!(started[0].ends_with("dpkg --force-confdef --force-confold --configure -a"), "{}", started[0]);
    assert!(started[1].ends_with(" update"), "{}", started[1]);
    assert!(started[2].contains(" install -- /home/poly/"), "{}", started[2]);
    assert_eq!(started.len(), 3);
}

#[test]
fn nothing_closes_the_window_while_the_tools_run() {
    let (mut app, fake) = app(0, true);
    // Before: Escape and Cancel close.
    assert!(format!("{:?}", key(&mut app, Key::Escape)).contains("quit: true"));
    click(&mut app, Action::Install);
    let mut cx = Cx::default();
    app.close(&mut cx);
    assert!(format!("{cx:?}").contains("quit: false"), "the close button waits");
    assert!(format!("{:?}", key(&mut app, Key::Escape)).contains("quit: false"));
    assert!(app.button_rects(&ui()).is_empty(), "no buttons while it runs");
    // A failure: the reason, and Close.
    fake.borrow_mut().script =
        alloc::vec![ended(false, &["E: Unable to correct problems, you have held broken packages."])];
    app.tick(&mut cx);
    assert!(matches!(&app.model.phase, Phase::Failed(why) if why.contains("held broken packages")));
    // The console hears what failed, with the tools' last words.
    let reports = fake.borrow().reports.clone();
    assert!(reports.iter().any(|l| l.starts_with("failed: The app could not be installed")), "{reports:?}");
    assert!(
        reports.iter().any(|l| l == "| E: Unable to correct problems, you have held broken packages."),
        "{reports:?}"
    );
    assert_eq!(fake.borrow().changed, 0);
    app.close(&mut cx);
    assert!(format!("{cx:?}").contains("quit: true"));
}

#[test]
fn a_command_that_cannot_start_is_a_failure() {
    let (mut app, fake) = app(0, true);
    fake.borrow_mut().refuse_start = true;
    click(&mut app, Action::Install);
    assert_eq!(app.model.phase, Phase::Failed("The app could not be installed: /usr/bin/apt-get: not found.".into()));
}

#[test]
fn draws_every_state_in_both_themes() {
    let _theme = THEME.lock().unwrap_or_else(|e| e.into_inner());
    for light in [false, true] {
        poly_shell::theme::set_light(light);
        let p = poly_shell::theme::pal();
        let (mut app, fake) = app(0, true);
        let mut fb = MemoryFramebuffer::new(SIZE.0 as u32, SIZE.1 as u32);
        app.draw(&mut fb, &ui(), true);
        assert_eq!(fb.pixel(4, 4), Some(p.s0), "the ground");
        // The main button is solid.
        let (install, _) = app.button_rects(&ui()).into_iter().find(|(_, a)| *a == Action::Install).unwrap();
        assert_eq!(fb.pixel((install.x + 8) as u32, (install.y + 4) as u32), Some(p.ink_primary));
        click(&mut app, Action::Install);
        app.draw(&mut fb, &ui(), true);
        fake.borrow_mut().script = alloc::vec![ended(true, &[])];
        app.tick(&mut Cx::default());
        app.draw(&mut fb, &ui(), true);
        // The bar is full when done.
        let bar_y = SIZE.1 - crate::view::MARGIN - crate::view::BUTTON_H - 22 + 3;
        assert_eq!(fb.pixel((SIZE.0 / 2) as u32, bar_y as u32), Some(p.ink_primary));
    }
    poly_shell::theme::set_light(false);
    // A file that is no package draws too.
    let model = Installer::new("/tmp/notes.txt", Ok(b"hello"), None);
    let mut app = InstallerApp::new(model, Box::new(FakeHost(Rc::default())));
    let mut fb = MemoryFramebuffer::new(SIZE.0 as u32, SIZE.1 as u32);
    app.draw(&mut fb, &ui(), true);
    assert_eq!(app.button_rects(&ui()).len(), 1);
    assert_eq!(app.title(), "Install notes.txt");
}

#[test]
fn text_wraps_to_the_width_and_the_lines_allowed() {
    let ui = ui();
    let text = "one two three four five six seven eight nine ten eleven twelve";
    let lines = wrap(&ui, text, poly_app::ui::DENSE, 120, 3);
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|l| ui.measure(l, poly_app::ui::DENSE) <= 120), "{lines:?}");
    assert!(lines[0].starts_with("one"));
    assert_eq!(wrap(&ui, "short", poly_app::ui::DENSE, 400, 3), ["short"]);
    assert!(wrap(&ui, "  ", poly_app::ui::DENSE, 400, 3).is_empty());
}

#[test]
fn paths_are_made_absolute_and_output_is_cut_into_lines() {
    assert_eq!(absolute("/tmp/a.deb", Some("/home/poly")), "/tmp/a.deb");
    assert_eq!(absolute("a.deb", Some("/home/poly/Downloads/")), "/home/poly/Downloads/a.deb");
    assert_eq!(absolute("./a.deb", Some("/home/poly")), "/home/poly/a.deb");
    assert_eq!(absolute("a.deb", None), "/a.deb");
    assert_eq!(absolute("a.deb", Some("relative")), "/a.deb");

    // The desktop starts the window; everyone else names only the file.
    assert_eq!(parse_args(["/tmp/a.deb"]), Some((false, Request::Install("/tmp/a.deb"))));
    assert_eq!(parse_args(["--window", "/tmp/a.deb"]), Some((true, Request::Install("/tmp/a.deb"))));
    assert_eq!(parse_args(["file:///tmp/a%20b.deb"]), Some((false, Request::Install("/tmp/a%20b.deb"))));
    // An app to uninstall is named by its desktop file ID.
    assert_eq!(
        parse_args(["--uninstall", "galculator.desktop"]),
        Some((false, Request::Uninstall("galculator.desktop")))
    );
    let entry = poly_shell::apps::uninstaller_entry("galculator.desktop");
    assert_eq!(
        parse_args(entry.argv.iter().skip(1).map(String::as_str)),
        Some((true, Request::Uninstall("galculator.desktop")))
    );
    assert_eq!(parse_args(["--uninstall"]), None);
    assert_eq!(parse_args(["--window"]), None);
    assert_eq!(parse_args([""]), None);
    let entry = poly_shell::apps::installer_entry("/tmp/a.deb");
    assert_eq!(parse_args(entry.argv.iter().skip(1).map(String::as_str)), Some((true, Request::Install("/tmp/a.deb"))));

    let mut buf = b"Reading package lists...\r\nBuilding\rdone\n\nhalf a li".to_vec();
    assert_eq!(take_lines(&mut buf), ["Reading package lists...", "Building", "done"]);
    assert_eq!(buf, b"half a li");
    buf.extend_from_slice(b"ne\n");
    assert_eq!(take_lines(&mut buf), ["half a line"]);
    assert!(buf.is_empty());
}

/// Whether the window asked to end.
fn quit(cx: &Cx) -> bool {
    format!("{cx:?}").contains("quit: true")
}

fn uninstall_app(plan: Result<Plan, String>) -> (InstallerApp, Rc<RefCell<Fake>>) {
    let model = Installer::for_removal("galculator.desktop", plan);
    let fake = Rc::new(RefCell::new(Fake::default()));
    (InstallerApp::new(model, Box::new(FakeHost(Rc::clone(&fake)))), fake)
}

fn galculator_plan() -> Plan {
    let info = poly_pkg::deb::Info::parse(std::str::from_utf8(CONTROL).unwrap()).unwrap();
    Plan {
        app: "Galculator".into(),
        removal: Removal::Package,
        package: Some(info),
        apps: alloc::vec![("galculator.desktop".into(), "Galculator".into())],
    }
}

#[test]
fn a_click_on_uninstall_removes_the_package_then_what_only_it_needed() {
    let (mut app, fake) = uninstall_app(Ok(galculator_plan()));
    assert_eq!(app.title(), "Uninstall Galculator");
    assert_eq!(app.model.phase, Phase::Ready);
    app.set_app_icon(Some("galculator".into()));
    assert!(app.handles().is_empty(), "nothing runs before the click");
    assert!(fake.borrow().started.is_empty());
    click(&mut app, Action::Uninstall);
    assert_eq!(app.model.phase, Phase::Working(Step::Remove));
    assert_eq!(&fake.borrow().started[0][fake.borrow().started[0].len() - 3..], ["remove", "--", "galculator"]);
    // The first command ends well: the second starts.
    fake.borrow_mut().script = alloc::vec![ended(true, &["Removing galculator (2.1.4-2) ..."])];
    let mut cx = Cx::default();
    app.tick(&mut cx);
    assert_eq!(app.model.phase, Phase::Working(Step::Autoremove));
    assert_eq!(fake.borrow().started[1].last().unwrap(), "autoremove");
    assert_eq!(fake.borrow().changed, 0, "the desktop hears when it is all over");
    fake.borrow_mut().script = alloc::vec![ended(true, &["Removing libquadmath0:amd64 (14.2.0-19) ..."])];
    app.tick(&mut cx);
    assert_eq!(app.model.phase, Phase::Done);
    assert_eq!(fake.borrow().changed, 1, "the desktop reads its apps again: the launcher entry is gone");
    assert_eq!(
        fake.borrow().reports,
        [
            "running /usr/bin/apt-get remove",
            "running /usr/bin/apt-get autoremove",
            "uninstalled Galculator: Galculator (galculator.desktop)"
        ]
    );
    assert_eq!(app.model.buttons(), [("Close", Action::Close)]);
    assert!(app.handles().is_empty() && app.deadline().is_none());
}

#[test]
fn cancel_and_escape_leave_the_app_alone() {
    let (mut app, fake) = uninstall_app(Ok(galculator_plan()));
    let cx = click(&mut app, Action::Close);
    assert!(quit(&cx));
    assert!(fake.borrow().started.is_empty() && fake.borrow().changed == 0);
    let (mut app, fake) = uninstall_app(Ok(galculator_plan()));
    assert!(quit(&key(&mut app, Key::Escape)));
    assert!(fake.borrow().started.is_empty());
    // A running removal cannot be closed.
    let (mut app, _) = uninstall_app(Ok(galculator_plan()));
    click(&mut app, Action::Uninstall);
    assert!(!quit(&key(&mut app, Key::Escape)));
}

#[test]
fn an_app_that_must_stay_says_why_and_offers_close() {
    let (mut app, fake) = uninstall_app(Err("chromium is part of Poly's base system.".into()));
    assert_eq!(app.model.buttons(), [("Close", Action::Close)]);
    let mut fb = MemoryFramebuffer::new(SIZE.0 as u32, SIZE.1 as u32);
    app.draw(&mut fb, &ui(), true);
    assert!(quit(&click(&mut app, Action::Close)));
    assert!(fake.borrow().started.is_empty());
}

#[test]
fn an_appimage_is_removed_by_the_window_itself_on_the_next_look() {
    let plan = Plan {
        app: "Hello Pixels".into(),
        removal: Removal::Image { slug: "hello-pixels".into(), size: 5_000_000 },
        package: None,
        apps: alloc::vec![("poly-appimage-hello-pixels.desktop".into(), "Hello Pixels".into())],
    };
    let (mut app, fake) = uninstall_app(Ok(plan));
    click(&mut app, Action::Uninstall);
    // No command: the window shows "Removing" first, then does it.
    assert_eq!(app.model.phase, Phase::Working(Step::Delete));
    assert!(fake.borrow().started.is_empty() && fake.borrow().deleted.is_empty());
    assert!(app.deadline().is_some());
    let mut cx = Cx::default();
    app.tick(&mut cx);
    assert_eq!(fake.borrow().deleted, ["hello-pixels"]);
    assert_eq!(app.model.phase, Phase::Done);
    assert_eq!(fake.borrow().changed, 1);

    // A refusal of the disk is the window's failure.
    let plan = Plan {
        app: "X".into(),
        removal: Removal::Image { slug: "x".into(), size: 1 },
        package: None,
        apps: Vec::new(),
    };
    let (mut app, fake) = uninstall_app(Ok(plan));
    fake.borrow_mut().refuse_delete = true;
    click(&mut app, Action::Uninstall);
    app.tick(&mut cx);
    assert_eq!(app.model.phase, Phase::Failed("The app's files could not be removed: permission denied.".into()));
    assert_eq!(fake.borrow().changed, 0);
}

#[test]
fn the_uninstall_button_is_the_danger_one_in_both_themes() {
    let _theme = THEME.lock().unwrap_or_else(|e| e.into_inner());
    for light in [false, true] {
        poly_shell::theme::set_light(light);
        let p = poly_shell::theme::pal();
        let (mut app, _) = uninstall_app(Ok(galculator_plan()));
        let mut fb = MemoryFramebuffer::new(SIZE.0 as u32, SIZE.1 as u32);
        app.draw(&mut fb, &ui(), true);
        let (r, _) = app.button_rects(&ui()).into_iter().find(|(_, a)| *a == Action::Uninstall).unwrap();
        // The button's corner-free edge, inside its box: the danger colour.
        assert_eq!(fb.pixel((r.x + 6) as u32, (r.y + r.height / 2) as u32), Some(p.danger));
        let (cancel, _) = app.button_rects(&ui()).into_iter().find(|(_, a)| *a == Action::Close).unwrap();
        assert_ne!(fb.pixel((cancel.x + 6) as u32, (cancel.y + cancel.height / 2) as u32), Some(p.danger));
    }
    poly_shell::theme::set_light(false);
}

fn image_app() -> (InstallerApp, Rc<RefCell<Fake>>) {
    let image = poly_pkg::appimage::Image {
        name: "Hello Pixels".into(),
        summary: "Draws a few pixels".into(),
        version: "1.2.3".into(),
        desktop: String::new(),
        icon: None,
        size: 5_000_000,
    };
    let model = Installer::for_image("/home/poly/Downloads/Hello.AppImage", Ok(image));
    let fake = Rc::new(RefCell::new(Fake::default()));
    (InstallerApp::new(model, Box::new(FakeHost(Rc::clone(&fake)))), fake)
}

#[test]
fn an_appimage_asks_run_or_install_and_installs_on_the_next_look() {
    let (mut app, fake) = image_app();
    assert_eq!(app.title(), "Install Hello Pixels");
    assert_eq!(app.button_rects(&ui()).len(), 3);
    click(&mut app, Action::Install);
    // The window shows "Installing" first; the copy is the next look's.
    assert_eq!(app.model.phase, Phase::Working(Step::Place));
    assert!(fake.borrow().placed.is_empty() && fake.borrow().started.is_empty());
    assert!(app.deadline().is_some());
    let mut cx = Cx::default();
    app.tick(&mut cx);
    assert_eq!(fake.borrow().placed, [("/home/poly/Downloads/Hello.AppImage".to_string(), "Hello Pixels".to_string())]);
    assert_eq!(app.model.phase, Phase::Done);
    assert_eq!(fake.borrow().changed, 1, "the launcher has it at once");
    assert_eq!(
        fake.borrow().reports,
        ["installed Hello Pixels 1.2.3: Hello Pixels (poly-appimage-hello-pixels.desktop)"]
    );
    assert_eq!(app.model.buttons(), [("Close", Action::Close), ("Open", Action::Open)]);
    let mut fb = MemoryFramebuffer::new(SIZE.0 as u32, SIZE.1 as u32);
    app.draw(&mut fb, &ui(), true);
    click(&mut app, Action::Open);
    assert_eq!(fake.borrow().opened, ["poly-appimage-hello-pixels.desktop"]);
}

#[test]
fn run_starts_the_appimage_where_it_is_and_installs_nothing() {
    let (mut app, fake) = image_app();
    let cx = click(&mut app, Action::Run);
    assert_eq!(fake.borrow().ran, ["/home/poly/Downloads/Hello.AppImage"]);
    assert!(quit(&cx));
    assert!(fake.borrow().placed.is_empty() && fake.borrow().changed == 0);
    // An AppImage that cannot start is the window's failure.
    let (mut app, fake) = image_app();
    fake.borrow_mut().refuse_run = true;
    let cx = click(&mut app, Action::Run);
    assert!(!quit(&cx));
    assert_eq!(app.model.phase, Phase::Failed("Hello Pixels could not be started: exec format error.".into()));
}

#[test]
fn an_appimage_that_cannot_be_copied_says_why() {
    let (mut app, fake) = image_app();
    fake.borrow_mut().refuse_place = true;
    click(&mut app, Action::Install);
    app.tick(&mut Cx::default());
    assert_eq!(app.model.phase, Phase::Failed("The app could not be installed: no space left.".into()));
    assert_eq!(fake.borrow().changed, 0);
    assert!(fake.borrow().reports.iter().any(|l| l.starts_with("failed: ")));
}
