//! `cargo xtask start`: a menu in the terminal of everything xtask boots or
//! checks, with the options that matter for each, remembering the last
//! choice (ADR 0133).
//!
//! The menu opens on the last settings with Start focused, so Enter runs
//! the same thing again; arrow keys, Enter, Space and Esc change it. What
//! cannot run on this host is shown greyed with the reason; what needs
//! something made first says so, and Start makes it first (Chromium's
//! disk). The settings are kept per checkout in `target/poly-start.json`
//! (git-ignored with the rest of `target/`; a corrupt or older file gives
//! the defaults).
//!
//! The menu runs the same functions the subcommands do (`desktop::prepare`
//! and `desktop::boot`, `run`, `toolkits::command`, ...), in this process.
//! The builds before a boot run at the same time (the kernel, the native
//! programs, the Win32 DLLs, the desktop's files) and show as a table
//! ([`crate::progress`]); then the boot gets the terminal, as with the
//! subcommand.
//!
//! - `cargo xtask start --last`: run the saved choice without the menu.
//! - `cargo xtask start --print`: print the commands it would run.
//!
//! Without a real terminal (output piped, or Git Bash's mintty, whose
//! pipes are not a Windows console: `winpty cargo xtask start` gives the
//! menu there) a plain numbered prompt stands in for the menu.

use crate::resources::{self, HostResources, KERNEL_MAX_CPUS, MEMORY_PRESETS};
use crate::{Result, progress};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Where the settings are kept.
pub const SETTINGS_FILE: &str = "poly-start.json";
/// The settings file's format; another one is read as the defaults.
const FORMAT: u64 = 1;

/// The reason for what only a Linux host can run.
const LINUX_ONLY: &str = "needs a Linux host: run it in the poly-dev container or WSL";

// ---- What can be run --------------------------------------------------------

/// One thing the menu runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Desktop,
    ChromiumRun,
    ChromiumDisk,
    Toolkits,
    Shell,
    Busybox,
    DiskRoot,
    BootTests,
    Ci,
}

/// The menu's entries, in its order.
pub const ENTRIES: [Entry; 9] = [
    Entry::Desktop,
    Entry::ChromiumRun,
    Entry::ChromiumDisk,
    Entry::Toolkits,
    Entry::Shell,
    Entry::Busybox,
    Entry::DiskRoot,
    Entry::BootTests,
    Entry::Ci,
];

/// An option of an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opt {
    Release,
    Accel,
    Scale,
    Size,
    FreshState,
    FreshDisk,
    KernelArgs,
    Display,
    Groups,
    Parts,
    Memory,
    Cpus,
}

impl Entry {
    /// Its name in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Entry::Desktop => "desktop",
            Entry::ChromiumRun => "chromium-run",
            Entry::ChromiumDisk => "chromium-disk",
            Entry::Toolkits => "toolkits",
            Entry::Shell => "shell",
            Entry::Busybox => "busybox",
            Entry::DiskRoot => "disk-root",
            Entry::BootTests => "boot-tests",
            Entry::Ci => "ci",
        }
    }

    pub fn from_id(id: &str) -> Option<Entry> {
        ENTRIES.into_iter().find(|e| e.id() == id)
    }

    /// Its name in the menu.
    pub fn title(self) -> &'static str {
        match self {
            Entry::Desktop => "Desktop",
            Entry::ChromiumRun => "Desktop with Chromium",
            Entry::ChromiumDisk => "Build Chromium's disk",
            Entry::Toolkits => "Toolkits disk (GTK, Qt, VLC)",
            Entry::Shell => "Kernel shell",
            Entry::Busybox => "BusyBox shell",
            Entry::DiskRoot => "Root disk bash",
            Entry::BootTests => "Boot tests",
            Entry::Ci => "CI checks",
        }
    }

    /// One line on what it does.
    pub fn about(self) -> &'static str {
        match self {
            Entry::Desktop => {
                "Poly's desktop in QEMU's window: compositor, shell, launcher, test apps (cargo xtask desktop)"
            }
            Entry::ChromiumRun => {
                "the desktop from Chromium's disk; type \"chromium\" in the launcher (cargo xtask chromium run)"
            }
            Entry::ChromiumDisk => "download the pinned Debian packages and make target/poly-chromium-disk.img",
            Entry::Toolkits => "GTK 3/4, Qt and VLC apps on the desktop, driven by steps (cargo xtask toolkits)",
            Entry::Shell => "the kernel's own shell on the serial port, native programs in /bin (cargo xtask run)",
            Entry::Busybox => "BusyBox as init and its shell on the console (cargo xtask run --busybox)",
            Entry::DiskRoot => {
                "bash, coreutils, Python and gcc from the persistent root disk (cargo xtask run --disk-root)"
            }
            Entry::BootTests => "boot test groups in QEMU, checked (cargo xtask boot-tests)",
            Entry::Ci => "what CI checks: fmt, clippy, tests, coverage (cargo xtask ci)",
        }
    }

    /// Its options, in the menu's order.
    pub fn options(self) -> &'static [Opt] {
        use Opt::*;
        match self {
            Entry::Desktop => &[Release, Accel, Scale, Size, FreshState, KernelArgs, Display],
            Entry::ChromiumRun => &[Release, Accel, FreshDisk, Display],
            Entry::ChromiumDisk => &[],
            Entry::Toolkits => &[Release, Accel, FreshDisk],
            Entry::Shell | Entry::Busybox => &[Release, Accel, KernelArgs],
            Entry::DiskRoot => &[Release, Accel, FreshDisk, KernelArgs],
            Entry::BootTests => &[Accel, Groups],
            Entry::Ci => &[Parts],
        }
    }

    /// The resources it can be given (`--memory=`, `--cpus=`, ADR 0141):
    /// the interactive boots only; the boot tests keep their fixed sizes.
    pub fn resources(self) -> &'static [Opt] {
        match self {
            Entry::Desktop | Entry::ChromiumRun | Entry::Toolkits | Entry::Shell | Entry::Busybox | Entry::DiskRoot => {
                &[Opt::Memory, Opt::Cpus]
            }
            Entry::ChromiumDisk | Entry::BootTests | Entry::Ci => &[],
        }
    }

    /// The memory and CPUs it boots with unless asked (what "auto" is).
    pub fn default_resources(self) -> Option<(&'static str, u32)> {
        let cpus = crate::QEMU_CPUS;
        match self {
            Entry::Desktop => Some((crate::desktop::MEMORY, cpus)),
            Entry::ChromiumRun => Some((crate::chromium::MEMORY, crate::chromium::CPUS.unwrap_or(cpus))),
            Entry::Toolkits => Some((crate::toolkits::MEMORY, cpus)),
            Entry::Shell | Entry::Busybox => Some((crate::DEFAULT_MEMORY, cpus)),
            Entry::DiskRoot => Some((crate::rootdisk::MEMORY, cpus)),
            Entry::ChromiumDisk | Entry::BootTests | Entry::Ci => None,
        }
    }

    /// Whether it boots a kernel (built first, with the progress table).
    fn kernel(self, s: &Settings) -> Option<bool> {
        match self {
            Entry::Desktop | Entry::ChromiumDisk | Entry::Ci => None,
            Entry::BootTests => Some(false),
            _ => Some(s.release),
        }
    }
}

impl Opt {
    pub fn title(self) -> &'static str {
        match self {
            Opt::Release => "Build",
            Opt::Accel => "Accelerator",
            Opt::Scale => "Scale",
            Opt::Size => "Screen size",
            Opt::FreshState => "Fresh settings",
            Opt::FreshDisk => "Fresh disk",
            Opt::KernelArgs => "Kernel arguments",
            Opt::Display => "QEMU display",
            Opt::Groups => "Groups",
            Opt::Parts => "Parts",
            Opt::Memory => "Memory",
            Opt::Cpus => "CPUs",
        }
    }

    /// What changing it does, for the help line.
    fn hint(self) -> &'static str {
        match self {
            Opt::Release => "release or debug build of the kernel and the programs",
            Opt::Accel => "POLY_QEMU_ACCEL: the host's hypervisor if usable (auto), or QEMU's emulator (tcg)",
            Opt::Scale => "--scale=N: Poly's display scale; auto fits the host's screen",
            Opt::Size => "--size=WxH: the guest's screen; auto fits the host's screen (Enter types one)",
            Opt::FreshState => "--fresh-state: start the desktop's settings disk again",
            Opt::FreshDisk => "--fresh-disk: make the root disk again",
            Opt::KernelArgs => "extra kernel arguments, such as poly.display.scale=2 (Enter types them)",
            Opt::Display => "POLY_QEMU_DISPLAY: QEMU's -display (auto: SDL on Windows; Enter types one)",
            Opt::Groups => "the boot test groups to run (none chosen: all of them)",
            Opt::Parts => "the CI parts to run (none chosen: all of them)",
            Opt::Memory => {
                "--memory=SIZE: the guest's RAM (QEMU -m); the kernel uses all it is given (Enter types a size)"
            }
            Opt::Cpus => {
                "--cpus=N: the guest's CPUs (QEMU -smp); the kernel starts every one, up to its limit (Enter types)"
            }
        }
    }
}

