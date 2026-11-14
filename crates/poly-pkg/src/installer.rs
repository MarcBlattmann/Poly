//! The installer's model (ADR 0132: one installer for every package
//! format; ADR 0173): what its window says about a package file and which
//! command runs next. No I/O: the program (`poly-installer`) reads the
//! file, runs the commands and feeds their output in, so every state is
//! tested on the host.
//!
//! A package goes through: [`Phase::Ready`] (the window says what the
//! package is; **Install** waits for a click), [`Phase::Working`] (`dpkg
//! --configure -a` if the base root's own packages are not configured yet,
//! which is once per disk; `apt-get update` if it has no package lists yet;
//! then `apt-get install` of the file, with the progress `apt` reports),
//! then [`Phase::Done`] or [`Phase::Failed`]. A file that is no package Poly installs stays in
//! [`Phase::Refused`] with the reason.

use crate::appimage::Image;
use crate::apt::{self, Progress};
use crate::deb::{self, Info};
use crate::format::{self, Format};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The descriptor number `apt-get` finds its progress pipe under, unless
/// the program has it under another ([`Installer::status_fd`]).
pub const STATUS_FD: u32 = 3;

/// How many lines of the commands' output are kept (the window shows the
/// last few, a failure's report the last many).
pub const LOG_LINES: usize = 400;

/// A command of an installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `dpkg --configure -a`.
    Configure,
    /// `apt-get update`.
    Update,
    /// `apt-get install <file>`.
    Install,
    /// `apt-get remove <package>`.
    Remove,
    /// `apt-get autoremove`: the packages that only the removed one needed.
    Autoremove,
    /// Take an installed AppImage's files away (the program does it).
    Delete,
    /// Copy an AppImage into the user's folder and add its launcher entry
    /// (the program does it).
    Place,
}

/// What the window is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Install the package file.
    Install,
    /// Remove an installed app (ADR 0180).
    Uninstall,
    /// Install an AppImage, or run it without (ADR 0180).
    Image,
}

/// Where the installer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The file is not something Poly installs: why.
    Refused(String),
    /// The package is shown; nothing has run.
    Ready,
    /// A command runs.
    Working(Step),
    /// Installed.
    Done,
    /// A command failed: what went wrong.
    Failed(String),
}

/// A button of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start the installation.
    Install,
    /// Close the window.
    Close,
    /// Start the app that was installed.
    Open,
    /// Remove the app (ADR 0180).
    Uninstall,
    /// Start the AppImage from where it is, without installing it.
    Run,
}

/// What an uninstall removes (ADR 0180).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Removal {
    /// A Debian package (`Installer::package` has what dpkg knows of it).
    Package,
    /// An AppImage Poly installed: its slug ([`crate::appimage::slug`]) and
    /// the size of its file in bytes.
    Image { slug: String, size: u64 },
}

/// What the program found out about an app to uninstall.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The app's name.
    pub app: String,
    pub removal: Removal,
    /// The package, for [`Removal::Package`].
    pub package: Option<Info>,
    /// The launcher entries that go with it (desktop file ID, name).
    pub apps: Vec<(String, String)>,
}

/// The installer for one package file.
#[derive(Clone, Debug)]
pub struct Installer {
    /// What the window is for.
    pub task: Task,
    /// The package file's absolute path (for an uninstall: the app's
    /// desktop file ID).
    pub path: String,
    /// For an uninstall: the app's name, the window's heading.
    pub title: Option<String>,
    /// For an uninstall: which kind of install it removes.
    pub removal: Option<Removal>,
    /// For an AppImage: what was read from it.
    pub image: Option<Image>,
    /// What the package says about itself, if it could be read.
    pub package: Option<Info>,
    /// The version of this package installed already, if any.
    pub installed: Option<String>,
    pub phase: Phase,
    /// How far the running command is, 0 to 100, once it says.
    pub percent: Option<u8>,
    /// What the running command is doing, in its words.
    pub activity: String,
    /// The last [`LOG_LINES`] lines the commands wrote.
    pub log: Vec<String>,
    /// The launcher entries the package brought (desktop file ID, name),
    /// known once it is installed.
    pub apps: Vec<(String, String)>,
    /// The descriptor number the commands write their progress to.
    pub status_fd: u32,
    /// Whether the base root has package lists (`apt-get update` ran).
    have_lists: bool,
    /// How many of the base root's packages are not configured yet.
    unconfigured: usize,
    /// How many of them the running `dpkg --configure -a` has started on.
    configured: usize,
}

/// Why the base root cannot install packages at all.
pub const NO_TOOLS: &str = "This Poly disk was made before it could install apps. Start Poly with a fresh disk (Fresh disk in the start menu, or --fresh-disk) to get the package tools.";