// ---- The settings -------------------------------------------------------------

/// What the menu remembers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub entry: Entry,
    pub release: bool,
    /// `auto`, `kvm`, `whpx` or `tcg`.
    pub accel: String,
    /// `auto` or 1 to 4.
    pub scale: String,
    /// `auto` or `WxH`.
    pub size: String,
    pub fresh_state: bool,
    pub fresh_disk: bool,
    pub kernel_args: String,
    /// `auto` or a QEMU `-display` value.
    pub display: String,
    pub groups: Vec<String>,
    pub parts: Vec<String>,
    /// `auto` (the entry's own) or a size such as `4G` or `3072M`.
    pub memory: String,
    /// `auto` or 1 to [`KERNEL_MAX_CPUS`].
    pub cpus: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            entry: Entry::Desktop,
            release: true,
            accel: "auto".into(),
            scale: "auto".into(),
            size: "auto".into(),
            fresh_state: false,
            fresh_disk: false,
            kernel_args: String::new(),
            display: "auto".into(),
            groups: vec!["desktop".into(), "desktop-shell".into()],
            parts: vec!["lint".into(), "host-tests".into()],
            memory: "auto".into(),
            cpus: "auto".into(),
        }
    }
}

const ACCELS: [&str; 4] = ["auto", "kvm", "whpx", "tcg"];
const SCALES: [&str; 5] = ["auto", "1", "2", "3", "4"];
const SIZES: [&str; 6] = ["auto", "1280x800", "1600x1000", "1920x1200", "2560x1600", "3840x2400"];
const DISPLAYS: [&str; 5] = ["auto", "sdl", "gtk", "vnc=:0", "none"];

/// `WxH` with each side 320 to 8192 (what `desktop --size=` takes).
fn valid_size(v: &str) -> bool {
    let Some((w, h)) = v.split_once(['x', 'X']) else { return false };
    let side = |s: &str| s.parse::<u32>().is_ok_and(|n| (320..=8192).contains(&n));
    side(w) && side(h)
}

/// 1 to the kernel's limit (what `--cpus=` can usefully be).
fn valid_cpus(v: &str) -> bool {
    v.parse::<u32>().is_ok_and(|n| (1..=KERNEL_MAX_CPUS).contains(&n))
}

/// The boot test groups, with their CI shard.
fn boot_groups() -> Vec<(&'static str, &'static str)> {
    crate::BOOT_GROUPS
        .iter()
        .map(|(name, _)| {
            let shard = crate::CI_BOOT_SHARDS.iter().find(|(_, g)| g.contains(name)).map_or("-", |(s, _)| s);
            (*name, shard)
        })
        .collect()
}

fn ci_parts() -> Vec<&'static str> {
    crate::CI_PARTS.iter().map(|(n, _)| *n).collect()
}

impl Settings {
    pub fn to_json(&self) -> Value {
        json!({
            "format": FORMAT,
            "entry": self.entry.id(),
            "release": self.release,
            "accel": self.accel,
            "scale": self.scale,
            "size": self.size,
            "fresh_state": self.fresh_state,
            "fresh_disk": self.fresh_disk,
            "kernel_args": self.kernel_args,
            "display": self.display,
            "groups": self.groups,
            "parts": self.parts,
            "memory": self.memory,
            "cpus": self.cpus,
        })
    }

    /// The settings in `text`: each value that is missing or wrong is the
    /// default's; another format, or no JSON object, is the defaults.
    pub fn from_json(text: &str) -> Settings {
        let d = Settings::default();
        let Ok(v) = serde_json::from_str::<Value>(text) else { return d };
        if v.get("format").and_then(Value::as_u64) != Some(FORMAT) {
            return d;
        }
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let flag = |k: &str, def: bool| v.get(k).and_then(Value::as_bool).unwrap_or(def);
        let one_of =
            |k: &str, list: &[&str], def: &str| text(k).filter(|s| list.contains(&s.as_str())).unwrap_or(def.into());
        let names = |k: &str, known: &[&str], def: &[String]| match v.get(k).and_then(Value::as_array) {
            Some(a) => a.iter().filter_map(Value::as_str).filter(|n| known.contains(n)).map(str::to_string).collect(),
            None => def.to_vec(),
        };
        let groups: Vec<&str> = boot_groups().into_iter().map(|(n, _)| n).collect();
        Settings {
            entry: text("entry").and_then(|e| Entry::from_id(&e)).unwrap_or(d.entry),
            release: flag("release", d.release),
            accel: one_of("accel", &ACCELS, "auto"),
            scale: one_of("scale", &SCALES, "auto"),
            size: text("size").filter(|s| s == "auto" || valid_size(s)).unwrap_or(d.size.clone()),
            fresh_state: flag("fresh_state", false),
            fresh_disk: flag("fresh_disk", false),
            kernel_args: text("kernel_args").filter(|a| !a.contains(['\n', '\r'])).unwrap_or_default(),
            display: text("display")
                .filter(|s| !s.trim().is_empty() && !s.contains(char::is_whitespace))
                .unwrap_or(d.display.clone()),
            groups: names("groups", &groups, &d.groups),
            parts: names("parts", &ci_parts(), &d.parts),
            memory: text("memory")
                .and_then(
                    |m| if m == "auto" { Some(m) } else { resources::parse_memory(&m).map(resources::format_memory) },
                )
                .unwrap_or(d.memory.clone()),
            cpus: text("cpus").filter(|c| c == "auto" || valid_cpus(c)).unwrap_or(d.cpus.clone()),
        }
    }

    fn path() -> PathBuf {
        crate::root().join("target").join(SETTINGS_FILE)
    }

    /// The saved settings, or the defaults.
    pub fn load() -> Settings {
        std::fs::read_to_string(Settings::path()).map(|t| Settings::from_json(&t)).unwrap_or_default()
    }

    /// Save them (through a temporary file, so a crash leaves the old ones).
    pub fn save(&self) -> Result {
        let path = Settings::path();
        let dir = path.parent().expect("target/");
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let tmp = dir.join(format!(".{SETTINGS_FILE}.{}.tmp", std::process::id()));
        let text = serde_json::to_string_pretty(&self.to_json()).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text + "\n").map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Options' values as the menu shows them.
    pub fn value(&self, o: Opt) -> String {
        let list = |l: &[String]| if l.is_empty() { "all".to_string() } else { l.join(", ") };
        match o {
            Opt::Release => if self.release { "release" } else { "debug" }.into(),
            Opt::Accel => self.accel.clone(),
            Opt::Scale => self.scale.clone(),
            Opt::Size => self.size.clone(),
            Opt::FreshState => if self.fresh_state { "yes" } else { "no" }.into(),
            Opt::FreshDisk => if self.fresh_disk { "yes" } else { "no" }.into(),
            Opt::KernelArgs => {
                if self.kernel_args.trim().is_empty() {
                    "(none)".into()
                } else {
                    self.kernel_args.clone()
                }
            }
            Opt::Display => self.display.clone(),
            Opt::Groups => list(&self.groups),
            Opt::Parts => list(&self.parts),
            Opt::Memory => self.memory.clone(),
            Opt::Cpus => self.cpus.clone(),
        }
    }
}

// ---- The host -------------------------------------------------------------------

/// What this host can do, for which entries are offered.
#[derive(Clone, Debug)]
pub struct Host {
    pub linux: bool,
    pub windows: bool,
    /// `POLY_BUSYBOX` names a BusyBox binary.
    pub busybox_env: bool,
    /// Chromium's disk image is there.
    pub chromium_disk: bool,
    /// Its memory and cores, for the resources it can give a guest.
    pub res: HostResources,
}

impl Host {
    pub fn probe() -> Host {
        Host {
            linux: cfg!(target_os = "linux"),
            windows: cfg!(windows),
            busybox_env: std::env::var("POLY_BUSYBOX").is_ok_and(|v| !v.is_empty() && v != "build"),
            chromium_disk: crate::root().join("target").join(crate::chromium::FILE_NAME).is_file(),
            res: HostResources::probe(),
        }
    }

    /// The accelerators worth offering here.
    fn accels(&self) -> Vec<&'static str> {
        ACCELS
            .into_iter()
            .filter(|a| match *a {
                "kvm" => self.linux,
                "whpx" => self.windows,
                _ => true,
            })
            .collect()
    }
}

/// Whether an entry can run here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Avail {
    Ready,
    /// It can, after `first` (which Start runs first), as `why` says.
    First {
        first: Entry,
        why: String,
    },
    /// It cannot, for this reason.
    No(String),
}

pub fn availability(e: Entry, host: &Host) -> Avail {
    match e {
        Entry::ChromiumRun if !host.chromium_disk => Avail::First {
            first: Entry::ChromiumDisk,
            why: "needs Chromium's disk, not built yet: Start builds it first (a download and some minutes)".into(),
        },
        Entry::Toolkits if !host.linux => Avail::No(format!("{LINUX_ONLY} (its disk is the host's GTK, Qt and VLC)")),
        Entry::DiskRoot if !host.linux => {
            Avail::No(format!("{LINUX_ONLY} (the root disk is the host's bash, coreutils, Python, gcc)"))
        }
        Entry::Busybox if !host.linux && !host.busybox_env => Avail::No(
            "needs BusyBox: a Linux host has or builds it; or set POLY_BUSYBOX to a static busybox binary".into(),
        ),
        _ => Avail::Ready,
    }
}

/// A note on an entry that can run, for this host.
fn note(e: Entry, host: &Host) -> Option<&'static str> {
    match e {
        Entry::BootTests if !host.linux => Some("on Windows the groups that drive QEMU through QMP skip themselves"),
        Entry::Ci if !host.linux => {
            Some("CI runs on Linux; here some parts fail or skip: the poly-dev container runs all")
        }
        Entry::ChromiumDisk if host.chromium_disk => Some("built already: this makes it again"),
        _ => None,
    }
}

/// What the desktop's launcher will offer (beyond Settings and
/// Notifications).
fn apps_line(e: Entry) -> Option<String> {
    match e {
        Entry::Desktop => {
            let apps: Vec<String> = crate::desktop::launcher_apps().into_iter().map(|(n, _)| n).collect();
            Some(format!("The desktop image's apps: {}", apps.join(", ")))
        }
        Entry::ChromiumRun => Some("The launcher: Chromium, the demo window, and the disk's desktop files".into()),
        _ => None,
    }
}

// ---- What Start runs --------------------------------------------------------------

/// One command of a plan: `cargo xtask <args>` with `env` set (`None`:
/// removed, so xtask chooses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub entry: Entry,
    pub args: Vec<String>,
    pub env: Vec<(&'static str, Option<String>)>,
}

impl Job {
    /// The command as it would be typed.
    pub fn line(&self) -> String {
        let mut line = format!("cargo xtask {}", self.args.join(" "));
        let set: Vec<String> = self.env.iter().filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={v}"))).collect();
        if !set.is_empty() {
            line.push_str(&format!("   (environment: {})", set.join(" ")));
        }
        line
    }
}

/// The arguments of `cargo xtask` for `e` with `s`.
pub fn args(e: Entry, s: &Settings) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let flag = |a: &mut Vec<String>, on: bool, f: &str| {
        if on {
            a.push(f.to_string());
        }
    };
    let kernel_args = |a: &mut Vec<String>| a.extend(s.kernel_args.split_whitespace().map(str::to_string));
    // --memory= and --cpus= for what takes them (ADR 0141), before the kernel arguments.
    let resources = |a: &mut Vec<String>| {
        if !e.resources().is_empty() {
            if s.memory != "auto" {
                a.push(format!("--memory={}", s.memory));
            }
            if s.cpus != "auto" {
                a.push(format!("--cpus={}", s.cpus));
            }
        }
    };
    match e {
        Entry::Desktop => {
            a.push("desktop".into());
            flag(&mut a, s.release, "--release");
            flag(&mut a, s.fresh_state, "--fresh-state");
            if s.scale != "auto" {
                a.push(format!("--scale={}", s.scale));
            }
            if s.size != "auto" {
                a.push(format!("--size={}", s.size));
            }
            resources(&mut a);
            kernel_args(&mut a);
        }
        Entry::ChromiumRun => {
            a.extend(["chromium".into(), "run".into()]);
            flag(&mut a, s.release, "--release");
            flag(&mut a, s.fresh_disk, "--fresh-disk");
            resources(&mut a);
        }
        Entry::ChromiumDisk => a.extend(["chromium".into(), "disk".into()]),
        Entry::Toolkits => {
            a.push("toolkits".into());
            flag(&mut a, s.release, "--release");
            flag(&mut a, s.fresh_disk, "--fresh-disk");
            resources(&mut a);
        }
        Entry::Shell | Entry::Busybox | Entry::DiskRoot => {
            a.push("run".into());
            flag(&mut a, s.release, "--release");
            flag(&mut a, e == Entry::Busybox, "--busybox");
            if e == Entry::DiskRoot {
                a.push("--disk-root".into());
                flag(&mut a, s.fresh_disk, "--fresh-disk");
            }
            resources(&mut a);
            kernel_args(&mut a);
        }
        Entry::BootTests => {
            a.push("boot-tests".into());
            a.extend(s.groups.iter().cloned());
        }
        Entry::Ci => {
            a.push("ci".into());
            a.extend(s.parts.iter().cloned());
        }
    }
    a
}

/// The environment of `e`'s run.
fn env(e: Entry, s: &Settings) -> Vec<(&'static str, Option<String>)> {
    let auto = |v: &str| (v != "auto").then(|| v.to_string());
    let mut env = Vec::new();
    if e.options().contains(&Opt::Accel) {
        env.push((crate::accel::ENV, auto(&s.accel)));
    }
    if e.options().contains(&Opt::Display) {
        env.push(("POLY_QEMU_DISPLAY", auto(&s.display)));
    }
    env
}

/// What Start runs for `s`: what is needed first, then the entry.
pub fn plan(s: &Settings, host: &Host) -> std::result::Result<Vec<Job>, String> {
    let job = |e: Entry| Job { entry: e, args: args(e, s), env: env(e, s) };
    match availability(s.entry, host) {
        Avail::Ready => Ok(vec![job(s.entry)]),
        Avail::First { first, .. } => Ok(vec![job(first), job(s.entry)]),
        Avail::No(why) => Err(format!("{}: {why}", s.entry.title())),
    }
}

/// Run `job` (only its builds with `build_only`), in this process.
fn execute(job: &Job, s: &Settings, build_only: bool) -> Result {
    for (k, v) in &job.env {
        // SAFETY: the menu is gone and nothing else runs yet: this thread
        // is the only one, so nothing reads the environment meanwhile.
        unsafe {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
    eprintln!("==> {}", job.line());
    let rest = &job.args[1..];
    match job.entry {
        Entry::Desktop => {
            let ready = progress::show("Building the desktop", || crate::desktop::prepare(rest))?;
            if build_only { Ok(()) } else { crate::desktop::boot(ready) }
        }
        Entry::ChromiumDisk => crate::dispatch(&job.args),
        e => {
            if let Some(release) = e.kernel(s) {
                progress::show("Building the kernel", || crate::build_kernel(release))?;
            }
            if build_only { Ok(()) } else { crate::dispatch(&job.args) }
        }
    }
}

// ---- The menu --------------------------------------------------------------------

/// A row the focus can be on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Entry(Entry),
    Opt(Opt),
    Start,
    BuildOnly,
    Quit,
}

pub use crate::term::Key;

/// What the menu asks for after a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Run { build_only: bool },
    Quit,
}

/// A list to pick several of (boot test groups, CI parts).
#[derive(Clone, Debug)]
pub struct Picker {
    opt: Opt,
    /// (name, note).
    items: Vec<(String, String)>,
    chosen: Vec<String>,
    before: Vec<String>,
    focus: usize,
}

/// The menu's state.
#[derive(Clone, Debug)]
pub struct Menu {
    pub s: Settings,
    pub host: Host,
    pub focus: usize,
    /// The text being typed for an option.
    pub edit: Option<(Opt, String)>,
    pub picker: Option<Picker>,
    pub message: String,
}

impl Menu {
    pub fn new(s: Settings, host: Host) -> Menu {
        let mut m = Menu { s, host, focus: 0, edit: None, picker: None, message: String::new() };
        m.focus = m.rows().iter().position(|r| *r == Row::Start).unwrap_or(0);
        m
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = ENTRIES.iter().map(|e| Row::Entry(*e)).collect();
        rows.extend(self.s.entry.options().iter().map(|o| Row::Opt(*o)));
        rows.extend(self.s.entry.resources().iter().map(|o| Row::Opt(*o)));
        rows.extend([Row::Start, Row::BuildOnly, Row::Quit]);
        rows
    }

    fn focused(&self) -> Row {
        let rows = self.rows();
        rows[self.focus.min(rows.len() - 1)]
    }

    fn focus_on(&mut self, row: Row) {
        if let Some(i) = self.rows().iter().position(|r| *r == row) {
            self.focus = i;
        }
    }