impl Installer {
    /// The installer for the file at `path`, from what was read of it:
    /// `head`, its first bytes (at least [`format::HEAD`] and [`deb::HEAD`],
    /// or all of it), and `control`, the control member's bytes if `head`
    /// said where it is ([`deb::control_member`]). `Err` in `head`: why the
    /// file could not be read at all.
    pub fn new(path: &str, head: Result<&[u8], &str>, control: Option<&[u8]>) -> Installer {
        let mut it = Installer {
            task: Task::Install,
            path: path.to_string(),
            title: None,
            removal: None,
            image: None,
            package: None,
            installed: None,
            phase: Phase::Ready,
            percent: None,
            activity: String::new(),
            log: Vec::new(),
            apps: Vec::new(),
            status_fd: STATUS_FD,
            have_lists: false,
            unconfigured: 0,
            configured: 0,
        };
        let head = match head {
            Ok(head) => head,
            Err(why) => {
                it.phase = Phase::Refused(alloc::format!("The file cannot be read: {why}."));
                return it;
            }
        };
        match format::detect(head) {
            Some(Format::Deb) => {}
            Some(other) => {
                let why = other.not_yet().unwrap_or("This file is read by Installer::for_image.");
                it.phase = Phase::Refused(why.to_string());
                return it;
            }
            None => {
                it.phase = Phase::Refused("This file is not an app package Poly knows.".to_string());
                return it;
            }
        }
        let info = deb::control_member(head).and_then(|m| {
            let data = control.ok_or(crate::Error::Truncated)?;
            let files = deb::control_files(&m.name, data)?;
            let (_, text) = files.iter().find(|(n, _)| n == "control").ok_or(crate::Error::NoControl)?;
            Info::parse(core::str::from_utf8(text).map_err(|_| crate::Error::Corrupt)?)
        });
        match info {
            Ok(info) => it.package = Some(info),
            Err(e) => it.phase = Phase::Refused(e.describe().to_string()),
        }
        it
    }

    /// The window for the AppImage at `path`: what was read of it, or why
    /// it cannot be (ADR 0180). **Run** starts it from where it is,
    /// **Install** copies it into the user's folder.
    pub fn for_image(path: &str, image: Result<Image, crate::Error>) -> Installer {
        let mut it = Installer::new(path, Err("nothing to read"), None);
        it.task = Task::Image;
        it.phase = Phase::Ready;
        it.have_lists = true;
        match image {
            Ok(image) => it.image = Some(image),
            Err(e) => it.phase = Phase::Refused(e.describe().to_string()),
        }
        it
    }

    /// The window that uninstalls the app with desktop file ID `id`: what
    /// the program found out about it (`Err`: why it cannot be removed).
    pub fn for_removal(id: &str, plan: Result<Plan, String>) -> Installer {
        let mut it = Installer::new(id, Err("nothing to read"), None);
        it.task = Task::Uninstall;
        it.phase = Phase::Ready;
        match plan {
            Ok(plan) => {
                it.title = Some(plan.app);
                it.removal = Some(plan.removal);
                it.package = plan.package;
                it.apps = plan.apps;
                // The base root's state does not matter for a removal.
                it.have_lists = true;
            }
            Err(why) => it.phase = Phase::Refused(why),
        }
        it
    }

    /// What the base root has: the installed version of this package (from
    /// dpkg's status file), whether `apt-get` is there, whether its package
    /// lists are, and how many of its packages are not configured yet
    /// ([`crate::dpkgdb::unconfigured`]).
    pub fn base_root(&mut self, installed: Option<String>, have_apt: bool, have_lists: bool, unconfigured: usize) {
        self.installed = installed;
        self.have_lists = have_lists;
        self.unconfigured = unconfigured;
        if !have_apt && self.phase == Phase::Ready {
            self.phase = Phase::Refused(NO_TOOLS.to_string());
        }
    }

    /// The package's name as the window's heading: `Package` as written.
    pub fn name(&self) -> &str {
        if let Some(title) = &self.title {
            return title;
        }
        if let Some(image) = &self.image {
            return &image.name;
        }
        self.package.as_ref().map_or_else(|| file_name(&self.path), |p| p.name.as_str())
    }

    /// The line under the heading: the package's own summary.
    pub fn summary(&self) -> &str {
        match &self.image {
            Some(image) => image.summary.as_str(),
            None => self.package.as_ref().map_or("", |p| p.summary.as_str()),
        }
    }

    /// The installation failed in a way only the program knows (a file it
    /// could not write): say so.
    pub fn fail(&mut self, why: &str) {
        self.phase = Phase::Failed(why.to_string());
    }

    /// Whether the window is for removing an app.
    pub fn uninstalling(&self) -> bool {
        self.task == Task::Uninstall
    }