    pub fn key(&mut self, k: Key) -> Action {
        self.message.clear();
        if self.picker.is_some() {
            self.picker_key(k);
            return Action::None;
        }
        if self.edit.is_some() {
            self.edit_key(k);
            return Action::None;
        }
        let n = self.rows().len();
        match k {
            Key::Up => self.focus = (self.focus + n - 1) % n,
            Key::Down | Key::Tab => self.focus = (self.focus + 1) % n,
            Key::Esc | Key::Char('q') => return Action::Quit,
            Key::Left | Key::Right => match self.focused() {
                Row::Opt(o) => self.change(o, if k == Key::Left { -1 } else { 1 }),
                Row::Start | Row::BuildOnly | Row::Quit => {
                    let buttons = [Row::Start, Row::BuildOnly, Row::Quit];
                    let i = buttons.iter().position(|b| *b == self.focused()).unwrap_or(0);
                    let next = if k == Key::Left { (i + 2) % 3 } else { (i + 1) % 3 };
                    self.focus_on(buttons[next]);
                }
                Row::Entry(_) => {}
            },
            Key::Enter | Key::Space => match self.focused() {
                Row::Entry(e) => self.choose(e),
                Row::Opt(o) => self.activate(o, k == Key::Enter),
                Row::Start => return self.run(false),
                Row::BuildOnly => return self.run(true),
                Row::Quit => return Action::Quit,
            },
            Key::Char('s') => return self.run(false),
            Key::Char('b') => return self.run(true),
            _ => {}
        }
        Action::None
    }

    fn run(&mut self, build_only: bool) -> Action {
        match availability(self.s.entry, &self.host) {
            Avail::No(why) => {
                self.message = format!("{} cannot run here: {why}", self.s.entry.title());
                Action::None
            }
            _ => Action::Run { build_only },
        }
    }

    fn choose(&mut self, e: Entry) {
        match availability(e, &self.host) {
            Avail::No(why) => self.message = format!("{}: {why}", e.title()),
            avail => {
                self.s.entry = e;
                if let Avail::First { why, .. } = avail {
                    self.message = why;
                }
                // The focus stays on the entry (its rows above the options
                // do not move).
                self.focus_on(Row::Entry(e));
            }
        }
    }

    /// Enter or Space on an option.
    fn activate(&mut self, o: Opt, enter: bool) {
        match o {
            Opt::Release | Opt::FreshState | Opt::FreshDisk | Opt::Accel | Opt::Scale => self.change(o, 1),
            Opt::Size | Opt::Display | Opt::KernelArgs if enter => {
                let now = match o {
                    Opt::Size if self.s.size == "auto" => String::new(),
                    Opt::Size => self.s.size.clone(),
                    Opt::Display if self.s.display == "auto" => String::new(),
                    Opt::Display => self.s.display.clone(),
                    _ => self.s.kernel_args.clone(),
                };
                self.edit = Some((o, now));
            }
            Opt::Memory | Opt::Cpus if enter => {
                let now = if self.s.value(o) == "auto" { String::new() } else { self.s.value(o) };
                self.edit = Some((o, now));
            }
            Opt::Size | Opt::Display | Opt::KernelArgs | Opt::Memory | Opt::Cpus => self.change(o, 1),
            Opt::Groups | Opt::Parts => {
                let (items, chosen): (Vec<(String, String)>, Vec<String>) = if o == Opt::Groups {
                    let items =
                        boot_groups().into_iter().map(|(n, s)| (n.to_string(), format!("CI shard {s}"))).collect();
                    (items, self.s.groups.clone())
                } else {
                    (ci_parts().into_iter().map(|n| (n.to_string(), String::new())).collect(), self.s.parts.clone())
                };
                self.picker = Some(Picker { opt: o, items, before: chosen.clone(), chosen, focus: 0 });
            }
        }
    }

    /// Step an option's value by `by` (±1), cycling.
    fn change(&mut self, o: Opt, by: i32) {
        let cycle = |list: &[&str], now: &str| {
            let i = list.iter().position(|v| *v == now).map_or(0, |i| i as i32 + by);
            list[i.rem_euclid(list.len() as i32) as usize].to_string()
        };
        let owned = |list: Vec<String>, now: &str| {
            let i = list.iter().position(|v| v == now).map_or(0, |i| i as i32 + by);
            list[i.rem_euclid(list.len() as i32) as usize].clone()
        };
        let (memory_choices, cpu_choices) = (self.memory_choices(), self.cpu_choices());
        let s = &mut self.s;
        match o {
            Opt::Memory => s.memory = owned(memory_choices, &s.memory),
            Opt::Cpus => s.cpus = owned(cpu_choices, &s.cpus),
            Opt::Release => s.release = !s.release,
            Opt::FreshState => s.fresh_state = !s.fresh_state,
            Opt::FreshDisk => s.fresh_disk = !s.fresh_disk,
            Opt::Accel => s.accel = cycle(&self.host.accels(), &s.accel),
            Opt::Scale => s.scale = cycle(&SCALES, &s.scale),
            Opt::Size => s.size = cycle(&SIZES, &s.size),
            Opt::Display => s.display = cycle(&DISPLAYS, &s.display),
            Opt::KernelArgs => self.edit = Some((o, s.kernel_args.clone())),
            Opt::Groups | Opt::Parts => self.activate(o, true),
        }
    }

    /// The memory sizes Left and Right step through: auto and the presets
    /// this host can give.
    fn memory_choices(&self) -> Vec<String> {
        let ok = |p: &&str| resources::parse_memory(p).is_some_and(|m| self.host.res.memory_refusal(m).is_none());
        std::iter::once("auto").chain(MEMORY_PRESETS.iter().copied().filter(ok)).map(str::to_string).collect()
    }

    /// The CPU counts they step through: auto and 1 to the host's cores
    /// (the kernel's limit at most).
    fn cpu_choices(&self) -> Vec<String> {
        std::iter::once("auto".to_string()).chain((1..=self.host.res.cpu_limit()).map(|n| n.to_string())).collect()
    }

    fn edit_key(&mut self, k: Key) {
        let Some((o, text)) = self.edit.as_mut() else { return };
        let o = *o;
        match k {
            Key::Char(c) if !c.is_control() => text.push(c),
            Key::Space => text.push(' '),
            Key::Backspace => {
                text.pop();
            }
            Key::Esc => self.edit = None,
            Key::Enter => {
                let v = text.trim().to_string();
                match o {
                    Opt::Size if v.is_empty() || v == "auto" => self.s.size = "auto".into(),
                    Opt::Size if valid_size(&v) => self.s.size = v.to_lowercase(),
                    Opt::Size => {
                        self.message = format!("{v:?}: a size is WxH, each side 320 to 8192, such as 2560x1600");
                        return;
                    }
                    Opt::Display if v.is_empty() || v == "auto" => self.s.display = "auto".into(),
                    Opt::Display if v.contains(char::is_whitespace) => {
                        self.message = "a QEMU display has no spaces (such as sdl, gtk,zoom-to-fit=on, vnc=:1)".into();
                        return;
                    }
                    Opt::Display => self.s.display = v,
                    Opt::Memory if v.is_empty() || v == "auto" => self.s.memory = "auto".into(),
                    Opt::Memory => match resources::parse_memory(&v) {
                        None => {
                            self.message = format!(
                                "{v:?}: a size is a number and M or G, from {}M, such as 4G or 3072M",
                                resources::MIN_MEMORY_MIB
                            );
                            return;
                        }
                        Some(mib) => {
                            if let Some(why) = self.host.res.memory_refusal(mib) {
                                self.message = why;
                                return;
                            }
                            self.s.memory = resources::format_memory(mib);
                        }
                    },
                    Opt::Cpus if v.is_empty() || v == "auto" => self.s.cpus = "auto".into(),
                    Opt::Cpus => match v.parse::<u32>().ok().filter(|n| *n >= 1) {
                        None => {
                            self.message = format!("{v:?}: a number of CPUs, from 1 to {KERNEL_MAX_CPUS}");
                            return;
                        }
                        Some(n) => {
                            if let Some(why) = self.host.res.cpu_refusal(n) {
                                self.message = why;
                                return;
                            }
                            self.s.cpus = n.to_string();
                        }
                    },
                    _ => self.s.kernel_args = v,
                }
                self.edit = None;
            }
            _ => {}
        }
    }

    fn picker_key(&mut self, k: Key) {
        let Some(p) = self.picker.as_mut() else { return };
        let n = p.items.len();
        match k {
            Key::Up => p.focus = (p.focus + n - 1) % n,
            Key::Down | Key::Tab => p.focus = (p.focus + 1) % n,
            Key::Space | Key::Right | Key::Left => {
                let name = &p.items[p.focus].0;
                if let Some(i) = p.chosen.iter().position(|c| c == name) {
                    p.chosen.remove(i);
                } else {
                    p.chosen.push(name.clone());
                }
            }
            Key::Char('a') => {
                if p.chosen.is_empty() {
                    p.chosen = p.items.iter().map(|(n, _)| n.clone()).collect();
                } else {
                    p.chosen.clear();
                }
            }
            Key::Esc => p.chosen = p.before.clone(),
            _ => {}
        }
        if matches!(k, Key::Enter | Key::Esc) {
            // In the order of the list.
            let order: Vec<String> = p.items.iter().map(|(n, _)| n.clone()).filter(|n| p.chosen.contains(n)).collect();
            match p.opt {
                Opt::Groups => self.s.groups = order,
                _ => self.s.parts = order,
            }
            self.picker = None;
        }
    }
}

// ---- Drawing ----------------------------------------------------------------------

/// How a piece of a line looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Plain,
    Title,
    Dim,
    Focus,
    Chosen,
    Warn,
    Bad,
    Command,
}

/// A line of the screen: its pieces.
pub type Line = Vec<(Look, String)>;

impl Menu {
    /// The screen's lines.
    pub fn lines(&self) -> Vec<Line> {
        if let Some(p) = &self.picker {
            return self.picker_lines(p);
        }
        let mut out: Vec<Line> = Vec::new();
        let focus = self.focused();
        let host = if self.host.windows {
            "Windows"
        } else if self.host.linux {
            "Linux"
        } else {
            "this host"
        };
        out.push(vec![
            (Look::Title, " Poly  cargo xtask start".into()),
            (Look::Dim, format!("   on {host}; settings in target/{SETTINGS_FILE}")),
        ]);
        out.push(vec![]);
        out.push(vec![(Look::Title, " What to run".into())]);
        for e in ENTRIES {
            let avail = availability(e, &self.host);
            let mark = if e == self.s.entry { "(*)" } else { "( )" };
            let name = format!("  {mark} {:<30}", e.title());
            let look = if focus == Row::Entry(e) {
                Look::Focus
            } else if matches!(avail, Avail::No(_)) {
                Look::Dim
            } else if e == self.s.entry {
                Look::Chosen
            } else {
                Look::Plain
            };
            let (tail, tail_look) = match &avail {
                Avail::No(why) => (why.clone(), Look::Dim),
                Avail::First { why, .. } => (why.clone(), Look::Warn),
                Avail::Ready => match note(e, &self.host) {
                    Some(n) => (format!("{}; {n}", e.about()), Look::Dim),
                    None => (e.about().to_string(), Look::Dim),
                },
            };
            out.push(vec![(look, name), (tail_look, format!(" {tail}"))]);
        }
        out.push(vec![]);
        let opts = self.s.entry.options();
        if opts.is_empty() {
            out.push(vec![(Look::Dim, format!(" {} has no options", self.s.entry.title()))]);
        } else {
            out.push(vec![(Look::Title, format!(" Options for {}", self.s.entry.title()))]);
        }
        for o in opts {
            let value = match &self.edit {
                Some((e, text)) if e == o => format!("{text}_"),
                _ => self.s.value(*o),
            };
            let look = if focus == Row::Opt(*o) { Look::Focus } else { Look::Plain };
            let arrows = if matches!(o, Opt::Groups | Opt::Parts | Opt::KernelArgs) { "  " } else { "< " };
            out.push(vec![
                (look, format!("      {:<18}{arrows}{value}", o.title())),
                (Look::Dim, if focus == Row::Opt(*o) { format!("   {}", o.hint()) } else { String::new() }),
            ]);
        }
        if !self.s.entry.resources().is_empty() {
            out.push(vec![]);
            out.push(vec![(Look::Title, format!(" Resources for {}", self.s.entry.title()))]);
            for o in self.s.entry.resources() {
                self.resource_lines(*o, focus == Row::Opt(*o), &mut out);
            }
        }
        out.push(vec![]);
        if let Some(apps) = apps_line(self.s.entry) {
            out.push(vec![(Look::Dim, format!(" {apps}"))]);
        }
        match plan(&self.s, &self.host) {
            Ok(jobs) => {
                for (i, j) in jobs.iter().enumerate() {
                    let head = if i == 0 { " Runs:  " } else { "  then: " };
                    out.push(vec![(Look::Dim, head.into()), (Look::Command, j.line())]);
                }
            }
            Err(why) => out.push(vec![(Look::Bad, format!(" Cannot run here: {why}"))]),
        }
        out.push(vec![]);
        let button = |row: Row, label: &str| {
            let look = if focus == row { Look::Focus } else { Look::Plain };
            (look, format!("[ {label} ]"))
        };
        out.push(vec![
            (Look::Plain, "     ".into()),
            button(Row::Start, "Start"),
            (Look::Plain, "   ".into()),
            button(Row::BuildOnly, "Build only"),
            (Look::Plain, "   ".into()),
            button(Row::Quit, "Quit"),
        ]);
        out.push(vec![]);
        if !self.message.is_empty() {
            out.push(vec![(Look::Warn, format!(" {}", self.message))]);
        }
        let help = if self.edit.is_some() {
            " type, Enter: done, Esc: cancel"
        } else {
            " Up/Down: move   Enter: choose or change   Left/Right: change   Esc or q: quit   s: Start"
        };
        out.push(vec![(Look::Dim, help.into())]);
        out
    }

    /// The rows of a resource: its value and what it means for this entry
    /// and this host, then the choices (those this host cannot give greyed,
    /// with the reason).
    fn resource_lines(&self, o: Opt, focused: bool, out: &mut Vec<Line>) {
        let (default_memory, default_cpus) = self.s.entry.default_resources().unwrap_or(("?", 0));
        let host = &self.host.res;
        let value = match &self.edit {
            Some((e, text)) if *e == o => format!("{text}_"),
            _ => self.s.value(o),
        };
        let asked_memory = resources::parse_memory(&self.s.memory);
        let asked_cpus = self.s.cpus.parse::<u32>().ok();
        // What the value comes to, and a warning where it will not be taken as it is.
        let (note, warn) = match o {
            Opt::Memory => match asked_memory {
                None => (format!("= {default_memory}, the default"), false),
                Some(m) if host.memory_refusal(m).is_some() => {
                    let limit = host.memory_limit_mib().unwrap_or(m) / 128 * 128;
                    (format!("more than this host can give: Start uses {}", resources::format_memory(limit)), true)
                }
                Some(m) if resources::parse_memory(default_memory).is_some_and(|d| m < d) => {
                    (format!("below the default {default_memory}: it may not boot"), true)
                }
                Some(_) => (format!("the default is {default_memory}"), false),
            },
            _ => match asked_cpus {
                None => (format!("= {default_cpus}, the default"), false),
                Some(n) if host.cpu_refusal(n).is_some() => {
                    (format!("more than this host has: Start uses {}", host.cpu_limit()), true)
                }
                Some(_) => (format!("the default is {default_cpus}"), false),
            },
        };
        let look = if focused { Look::Focus } else { Look::Plain };
        let tail = if focused && !warn { o.hint().to_string() } else { note };
        out.push(vec![
            (look, format!("      {:<18}< {value}", o.title())),
            (if warn { Look::Warn } else { Look::Dim }, format!("   {tail}")),
        ]);
        // The choices.
        let mut strip: Line = vec![(Look::Plain, "                        ".into())];
        let mut greyed: Vec<String> = Vec::new();
        let mut item = |strip: &mut Line, text: &str, chosen: bool, refusal: Option<String>| {
            let look = match (&refusal, chosen) {
                (Some(_), _) => Look::Dim,
                (None, true) => Look::Chosen,
                (None, false) => Look::Plain,
            };
            if refusal.is_some() {
                greyed.push(text.to_string());
            }
            strip.push((look, format!("{text:<6}")));
        };
        let explain = match o {
            Opt::Memory => {
                let mut sizes: Vec<String> =
                    std::iter::once("auto".to_string()).chain(MEMORY_PRESETS.iter().map(|p| p.to_string())).collect();
                if !sizes.contains(&self.s.memory) {
                    sizes.push(self.s.memory.clone());
                }
                for size in &sizes {
                    let refusal = resources::parse_memory(size).and_then(|m| host.memory_refusal(m));
                    item(&mut strip, size, *size == self.s.memory, refusal);
                }
                match (host.memory_mib, host.memory_limit_mib()) {
                    (Some(total), Some(limit)) if !greyed.is_empty() => format!(
                        "{} greyed: this host has {}; {} at most, the rest stays for it",
                        greyed.join(", "),
                        resources::gib(total),
                        resources::format_memory(limit / 128 * 128)
                    ),
                    (Some(total), _) => format!("this host has {}", resources::gib(total)),
                    _ => "this host's memory is unknown: nothing greyed".to_string(),
                }
            }
            _ => {
                item(&mut strip, "auto", self.s.cpus == "auto", None);
                for n in 1..=host.cpu_limit() {
                    item(&mut strip, &n.to_string(), self.s.cpus == n.to_string(), None);
                }
                let kernel = if host.cpus > KERNEL_MAX_CPUS {
                    format!("the kernel starts at most {KERNEL_MAX_CPUS} of them")
                } else {
                    format!("the kernel starts every one, up to {KERNEL_MAX_CPUS}")
                };
                format!("this host has {} logical cores; {kernel}", host.cpus)
            }
        };
        out.push(strip);
        out.push(vec![(Look::Dim, format!("                        {explain}"))]);
    }