    /// The facts the window lists: label and value.
    pub fn facts(&self) -> Vec<(&'static str, String)> {
        if let Some(image) = &self.image {
            let mut out = Vec::new();
            if !image.version.is_empty() {
                out.push(("Version", image.version.clone()));
            }
            out.push(("Size", deb::human_kib(image.size.div_ceil(1024))));
            out.push(("Kind", alloc::format!("Linux app, {}", Format::AppImage.name())));
            return out;
        }
        if let Some(Removal::Image { size, .. }) = &self.removal {
            return alloc::vec![
                ("Size", deb::human_kib(size.div_ceil(1024))),
                ("Kind", alloc::format!("Linux app, {}", Format::AppImage.name())),
            ];
        }
        let Some(p) = &self.package else { return Vec::new() };
        let mut out = alloc::vec![("Version", p.version.clone())];
        if let Some(v) = &self.installed {
            let value = if *v == p.version { alloc::format!("{v} (this version)") } else { v.clone() };
            out.push(("Installed", value));
        }
        if let Some(kib) = p.installed_kib {
            out.push(("Size", deb::human_kib(kib)));
        }
        if !p.publisher().is_empty() {
            out.push(("From", p.publisher().to_string()));
        }
        out.push(("Kind", alloc::format!("Linux app, {} ({})", Format::Deb.name(), p.architecture)));
        if !p.depends.is_empty() && !self.uninstalling() {
            out.push(("Needs", needs(&p.depends)));
        }
        out
    }

    /// The sentence under the facts: what happens next, is happening, or
    /// happened.
    pub fn message(&self) -> String {
        if self.uninstalling() {
            return self.removal_message();
        }
        match &self.phase {
            Phase::Refused(why) | Phase::Failed(why) => why.clone(),
            Phase::Ready if self.image.is_some() => {
                "Install copies the app to your user folder and adds it to the launcher and to Applications. Run starts it from where it is."
                    .to_string()
            }
            Phase::Ready => {
                "Installing adds the app and what it needs from Debian's archive to this computer.".to_string()
            }
            Phase::Working(Step::Configure) if self.activity.is_empty() => {
                "Preparing this computer for Linux apps (once)\u{2026}".to_string()
            }
            Phase::Working(Step::Configure) => alloc::format!("Preparing this computer (once): {}", self.activity),
            Phase::Working(Step::Update) => "Getting the list of Debian's packages\u{2026}".to_string(),
            Phase::Working(Step::Install) if self.activity.is_empty() => "Installing\u{2026}".to_string(),
            Phase::Working(Step::Place) => "Installing\u{2026}".to_string(),
            // The steps of an uninstall ([`Installer::removal_message`]).
            Phase::Working(_) => self.activity.clone(),
            Phase::Done => match self.apps.first() {
                Some((_, name)) => alloc::format!("{name} is installed. It is in the launcher and in Applications."),
                None => alloc::format!("{} is installed.", self.name()),
            },
        }
    }

    fn removal_message(&self) -> String {
        let app = self.name();
        match &self.phase {
            Phase::Refused(why) | Phase::Failed(why) => why.clone(),
            Phase::Ready => match &self.removal {
                Some(Removal::Image { .. }) => alloc::format!(
                    "Uninstalling removes {app} and its files from your user folder. Your own files stay."
                ),
                _ => alloc::format!(
                    "Uninstalling removes {app} and what only it needed from this computer. Your own files stay."
                ),
            },
            Phase::Working(Step::Autoremove) if self.activity.is_empty() => {
                alloc::format!("Removing what only {app} needed\u{2026}")
            }
            Phase::Working(Step::Autoremove) => self.activity.clone(),
            Phase::Working(Step::Remove) if !self.activity.is_empty() => self.activity.clone(),
            Phase::Working(_) => alloc::format!("Removing {app}\u{2026}"),
            Phase::Done => alloc::format!("{app} is uninstalled. It is gone from the launcher and from Applications."),
        }
    }

    /// The window's buttons, the main one last (it is drawn on the right).
    pub fn buttons(&self) -> Vec<(&'static str, Action)> {
        if self.uninstalling() {
            return match &self.phase {
                Phase::Ready => alloc::vec![("Cancel", Action::Close), ("Uninstall", Action::Uninstall)],
                Phase::Working(_) => Vec::new(),
                _ => alloc::vec![("Close", Action::Close)],
            };
        }
        match &self.phase {
            Phase::Refused(_) | Phase::Failed(_) => alloc::vec![("Close", Action::Close)],
            Phase::Ready if self.image.is_some() => {
                alloc::vec![("Cancel", Action::Close), ("Run", Action::Run), ("Install", Action::Install)]
            }
            Phase::Ready => {
                let same = self.package.as_ref().is_some_and(|p| self.installed.as_deref() == Some(p.version.as_str()));
                alloc::vec![
                    ("Cancel", Action::Close),
                    (if same { "Install again" } else { "Install" }, Action::Install)
                ]
            }
            Phase::Working(_) => Vec::new(),
            Phase::Done if self.apps.is_empty() => alloc::vec![("Close", Action::Close)],
            Phase::Done => alloc::vec![("Close", Action::Close), ("Open", Action::Open)],
        }
    }