    fn picker_lines(&self, p: &Picker) -> Vec<Line> {
        let mut out: Vec<Line> =
            vec![vec![(Look::Title, format!(" {} for {}", p.opt.title(), self.s.entry.title()))], vec![]];
        for (i, (name, note)) in p.items.iter().enumerate() {
            let on = p.chosen.contains(name);
            let look = if i == p.focus {
                Look::Focus
            } else if on {
                Look::Chosen
            } else {
                Look::Plain
            };
            out.push(vec![
                (look, format!("   [{}] {name:<20}", if on { 'x' } else { ' ' })),
                (Look::Dim, format!(" {note}")),
            ]);
        }
        out.push(vec![]);
        let none = if p.chosen.is_empty() { " (none chosen: all of them)" } else { "" };
        out.push(vec![(Look::Dim, format!(" {} chosen{none}", p.chosen.len()))]);
        out.push(vec![(Look::Dim, " Space: choose   a: all or none   Enter: done   Esc: cancel".into())]);
        out
    }
}

/// Draw `lines` on the terminal (the part that fits; the focused line is
/// kept in view).
fn draw(out: &mut impl std::io::Write, lines: &[Line]) -> std::io::Result<()> {
    use crate::term::{CLEAR, goto, sgr};
    let (width, height) = crate::term::size();
    let focus_line = lines.iter().position(|l| l.iter().any(|(k, _)| *k == Look::Focus)).unwrap_or(0);
    let skip = (focus_line + 3).saturating_sub(height);
    let mut screen = String::from(CLEAR);
    for (row, line) in lines.iter().skip(skip).take(height).enumerate() {
        screen.push_str(&goto(row, 0));
        let mut room = width.saturating_sub(1);
        for (look, text) in line {
            if room == 0 {
                break;
            }
            let text: String = text.chars().take(room).collect();
            room -= text.chars().count();
            let codes = match look {
                Look::Plain => "0",
                Look::Title => "1;36",
                Look::Dim => "90",
                Look::Focus => "1;7",
                Look::Chosen => "32",
                Look::Warn => "33",
                Look::Bad => "31",
                Look::Command => "1;37",
            };
            screen.push_str(&sgr(codes));
            screen.push_str(&text);
            screen.push_str(&sgr("0"));
        }
    }
    out.write_all(screen.as_bytes())?;
    out.flush()
}

/// The menu on the terminal until Start or Quit; the settings and what to
/// do with them.
fn menu(s: Settings, host: Host) -> Result<(Settings, Action)> {
    use crate::term::{ALT_OFF, ALT_ON, HIDE, Raw, SHOW};
    use std::io::Write;
    let mut m = Menu::new(s, host);
    let mut out = std::io::stdout();
    let raw = Raw::enable().map_err(|e| format!("the terminal's raw mode: {e}"))?;
    let _ = write!(out, "{ALT_ON}{HIDE}");
    let action = 'keys: loop {
        if let Err(e) = draw(&mut out, &m.lines()) {
            break Err(format!("drawing the menu: {e}"));
        }
        match crate::term::read_keys() {
            Ok(keys) => {
                for k in keys {
                    match m.key(k) {
                        Action::None => {}
                        a => break 'keys Ok(a),
                    }
                }
            }
            Err(e) => break Err(format!("reading a key: {e}")),
        }
    };
    let _ = write!(out, "{SHOW}{ALT_OFF}");
    let _ = out.flush();
    drop(raw);
    Ok((m.s, action?))
}

// ---- Without a terminal -----------------------------------------------------------

/// Whether the menu can draw and read keys here: standard input and output
/// are a terminal, on Windows the console (Git Bash's mintty gives pipes,
/// which std counts as a terminal but which are no console).
fn interactive() -> bool {
    if std::env::var("TERM").is_ok_and(|t| t == "dumb") {
        return false;
    }
    crate::term::is_console()
}

/// The plain prompt: the saved choice, Enter to run it, or the number of
/// another entry (with the saved options).
fn prompt(s: Settings, host: &Host) -> Result<(Settings, Action)> {
    use std::io::BufRead;
    if cfg!(windows) && std::env::var_os("MSYSTEM").is_some() {
        eprintln!("(Git Bash's terminal is not a Windows console: `winpty cargo xtask start` shows the full menu)");
    }
    eprintln!("Poly: cargo xtask start (settings in target/{SETTINGS_FILE})");
    for (i, e) in ENTRIES.iter().enumerate() {
        let mark = if *e == s.entry { '*' } else { ' ' };
        let tail = match availability(*e, host) {
            Avail::No(why) => format!("(not here: {why})"),
            Avail::First { why, .. } => why,
            Avail::Ready => e.about().to_string(),
        };
        eprintln!(" {mark}{:>2}. {:<30} {tail}", i + 1, e.title());
    }
    match plan(&s, host) {
        Ok(jobs) => {
            for j in &jobs {
                eprintln!("   last: {}", j.line());
            }
        }
        Err(why) => eprintln!("   last: cannot run here: {why}"),
    }
    eprint!("Enter: run the last again; a number: run that entry; b: build only; q: quit > ");
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| format!("standard input: {e}"))?;
    let answer = line.trim();
    let mut s = s;
    let action = match answer {
        "" => Action::Run { build_only: false },
        "b" => Action::Run { build_only: true },
        "q" => Action::Quit,
        n => match n.parse::<usize>().ok().and_then(|i| ENTRIES.get(i.wrapping_sub(1))) {
            Some(e) => {
                s.entry = *e;
                Action::Run { build_only: false }
            }
            None => return Err(format!("{n:?}: not an entry")),
        },
    };
    Ok((s, action))
}

// ---- The command ------------------------------------------------------------------

/// `cargo xtask start [--last | --print]`.
pub fn command(args: &[String]) -> Result {
    let last = args.iter().any(|a| a == "--last");
    let print = args.iter().any(|a| a == "--print");
    if let Some(bad) = args.iter().find(|a| !matches!(a.as_str(), "--last" | "--print")) {
        return Err(format!("start: unknown option {bad} (expected --last or --print)"));
    }
    let host = Host::probe();
    let saved = Settings::load();
    if print {
        for j in plan(&saved, &host)? {
            println!("{}", j.line());
        }
        return Ok(());
    }
    let (s, action) = if last {
        (saved, Action::Run { build_only: false })
    } else if interactive() {
        menu(saved, host.clone())?
    } else {
        prompt(saved, &host)?
    };
    let Action::Run { build_only } = action else { return Ok(()) };
    s.save()?;
    let started = std::time::Instant::now();
    for job in plan(&s, &host)? {
        execute(&job, &s, build_only)?;
    }
    if build_only {
        eprintln!("==> built in {}", progress::took(started.elapsed()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host of 16 GiB and 8 cores (so 12G is the most memory offered).
    const BIG: HostResources = HostResources { memory_mib: Some(16 * 1024), cpus: 8 };

    fn windows() -> Host {
        Host { linux: false, windows: true, busybox_env: false, chromium_disk: false, res: BIG }
    }

    fn linux() -> Host {
        Host { linux: true, windows: false, busybox_env: false, chromium_disk: true, res: BIG }
    }

    #[test]
    fn settings_round_trip() {
        let s = Settings {
            entry: Entry::BootTests,
            release: false,
            accel: "tcg".into(),
            scale: "2".into(),
            size: "2560x1600".into(),
            fresh_state: true,
            fresh_disk: true,
            kernel_args: "poly.x=1 poly.y".into(),
            display: "gtk,zoom-to-fit=on".into(),
            groups: vec!["desktop".into(), "wayland".into()],
            parts: vec![],
            memory: "3G".into(),
            cpus: "6".into(),
        };
        assert_eq!(Settings::from_json(&s.to_json().to_string()), s);
    }

    #[test]
    fn corrupt_or_old_settings_are_the_defaults() {
        let d = Settings::default();
        assert_eq!(Settings::from_json(""), d);
        assert_eq!(Settings::from_json("{not json"), d);
        assert_eq!(Settings::from_json("[1, 2]"), d);
        assert_eq!(Settings::from_json(r#"{"format": 0, "entry": "ci"}"#), d);
        // Wrong values fall back one by one.
        let s = Settings::from_json(
            r#"{"format": 1, "entry": "nope", "accel": "hvf", "scale": "9", "size": "10x10", "release": "yes",
                "groups": ["desktop", "no-such-group", 3], "display": "a b"}"#,
        );
        assert_eq!(s.entry, Entry::Desktop);
        assert_eq!((s.accel.as_str(), s.scale.as_str(), s.size.as_str()), ("auto", "auto", "auto"));
        assert!(s.release);
        assert_eq!(s.groups, ["desktop"]);
        assert_eq!(s.display, "auto");
    }

    #[test]
    fn command_lines() {
        let mut s = Settings::default();
        assert_eq!(args(Entry::Desktop, &s), ["desktop", "--release"]);
        s.scale = "2".into();
        s.size = "2560x1600".into();
        s.fresh_state = true;
        s.kernel_args = " poly.a=1  poly.b ".into();
        assert_eq!(
            args(Entry::Desktop, &s),
            ["desktop", "--release", "--fresh-state", "--scale=2", "--size=2560x1600", "poly.a=1", "poly.b"]
        );
        s.release = false;
        s.fresh_disk = true;
        assert_eq!(args(Entry::ChromiumRun, &s), ["chromium", "run", "--fresh-disk"]);
        assert_eq!(args(Entry::ChromiumDisk, &s), ["chromium", "disk"]);
        assert_eq!(args(Entry::Busybox, &s), ["run", "--busybox", "poly.a=1", "poly.b"]);
        assert_eq!(args(Entry::DiskRoot, &s), ["run", "--disk-root", "--fresh-disk", "poly.a=1", "poly.b"]);
        assert_eq!(args(Entry::BootTests, &s), ["boot-tests", "desktop", "desktop-shell"]);
        s.parts.clear();
        assert_eq!(args(Entry::Ci, &s), ["ci"]);
        // Every command line is one xtask knows.
        for e in ENTRIES {
            assert!(crate::COMMANDS.contains(&args(e, &s)[0].as_str()), "{e:?}");
        }
    }

    #[test]
    fn the_environment_is_set_or_left_to_xtask() {
        let mut s = Settings::default();
        let j = &plan(&s, &linux()).unwrap()[0];
        assert_eq!(j.env, [(crate::accel::ENV, None), ("POLY_QEMU_DISPLAY", None)]);
        assert_eq!(j.line(), "cargo xtask desktop --release");
        s.accel = "tcg".into();
        s.display = "gtk".into();
        let j = &plan(&s, &linux()).unwrap()[0];
        assert_eq!(
            j.line(),
            "cargo xtask desktop --release   (environment: POLY_QEMU_ACCEL=tcg POLY_QEMU_DISPLAY=gtk)"
        );
        s.entry = Entry::Ci;
        assert!(plan(&s, &linux()).unwrap()[0].env.is_empty());
    }

    #[test]
    fn what_cannot_run_here_says_why_and_prerequisites_come_first() {
        let w = windows();
        for e in [Entry::Toolkits, Entry::DiskRoot, Entry::Busybox] {
            match availability(e, &w) {
                Avail::No(why) => assert!(why.contains("Linux host") || why.contains("BusyBox"), "{why}"),
                other => panic!("{e:?}: {other:?}"),
            }
            assert_eq!(availability(e, &linux()), Avail::Ready, "{e:?}");
        }
        let with_busybox = Host { busybox_env: true, ..windows() };
        assert_eq!(availability(Entry::Busybox, &with_busybox), Avail::Ready);
        let s = Settings { entry: Entry::ChromiumRun, ..Settings::default() };
        let jobs = plan(&s, &w).unwrap();
        assert_eq!(jobs.iter().map(|j| j.entry).collect::<Vec<_>>(), [Entry::ChromiumDisk, Entry::ChromiumRun]);
        assert_eq!(plan(&s, &linux()).unwrap().len(), 1);
        let s = Settings { entry: Entry::Toolkits, ..Settings::default() };
        assert!(plan(&s, &w).unwrap_err().contains("Linux host"));
    }

    #[test]
    fn enter_runs_the_last_choice() {
        let mut m = Menu::new(Settings::default(), windows());
        assert_eq!(m.focused(), Row::Start);
        assert_eq!(m.key(Key::Enter), Action::Run { build_only: false });
        assert_eq!(m.key(Key::Right), Action::None);
        assert_eq!(m.focused(), Row::BuildOnly);
        assert_eq!(m.key(Key::Enter), Action::Run { build_only: true });
        assert_eq!(m.key(Key::Esc), Action::Quit);
    }

    #[test]
    fn choosing_entries_and_options() {
        let mut m = Menu::new(Settings::default(), windows());
        m.focus = 0;
        assert_eq!(m.key(Key::Up), Action::None);
        assert_eq!(m.focused(), Row::Quit);
        // The toolkits are greyed on Windows: not chosen, the reason shown.
        m.focus_on(Row::Entry(Entry::Toolkits));
        m.key(Key::Enter);
        assert_eq!(m.s.entry, Entry::Desktop);
        assert!(m.message.contains("Linux host"), "{}", m.message);
        assert!(m.lines().iter().flatten().any(|(_, t)| t.contains("poly-dev container")));
        // Chromium: chosen, with what comes first.
        m.focus_on(Row::Entry(Entry::ChromiumRun));
        m.key(Key::Enter);
        assert_eq!(m.s.entry, Entry::ChromiumRun);
        assert!(m.message.contains("Chromium's disk"));
        assert!(m.rows().contains(&Row::Opt(Opt::FreshDisk)) && !m.rows().contains(&Row::Opt(Opt::Scale)));
        // Options: cycle the accelerator through what Windows has.
        m.focus_on(Row::Opt(Opt::Accel));
        let seen: Vec<String> = (0..3)
            .map(|_| {
                m.key(Key::Right);
                m.s.accel.clone()
            })
            .collect();
        assert_eq!(seen, ["whpx", "tcg", "auto"]);
        m.focus_on(Row::Opt(Opt::Release));
        m.key(Key::Space);
        assert!(!m.s.release);
    }

    #[test]
    fn typing_a_size() {
        let mut m = Menu::new(Settings::default(), windows());
        m.focus_on(Row::Opt(Opt::Size));
        m.key(Key::Enter);
        for c in "12x3".chars() {
            m.key(Key::Char(c));
        }
        m.key(Key::Enter);
        assert!(m.edit.is_some(), "a bad size stays in the editor");
        assert!(m.message.contains("WxH"));
        for _ in 0..4 {
            m.key(Key::Backspace);
        }
        for c in "1920X1200".chars() {
            m.key(Key::Char(c));
        }
        m.key(Key::Enter);
        assert_eq!((m.edit.is_none(), m.s.size.as_str()), (true, "1920x1200"));
        // Esc leaves it as it was.
        m.key(Key::Enter);
        m.key(Key::Char('9'));
        m.key(Key::Esc);
        assert_eq!(m.s.size, "1920x1200");
        // Left and Right step through the presets.
        m.key(Key::Right);
        assert_eq!(m.s.size, "2560x1600");
    }

    #[test]
    fn picking_boot_test_groups() {
        let s = Settings { entry: Entry::BootTests, groups: vec![], ..Settings::default() };
        let mut m = Menu::new(s, linux());
        m.focus_on(Row::Opt(Opt::Groups));
        m.key(Key::Enter);
        assert!(m.picker.is_some());
        m.key(Key::Down);
        m.key(Key::Space);
        m.key(Key::Up);
        m.key(Key::Space);
        m.key(Key::Enter);
        // In the list's order, whatever order they were picked in.
        assert_eq!(m.s.groups, [crate::BOOT_GROUPS[0].0, crate::BOOT_GROUPS[1].0]);
        m.key(Key::Enter);
        m.key(Key::Char('a'));
        m.key(Key::Esc);
        assert_eq!(m.s.groups.len(), 2, "Esc keeps what was chosen before");
        assert!(m.lines().iter().flatten().any(|(_, t)| t.contains("cargo xtask boot-tests selftest-")));
    }

    fn texts(m: &Menu) -> String {
        m.lines().iter().map(|l| l.iter().map(|(_, t)| t.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn settings_without_resources_load_with_the_defaults() {
        // A file from before the resources: every other value is kept.
        let s = Settings::from_json(r#"{"format": 1, "entry": "toolkits", "release": false, "scale": "2"}"#);
        assert_eq!((s.entry, s.release, s.scale.as_str()), (Entry::Toolkits, false, "2"));
        assert_eq!((s.memory.as_str(), s.cpus.as_str()), ("auto", "auto"));
        // Sizes are kept in their shortest form; wrong values fall back one by one.
        let s = Settings::from_json(r#"{"format": 1, "memory": "4096M", "cpus": "4", "size": "1920x1200"}"#);
        assert_eq!((s.memory.as_str(), s.cpus.as_str(), s.size.as_str()), ("4G", "4", "1920x1200"));
        for (memory, cpus) in [("lots", "0"), ("64M", "17"), ("4T", "two"), ("", "-1")] {
            let s = Settings::from_json(&format!(r#"{{"format": 1, "memory": "{memory}", "cpus": "{cpus}"}}"#));
            assert_eq!((s.memory.as_str(), s.cpus.as_str()), ("auto", "auto"), "{memory:?} {cpus:?}");
        }
        let s = Settings::from_json(r#"{"format": 1, "memory": 4, "cpus": 4}"#);
        assert_eq!((s.memory.as_str(), s.cpus.as_str()), ("auto", "auto"), "numbers are not sizes");
        // Saved and loaded again.
        let s = Settings { memory: "6G".into(), cpus: "16".into(), ..Settings::default() };
        assert_eq!(Settings::from_json(&s.to_json().to_string()), s);
    }

    #[test]
    fn resources_become_flags_of_the_interactive_boots_only() {
        let mut s = Settings::default();
        for e in ENTRIES {
            assert!(!args(e, &s).iter().any(|a| a.starts_with("--memory") || a.starts_with("--cpus")), "{e:?}");
        }
        s.memory = "4G".into();
        s.cpus = "3".into();
        s.kernel_args = "poly.a=1".into();
        assert_eq!(
            args(Entry::Desktop, &s),
            ["desktop", "--release", "--memory=4G", "--cpus=3", "poly.a=1"],
            "before the kernel arguments"
        );
        assert_eq!(args(Entry::ChromiumRun, &s), ["chromium", "run", "--release", "--memory=4G", "--cpus=3"]);
        assert_eq!(args(Entry::Toolkits, &s), ["toolkits", "--release", "--memory=4G", "--cpus=3"]);
        assert_eq!(args(Entry::Shell, &s), ["run", "--release", "--memory=4G", "--cpus=3", "poly.a=1"]);
        assert_eq!(args(Entry::Busybox, &s), ["run", "--release", "--busybox", "--memory=4G", "--cpus=3", "poly.a=1"]);
        assert_eq!(
            args(Entry::DiskRoot, &s),
            ["run", "--release", "--disk-root", "--memory=4G", "--cpus=3", "poly.a=1"]
        );
        // The boot tests and CI keep their fixed sizes; so does building the disk.
        for e in [Entry::BootTests, Entry::Ci, Entry::ChromiumDisk] {
            assert!(!args(e, &s).iter().any(|a| a.starts_with("--memory") || a.starts_with("--cpus")), "{e:?}");
            assert!(e.resources().is_empty() && e.default_resources().is_none(), "{e:?}");
        }
        // One of them alone.
        s.cpus = "auto".into();
        assert_eq!(args(Entry::Toolkits, &s), ["toolkits", "--release", "--memory=4G"]);
        // What --print shows is what the subcommand takes.
        let s = Settings { memory: "4G".into(), cpus: "3".into(), ..Settings::default() };
        assert_eq!(plan(&s, &linux()).unwrap()[0].line(), "cargo xtask desktop --release --memory=4G --cpus=3");
        for e in ENTRIES {
            let a = args(e, &s);
            if !e.resources().is_empty() {
                let (r, notes) = resources::Resources::parse(&a, &BIG).unwrap();
                assert_eq!((r.memory_mib, r.cpus, notes.len()), (Some(4096), Some(3), 0), "{e:?}");
            }
        }
    }

    #[test]
    fn every_entry_that_takes_resources_knows_its_defaults() {
        for e in ENTRIES {
            assert_eq!(e.resources().is_empty(), e.default_resources().is_none(), "{e:?}");
        }
        assert_eq!(Entry::Desktop.default_resources(), Some(("512M", 2)));
        assert_eq!(Entry::ChromiumRun.default_resources(), Some(("4G", 4)));
        assert_eq!(Entry::Toolkits.default_resources(), Some(("2G", 2)));
        assert_eq!(Entry::Shell.default_resources(), Some(("256M", 2)));
        assert_eq!(Entry::DiskRoot.default_resources(), Some(("1G", 2)));
        for e in ENTRIES {
            if let Some((memory, _)) = e.default_resources() {
                assert!(resources::parse_memory(memory).is_some(), "{e:?}: {memory}");
            }
        }
    }

    #[test]
    fn memory_and_cpus_step_through_what_the_host_gives() {
        let mut m = Menu::new(Settings::default(), linux());
        assert!(m.rows().contains(&Row::Opt(Opt::Memory)) && m.rows().contains(&Row::Opt(Opt::Cpus)));
        m.focus_on(Row::Opt(Opt::Memory));
        let seen: Vec<String> = (0..7)
            .map(|_| {
                m.key(Key::Right);
                m.s.memory.clone()
            })
            .collect();
        // 16G is more than the 12G this host can give: skipped.
        assert_eq!(seen, ["512M", "1G", "2G", "4G", "8G", "auto", "512M"]);
        m.key(Key::Left);
        assert_eq!(m.s.memory, "auto");
        m.key(Key::Left);
        assert_eq!(m.s.memory, "8G", "Left from auto goes to the biggest");
        m.focus_on(Row::Opt(Opt::Cpus));
        let seen: Vec<String> = (0..10)
            .map(|_| {
                m.key(Key::Space);
                m.s.cpus.clone()
            })
            .collect();
        assert_eq!(seen, ["1", "2", "3", "4", "5", "6", "7", "8", "auto", "1"]);
    }

    #[test]
    fn what_the_host_cannot_give_is_greyed_with_the_reason() {
        let m = Menu::new(Settings::default(), linux());
        let text = texts(&m);
        assert!(text.contains("Resources for Desktop"), "{text}");
        assert!(text.contains("= 512M, the default") && text.contains("= 2, the default"), "{text}");
        assert!(text.contains("16G greyed: this host has 16.0 GiB; 12G at most"), "{text}");
        // The 16G choice is drawn dim, 8G is not.
        let strip = m.lines().into_iter().find(|l| l.iter().any(|(_, t)| t.trim() == "16G")).unwrap();
        assert!(strip.iter().any(|(k, t)| *k == Look::Dim && t.trim() == "16G"));
        assert!(strip.iter().any(|(k, t)| *k == Look::Plain && t.trim() == "8G"));
        assert!(text.contains("this host has 8 logical cores; the kernel starts every one, up to 16"), "{text}");
        // A host with more cores than the kernel starts says so.
        let many = Host { res: HostResources { memory_mib: None, cpus: 64 }, ..linux() };
        let m = Menu::new(Settings::default(), many);
        let text = texts(&m);
        assert!(text.contains("the kernel starts at most 16 of them"), "{text}");
        assert!(text.contains("this host's memory is unknown: nothing greyed"), "{text}");
        assert_eq!(m.memory_choices().len(), 1 + MEMORY_PRESETS.len());
        assert_eq!(m.cpu_choices().len(), 1 + 16);
        // No resources section for what does not boot interactively.
        let s = Settings { entry: Entry::BootTests, ..Settings::default() };
        let m = Menu::new(s, linux());
        assert!(!texts(&m).contains("Resources for") && !m.rows().contains(&Row::Opt(Opt::Memory)));
    }

    #[test]
    fn the_menu_warns_where_a_value_will_not_be_taken_as_it_is() {
        // A saved size below the entry's default.
        let s = Settings { entry: Entry::ChromiumRun, memory: "1G".into(), ..Settings::default() };
        let text = texts(&Menu::new(s, linux()));
        assert!(text.contains("below the default 4G: it may not boot"), "{text}");
        // Saved on a bigger host: Start clamps it.
        let s = Settings { memory: "16G".into(), cpus: "16".into(), ..Settings::default() };
        let text = texts(&Menu::new(s, linux()));
        assert!(text.contains("more than this host can give: Start uses 12G"), "{text}");
        assert!(text.contains("more than this host has: Start uses 8"), "{text}");
    }

    #[test]
    fn typing_memory_and_cpus() {
        let mut m = Menu::new(Settings::default(), linux());
        let type_in = |m: &mut Menu, o: Opt, text: &str| {
            m.focus_on(Row::Opt(o));
            m.key(Key::Enter);
            for c in text.chars() {
                m.key(Key::Char(c));
            }
            m.key(Key::Enter);
        };
        type_in(&mut m, Opt::Memory, "64G");
        assert!(m.edit.is_some() && m.s.memory == "auto", "too much stays in the editor");
        assert!(m.message.contains("more than this host can give") && m.message.contains("12G"), "{}", m.message);
        m.key(Key::Esc);
        type_in(&mut m, Opt::Memory, "lots");
        assert!(m.edit.is_some() && m.message.contains("such as 4G or 3072M"), "{}", m.message);
        m.key(Key::Esc);
        type_in(&mut m, Opt::Memory, "3072m");
        assert_eq!((m.edit.is_none(), m.s.memory.as_str()), (true, "3G"));
        type_in(&mut m, Opt::Cpus, "9");
        assert!(m.edit.is_some() && m.message.contains("8 logical cores"), "{}", m.message);
        m.key(Key::Esc);
        type_in(&mut m, Opt::Cpus, "0");
        assert!(m.edit.is_some() && m.message.contains("from 1 to 16"), "{}", m.message);
        m.key(Key::Esc);
        type_in(&mut m, Opt::Cpus, "6");
        assert_eq!((m.edit.is_none(), m.s.cpus.as_str()), (true, "6"));
        // The editor starts on the value; empty or auto is auto again.
        m.focus_on(Row::Opt(Opt::Memory));
        m.key(Key::Enter);
        assert_eq!(m.edit.as_ref().map(|(_, t)| t.as_str()), Some("3G"));
        for _ in 0..2 {
            m.key(Key::Backspace);
        }
        m.key(Key::Enter);
        assert_eq!((m.edit.is_none(), m.s.memory.as_str()), (true, "auto"));
        // The line Start runs.
        assert_eq!(plan(&m.s, &m.host).unwrap()[0].line(), "cargo xtask desktop --release --cpus=6");
    }
}