    /// Whether a command runs (the window cannot be closed meanwhile:
    /// `dpkg` must not be cut off half way).
    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Working(_))
    }

    fn command(&self, step: Step) -> Vec<String> {
        match step {
            Step::Configure => apt::configure(),
            Step::Update => apt::update(self.status_fd),
            Step::Install => apt::install(&self.path, self.status_fd),
            Step::Remove => apt::remove(self.package.as_ref().map_or("", |p| p.name.as_str()), self.status_fd),
            Step::Autoremove => apt::autoremove(self.status_fd),
            // The program does it.
            Step::Delete | Step::Place => Vec::new(),
        }
    }

    /// The step after `done` (`None`: the first), if one is left.
    fn next_step(&self, done: Option<Step>) -> Option<Step> {
        let all: Vec<(Step, bool)> = match &self.removal {
            None if self.image.is_some() => alloc::vec![(Step::Place, true)],
            None => alloc::vec![
                (Step::Configure, self.unconfigured > 0),
                (Step::Update, !self.have_lists),
                (Step::Install, true)
            ],
            Some(Removal::Package) => alloc::vec![(Step::Remove, true), (Step::Autoremove, true)],
            Some(Removal::Image { .. }) => alloc::vec![(Step::Delete, true)],
        };
        let from = done.map_or(0, |d| all.iter().position(|(s, _)| *s == d).map_or(all.len(), |i| i + 1));
        all[from..].iter().find(|(_, needed)| *needed).map(|(s, _)| *s)
    }

    fn begin(&mut self, step: Step) -> Vec<String> {
        self.phase = Phase::Working(step);
        self.percent = None;
        self.activity.clear();
        self.command(step)
    }

    /// **Install** was clicked: the first command to run, if the package
    /// can be installed.
    pub fn start(&mut self) -> Option<Vec<String>> {
        let known =
            self.package.is_some() || self.image.is_some() || matches!(self.removal, Some(Removal::Image { .. }));
        if self.phase != Phase::Ready || !known {
            return None;
        }
        let first = self.next_step(None)?;
        Some(self.begin(first))
    }

    /// The running command wrote `line` (its output or error stream).
    pub fn output(&mut self, line: &str) {
        let line = line.trim_end();
        if line.is_empty() {
            return;
        }
        if self.log.len() == LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(line.to_string());
        // dpkg configuring the base root says which package it is at.
        if self.phase == Phase::Working(Step::Configure)
            && let Some(package) = apt::setting_up(line)
        {
            self.configured += 1;
            self.activity = package.to_string();
            let total = self.unconfigured.max(self.configured) as u64;
            self.percent = Some((self.configured as u64 * 100 / total).min(99) as u8);
        }
    }

    /// The running command wrote `line` to its status descriptor.
    pub fn status(&mut self, line: &str) {
        let Phase::Working(step) = self.phase else { return };
        match apt::parse_status(line) {
            // The download is the first third of an installation, `dpkg`'s
            // work the rest; an update is all download.
            Some(Progress::Download { percent, text }) => {
                self.percent = Some(if step == Step::Install { percent / 3 } else { percent });
                self.activity = alloc::format!("Downloading: {}", text.trim());
            }
            // A removal is two commands, a half of the bar each.
            Some(Progress::Install { percent, text }) if self.uninstalling() => {
                let half = (u32::from(percent.min(100)) / 2) as u8;
                self.percent = Some(if step == Step::Autoremove { 50 + half } else { half });
                self.activity = text.trim().to_string();
            }
            Some(Progress::Install { percent, text }) => {
                self.percent = Some(33 + (u32::from(percent) * 67 / 100) as u8);
                self.activity = text.trim().to_string();
            }
            Some(Progress::Error { package, text }) => self.output(&alloc::format!("E: {package}: {text}")),
            None => {}
        }
    }

    /// The running command ended (`ok`: with exit status 0). Returns the
    /// next command, if there is one. After the last, `apps` (the launcher
    /// entries the package brought: desktop file ID and name) says what
    /// **Open** starts.
    pub fn finished(&mut self, ok: bool, apps: impl FnOnce(&str) -> Vec<(String, String)>) -> Option<Vec<String>> {
        let Phase::Working(step) = self.phase else { return None };
        if !ok {
            let what = match step {
                Step::Configure => "This computer could not be prepared for Linux apps",
                Step::Update => "The list of Debian's packages could not be downloaded",
                Step::Install => "The app could not be installed",
                Step::Remove => "The app could not be uninstalled",
                Step::Autoremove => "What only the app needed could not be removed",
                Step::Delete => "The app's files could not be removed",
                Step::Place => "The app could not be installed",
            };
            let lines: Vec<&str> = self.log.iter().map(String::as_str).collect();
            let why = match (step, apt::dpkg_error(&lines), apt::error_line(lines.iter().copied())) {
                // apt's own last word about a failed dpkg run names no cause.
                (Step::Configure, Some(dpkg), _) | (_, Some(dpkg), None) => dpkg,
                (_, _, Some(apt)) => apt.to_string(),
                _ => "the package tools stopped".to_string(),
            };
            self.phase = Phase::Failed(alloc::format!("{what}: {why}."));
            return None;
        }
        match step {
            Step::Configure | Step::Update => {
                if step == Step::Configure {
                    self.unconfigured = 0;
                } else {
                    self.have_lists = true;
                }
                let next = self.next_step(Some(step))?;
                Some(self.begin(next))
            }
            Step::Remove | Step::Autoremove => match self.next_step(Some(step)) {
                Some(next) => Some(self.begin(next)),
                None => {
                    self.percent = Some(100);
                    self.installed = None;
                    self.phase = Phase::Done;
                    None
                }
            },
            Step::Delete => {
                self.percent = Some(100);
                self.phase = Phase::Done;
                None
            }
            Step::Install | Step::Place => {
                let name = self.package.as_ref().map(|p| p.name.clone()).unwrap_or_default();
                self.apps = apps(&name);
                self.installed = self.package.as_ref().map(|p| p.version.clone());
                self.percent = Some(100);
                self.phase = Phase::Done;
                None
            }
        }
    }
}

/// The dependencies as a short list: the first few names and how many more.
fn needs(depends: &[String]) -> String {
    const SHOWN: usize = 4;
    let mut text = depends.iter().take(SHOWN).map(String::as_str).collect::<Vec<_>>().join(", ");
    if depends.len() > SHOWN {
        text.push_str(&alloc::format!(" and {} more", depends.len() - SHOWN));
    }
    text
}

/// The last component of a path.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ar;

    const CONTROL: &[u8] = b"Package: galculator\nVersion: 2.1.4-2\nArchitecture: amd64\n\
Maintainer: Debian QA Group <packages@qa.debian.org>\nInstalled-Size: 1297\n\
Depends: libc6 (>= 2.34), libgdk-pixbuf-2.0-0, libglib2.0-0t64, libgtk-3-0t64, libpango-1.0-0, libx\n\
Description: scientific calculator\n A GTK 3 calculator.\n";

    fn package() -> (Vec<u8>, Vec<u8>) {
        let deb = deb::build(&[("control", CONTROL)], &[("usr/bin/galculator", b"\x7fELF")]);
        let m = deb::control_member(&deb).unwrap();
        let control = ar::data(&deb, &m).unwrap().to_vec();
        (deb, control)
    }

    fn ready() -> Installer {
        let (deb, control) = package();
        let mut it = Installer::new("/home/poly/Downloads/galculator_2.1.4-2_amd64.deb", Ok(&deb), Some(&control));
        it.base_root(None, true, false, 0);
        it
    }

    #[test]
    fn the_window_says_what_the_package_is() {
        let it = ready();
        assert_eq!(it.phase, Phase::Ready);
        assert_eq!(it.name(), "galculator");
        assert_eq!(it.summary(), "scientific calculator");
        assert_eq!(
            it.facts(),
            [
                ("Version", "2.1.4-2".to_string()),
                ("Size", "1.2 MB".to_string()),
                ("From", "Debian QA Group".to_string()),
                ("Kind", "Linux app, Debian package (amd64)".to_string()),
                ("Needs", "libc6, libgdk-pixbuf-2.0-0, libglib2.0-0t64, libgtk-3-0t64 and 2 more".to_string()),
            ]
        );
        assert_eq!(it.buttons(), [("Cancel", Action::Close), ("Install", Action::Install)]);
        assert!(!it.busy());
    }

    #[test]
    fn the_first_install_fetches_the_lists_then_installs() {
        let mut it = ready();
        let update = it.start().unwrap();
        assert_eq!(update.last().map(String::as_str), Some("update"));
        assert_eq!(it.phase, Phase::Working(Step::Update));
        assert!(it.busy() && it.buttons().is_empty());
        // A second click does nothing.
        assert_eq!(it.start(), None);
        it.status("dlstatus:1:50.0:Retrieving file 1 of 2");
        assert_eq!(it.percent, Some(50));
        let install = it.finished(true, |_| Vec::new()).unwrap();
        assert_eq!(install.last().map(String::as_str), Some("/home/poly/Downloads/galculator_2.1.4-2_amd64.deb"));
        assert_eq!(it.phase, Phase::Working(Step::Install));
        assert_eq!(it.percent, None, "a new command starts from nothing");
        it.status("dlstatus:1:90:Retrieving file 3 of 3");
        assert_eq!(it.percent, Some(30));
        it.status("pmstatus:galculator:50.0:Unpacking galculator (amd64)");
        assert_eq!((it.percent, it.message().as_str()), (Some(66), "Unpacking galculator (amd64)"));
        it.output("Setting up galculator (2.1.4-2) ...\n");
        let next = it.finished(true, |name| {
            assert_eq!(name, "galculator");
            alloc::vec![("galculator.desktop".to_string(), "Galculator".to_string())]
        });
        assert_eq!(next, None);
        assert_eq!(it.phase, Phase::Done);
        assert_eq!(it.percent, Some(100));
        assert_eq!(it.message(), "Galculator is installed. It is in the launcher and in Applications.");
        assert_eq!(it.buttons(), [("Close", Action::Close), ("Open", Action::Open)]);
        assert_eq!(it.installed.as_deref(), Some("2.1.4-2"));
    }

    #[test]
    fn with_the_lists_there_it_installs_at_once() {
        let (deb, control) = package();
        let mut it = Installer::new("/tmp/g.deb", Ok(&deb), Some(&control));
        it.base_root(Some("2.1.4-2".into()), true, true, 0);
        assert_eq!(it.buttons()[1], ("Install again", Action::Install));
        assert!(it.facts().contains(&("Installed", "2.1.4-2 (this version)".to_string())));
        let cmd = it.start().unwrap();
        assert!(cmd.contains(&"install".to_string()));
        assert_eq!(it.phase, Phase::Working(Step::Install));
        // A package without a launcher entry is still done.
        it.finished(true, |_| Vec::new());
        assert_eq!(it.message(), "galculator is installed.");
        assert_eq!(it.buttons(), [("Close", Action::Close)]);
    }

    #[test]
    fn a_failure_says_what_apt_said() {
        let mut it = ready();
        it.start();
        it.output("Err:1 http://snapshot.debian.org trixie InRelease");
        it.output("E: Failed to fetch http://snapshot.debian.org/x  Temporary failure resolving 'snapshot.debian.org'");
        assert_eq!(it.finished(false, |_| Vec::new()), None);
        let Phase::Failed(why) = &it.phase else { panic!("{:?}", it.phase) };
        assert!(why.starts_with("The list of Debian's packages could not be downloaded: Failed to fetch"), "{why}");
        assert!(why.ends_with("'snapshot.debian.org'."), "{why}");
        assert_eq!(it.buttons(), [("Close", Action::Close)]);
        assert_eq!(it.start(), None);

        let (deb, control) = package();
        let mut it = Installer::new("/tmp/g.deb", Ok(&deb), Some(&control));
        it.base_root(None, true, true, 0);
        it.start();
        it.status("pmerror:galculator:40:dependency problems - leaving unconfigured");
        it.finished(false, |_| Vec::new());
        assert_eq!(
            it.phase,
            Phase::Failed(
                "The app could not be installed: galculator: dependency problems - leaving unconfigured.".into()
            )
        );
    }

    #[test]
    fn other_files_are_refused_with_the_reason() {
        let rpm = [0xed, 0xab, 0xee, 0xdb, 3, 0, 0, 0];
        let it = Installer::new("/tmp/app.rpm", Ok(&rpm), None);
        assert!(matches!(&it.phase, Phase::Refused(why) if why.contains("RPM")));
        assert_eq!(it.name(), "app.rpm");
        assert_eq!(it.buttons(), [("Close", Action::Close)]);
        assert!(it.facts().is_empty());

        let it = Installer::new("/tmp/notes.txt", Ok(b"hello"), None);
        assert_eq!(it.phase, Phase::Refused("This file is not an app package Poly knows.".into()));

        let it = Installer::new("/tmp/gone.deb", Err("no such file"), None);
        assert_eq!(it.phase, Phase::Refused("The file cannot be read: no such file.".into()));

        // A download that was cut off.
        let (deb, _) = package();
        let it = Installer::new("/tmp/cut.deb", Ok(&deb[..200]), None);
        assert_eq!(it.phase, Phase::Refused(crate::Error::Truncated.describe().into()));

        // A disk without the package tools.
        let mut it = ready();
        it.base_root(None, false, false, 0);
        assert_eq!(it.phase, Phase::Refused(NO_TOOLS.into()));
        assert_eq!(it.start(), None);
    }

    #[test]
    fn a_new_disk_is_configured_first_once() {
        let (deb, control) = package();
        let mut it = Installer::new("/tmp/g.deb", Ok(&deb), Some(&control));
        it.status_fd = 9;
        it.base_root(None, true, false, 4);
        assert_eq!(it.start().unwrap(), apt::configure());
        assert_eq!(it.phase, Phase::Working(Step::Configure));
        assert!(it.message().starts_with("Preparing this computer for Linux apps"));
        it.output("Setting up libc-bin (2.41-12+deb13u4) ...");
        it.output(
            "update-alternatives: using /usr/bin/which.debianutils to provide /usr/bin/which (which) in auto mode",
        );
        it.output("Setting up debianutils (5.23.2) ...");
        assert_eq!((it.percent, it.message().as_str()), (Some(50), "Preparing this computer (once): debianutils"));
        // Then the lists, then the package, with the program's descriptor.
        let update = it.finished(true, |_| Vec::new()).unwrap();
        assert!(update.contains(&"APT::Status-Fd=9".to_string()) && update.contains(&"update".to_string()));
        let install = it.finished(true, |_| Vec::new()).unwrap();
        assert!(install.contains(&"install".to_string()));
        assert_eq!(it.finished(true, |_| Vec::new()), None);
        assert_eq!(it.phase, Phase::Done);

        // A script that fails stops everything, with dpkg's words.
        let mut it = Installer::new("/tmp/g.deb", Ok(&deb), Some(&control));
        it.base_root(None, true, true, 2);
        it.start();
        it.output("Setting up base-files (13.8+deb13u7) ...");
        it.output("chown: invalid group: 'root:mail'");
        it.output("dpkg: error processing package base-files (--configure):");
        it.output(" installed base-files package post-installation script subprocess returned error exit status 1");
        assert_eq!(it.finished(false, |_| Vec::new()), None);
        assert_eq!(
            it.phase,
            Phase::Failed(
                "This computer could not be prepared for Linux apps: base-files could not be set up (chown: invalid group: 'root:mail')."
                    .into()
            )
        );
    }

    fn galculator_plan() -> Plan {
        let info = Info::parse("Package: galculator\nVersion: 2.1.4-2\nArchitecture: amd64\nMaintainer: Debian QA Group <packages@qa.debian.org>\nInstalled-Size: 1297\nDescription: scientific calculator\n A GTK 3 calculator.\n").unwrap();
        Plan {
            app: "Galculator".into(),
            removal: Removal::Package,
            package: Some(info),
            apps: alloc::vec![("galculator.desktop".to_string(), "Galculator".to_string())],
        }
    }

    #[test]
    fn an_uninstall_asks_first_then_removes_then_autoremoves() {
        let mut it = Installer::for_removal("galculator.desktop", Ok(galculator_plan()));
        it.status_fd = 5;
        assert_eq!(it.phase, Phase::Ready);
        assert!(it.uninstalling());
        assert_eq!((it.name(), it.summary()), ("Galculator", "scientific calculator"));
        assert_eq!(
            it.facts(),
            [
                ("Version", "2.1.4-2".to_string()),
                ("Size", "1.2 MB".to_string()),
                ("From", "Debian QA Group".to_string()),
                ("Kind", "Linux app, Debian package (amd64)".to_string()),
            ]
        );
        assert_eq!(
            it.message(),
            "Uninstalling removes Galculator and what only it needed from this computer. Your own files stay."
        );
        assert_eq!(it.buttons(), [("Cancel", Action::Close), ("Uninstall", Action::Uninstall)]);
        // Nothing happened yet; a click starts apt's remove.
        let remove = it.start().unwrap();
        assert_eq!(&remove[remove.len() - 3..], ["remove", "--", "galculator"]);
        assert!(remove.contains(&"APT::Status-Fd=5".to_string()));
        assert_eq!(it.phase, Phase::Working(Step::Remove));
        assert!(it.busy() && it.buttons().is_empty());
        assert_eq!(it.message(), "Removing Galculator\u{2026}");
        it.status("pmstatus:galculator:50.0:Removing galculator (amd64)");
        assert_eq!((it.percent, it.message().as_str()), (Some(25), "Removing galculator (amd64)"));
        // Then what only it needed.
        let auto = it.finished(true, |_| panic!("nothing is read after a removal")).unwrap();
        assert_eq!(auto.last().map(String::as_str), Some("autoremove"));
        assert_eq!(it.phase, Phase::Working(Step::Autoremove));
        assert_eq!(it.message(), "Removing what only Galculator needed\u{2026}");
        it.status("pmstatus:libquadmath0:100.0:Removing libquadmath0:amd64 (amd64)");
        assert_eq!(it.percent, Some(100));
        assert_eq!(it.finished(true, |_| panic!("nothing is read after a removal")), None);
        assert_eq!(it.phase, Phase::Done);
        assert_eq!(it.message(), "Galculator is uninstalled. It is gone from the launcher and from Applications.");
        assert_eq!(it.buttons(), [("Close", Action::Close)]);
        assert_eq!(it.installed, None);
        // The launcher entries that went stay known, for the desktop's rescan.
        assert_eq!(it.apps, [("galculator.desktop".to_string(), "Galculator".to_string())]);
    }

    #[test]
    fn an_uninstall_that_apt_refuses_says_why() {
        let mut it = Installer::for_removal("galculator.desktop", Ok(galculator_plan()));
        it.start();
        it.output("E: Could not get lock /var/lib/dpkg/lock-frontend");
        assert_eq!(it.finished(false, |_| Vec::new()), None);
        assert_eq!(
            it.phase,
            Phase::Failed("The app could not be uninstalled: Could not get lock /var/lib/dpkg/lock-frontend.".into())
        );
        assert_eq!(it.buttons(), [("Close", Action::Close)]);
        // The first command failing leaves the second unstarted.
        assert_eq!(it.start(), None);
    }

    #[test]
    fn an_app_that_cannot_be_uninstalled_is_refused_with_the_reason() {
        let it = Installer::for_removal("chromium.desktop", Err("chromium is part of Poly's base system.".into()));
        assert!(it.uninstalling());
        assert_eq!(it.phase, Phase::Refused("chromium is part of Poly's base system.".into()));
        assert_eq!(it.message(), "chromium is part of Poly's base system.");
        assert_eq!(it.buttons(), [("Close", Action::Close)]);
        assert!(it.facts().is_empty());
        let mut it = it;
        assert_eq!(it.start(), None);
    }

    #[test]
    fn an_appimage_is_removed_by_the_program_in_one_step() {
        let plan = Plan {
            app: "Hello Pixels".into(),
            removal: Removal::Image { slug: "hello-pixels".into(), size: 5_000_000 },
            package: None,
            apps: alloc::vec![("poly-appimage-hello-pixels.desktop".to_string(), "Hello Pixels".to_string())],
        };
        let mut it = Installer::for_removal("poly-appimage-hello-pixels.desktop", Ok(plan));
        assert_eq!(it.facts(), [("Size", "4.7 MB".to_string()), ("Kind", "Linux app, AppImage".to_string())]);
        assert_eq!(
            it.message(),
            "Uninstalling removes Hello Pixels and its files from your user folder. Your own files stay."
        );
        // No command: the program deletes the files itself.
        assert_eq!(it.start(), Some(Vec::new()));
        assert_eq!(it.phase, Phase::Working(Step::Delete));
        assert_eq!(it.message(), "Removing Hello Pixels\u{2026}");
        assert_eq!(it.finished(true, |_| Vec::new()), None);
        assert_eq!(it.phase, Phase::Done);
        assert_eq!(it.percent, Some(100));
        // A failure to delete is the program's words.
        let plan = Plan {
            app: "X".into(),
            removal: Removal::Image { slug: "x".into(), size: 1 },
            package: None,
            apps: Vec::new(),
        };
        let mut it = Installer::for_removal("poly-appimage-x.desktop", Ok(plan));
        it.start();
        it.output("E: /home/poly/.local/share/poly/appimages/x: Permission denied");
        it.finished(false, |_| Vec::new());
        assert_eq!(
            it.phase,
            Phase::Failed(
                "The app's files could not be removed: /home/poly/.local/share/poly/appimages/x: Permission denied."
                    .into()
            )
        );
    }

    fn hello_image() -> Image {
        Image {
            name: "Hello Pixels".into(),
            summary: "Draws a few pixels".into(),
            version: "1.2.3".into(),
            desktop: String::new(),
            icon: None,
            size: 5_000_000,
        }
    }

    #[test]
    fn an_appimage_is_run_or_installed_in_one_step_done_by_the_program() {
        let mut it = Installer::for_image("/home/poly/Downloads/Hello.AppImage", Ok(hello_image()));
        assert!(!it.uninstalling());
        assert_eq!((it.name(), it.summary()), ("Hello Pixels", "Draws a few pixels"));
        assert_eq!(
            it.facts(),
            [
                ("Version", "1.2.3".to_string()),
                ("Size", "4.7 MB".to_string()),
                ("Kind", "Linux app, AppImage".to_string())
            ]
        );
        assert_eq!(it.buttons(), [("Cancel", Action::Close), ("Run", Action::Run), ("Install", Action::Install)]);
        assert!(it.message().starts_with("Install copies the app to your user folder"));
        // Install: no command, the program places the files.
        assert_eq!(it.start(), Some(Vec::new()));
        assert_eq!(it.phase, Phase::Working(Step::Place));
        assert!(it.buttons().is_empty() && it.busy());
        assert_eq!(it.message(), "Installing\u{2026}");
        let id = "poly-appimage-hello-pixels.desktop".to_string();
        assert_eq!(it.finished(true, |_| alloc::vec![(id.clone(), "Hello Pixels".to_string())]), None);
        assert_eq!(it.phase, Phase::Done);
        assert_eq!(it.message(), "Hello Pixels is installed. It is in the launcher and in Applications.");
        assert_eq!(it.buttons(), [("Close", Action::Close), ("Open", Action::Open)]);
        assert_eq!(it.apps[0].0, id);
        // An AppImage that cannot be read is refused with the reason.
        let it = Installer::for_image("/tmp/old.AppImage", Err(crate::Error::OldAppImage));
        assert_eq!(it.phase, Phase::Refused(crate::Error::OldAppImage.describe().into()));
        assert_eq!(it.buttons(), [("Close", Action::Close)]);
        assert_eq!(it.name(), "old.AppImage");
        // The program's own failure.
        let mut it = Installer::for_image("/tmp/a.AppImage", Ok(hello_image()));
        it.start();
        it.output("E: /root/.local/share/poly: No space left on device");
        it.finished(false, |_| Vec::new());
        assert_eq!(
            it.phase,
            Phase::Failed("The app could not be installed: /root/.local/share/poly: No space left on device.".into())
        );
    }

    #[test]
    fn the_log_keeps_the_last_lines() {
        let mut it = ready();
        for i in 0..LOG_LINES + 5 {
            it.output(&alloc::format!("line {i}"));
        }
        it.output("   \n");
        assert_eq!(it.log.len(), LOG_LINES);
        assert_eq!(it.log[0], "line 5");
    }
}
