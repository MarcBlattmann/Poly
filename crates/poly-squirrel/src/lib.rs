//! Squirrel installers, unpacked by Poly (the owner's decision for batch W6 of
//! `docs/windows/apps-survey.md`: no .NET Framework). A Squirrel `Setup.exe` (Discord, Slack and
//! other apps) carries its app as a NuGet package and installs it with `Update.exe`, a .NET
//! Framework program Poly cannot run. Poly does the install itself, from the files' formats:
//!
//! - **The bundle** ([`bundle`]): a program resource of the named type `DATA` holds a zip archive
//!   (APPNOTE.TXT) with `RELEASES`, `Update.exe` and the full package `<id>-<version>-full.nupkg`.
//!   `RELEASES` lines are `<SHA-1> <file> <size>`.
//! - **The package**: a NuGet package is an Open Packaging Conventions zip (ECMA-376 Part 2) with a
//!   manifest `<id>.nuspec` (Learn, ".nuspec reference": `id`, `version`, `title`, `authors`,
//!   `description` in `package/metadata`) and the app under `lib/net45/`.
//! - **The install** ([`plan`]), as Squirrel's public user documentation describes it: the app in
//!   `%LOCALAPPDATA%\<id>\app-<version>\`, `Update.exe` beside the version folders, the package and
//!   `RELEASES` in `packages\`; a Start menu shortcut in a folder named for the publisher and one on
//!   the desktop; the per-user `Uninstall` key (Learn, "Uninstall Registry Key"). Poly's shortcuts
//!   start the app's program directly (Squirrel's own start `Update.exe --processStart`, which is
//!   the .NET program); `Update.exe` is still put where the app expects it.
//!
//! Nothing of Squirrel itself was read. `polysquirrel.exe` (ADR 0256) does the writing on the NT
//! personality; Poly's installer window recognises a Squirrel installer with [`bundle`].

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_zip::{Archive, Entry};

/// The resource type (a name) whose first resource is the bundle.
pub const RESOURCE_TYPE: &str = "DATA";

/// The bundle's bytes in a program image (the whole file): the first resource of the type
/// [`RESOURCE_TYPE`], its first language.
pub fn bundle_resource(image: &[u8]) -> Option<&[u8]> {
    use poly_pe::ResourceKey;
    let pe = poly_pe::Pe::parse(image).ok()?;
    let root = pe.resources().ok()??;
    let names = root.find(ResourceKey::Name(RESOURCE_TYPE)).ok()??.directory().ok()??;
    let langs = names.entries().next()?.ok()?.directory().ok()??;
    let leaf = langs.entries().next()?.ok()?.data().ok()??;
    pe.resource_bytes(&leaf).ok()
}

/// The bundle's entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    /// The full package.
    pub package: Entry,
    /// `Update.exe`, if the bundle has it.
    pub update: Option<Entry>,
    /// `RELEASES`, if the bundle has it.
    pub releases: Option<Entry>,
    /// The id and version the package's file name gives.
    pub id: String,
    pub version: String,
}

/// The Squirrel bundle of an archive: a top-level `<id>-<version>-full.nupkg` with `RELEASES` or
/// `Update.exe` beside it.
pub fn bundle(a: &Archive) -> Option<Bundle> {
    let top = |e: &&Entry| !e.name.contains('/');
    let package = a.entries.iter().filter(top).find(|e| package_file_name(&e.name).is_some())?.clone();
    let (id, version) = package_file_name(&package.name)?;
    let update = a.entries.iter().filter(top).find(|e| e.name.eq_ignore_ascii_case("Update.exe")).cloned();
    let releases = a.entries.iter().filter(top).find(|e| e.name.eq_ignore_ascii_case("RELEASES")).cloned();
    if update.is_none() && releases.is_none() {
        return None;
    }
    Some(Bundle { package, update, releases, id, version })
}

/// The id and version of a full package's file name, `<id>-<version>-full.nupkg` (the id may have
/// dashes; the version starts with a digit).
pub fn package_file_name(name: &str) -> Option<(String, String)> {
    let lower = name.to_ascii_lowercase();
    let stem = &name[..lower.strip_suffix("-full.nupkg")?.len()];
    let dash =
        stem.char_indices().rfind(|&(i, c)| c == '-' && stem[i + 1..].starts_with(|d: char| d.is_ascii_digit()))?.0;
    let (id, version) = (&stem[..dash], &stem[dash + 1..]);
    (!id.is_empty() && !version.is_empty()).then(|| (id.to_string(), version.to_string()))
}

/// What a package's manifest says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spec {
    pub id: String,
    pub version: String,
    /// `title`, else the id.
    pub title: String,
    /// `authors` (a comma-separated list).
    pub authors: String,
    pub description: String,
}

/// The text of the first `<tag>...</tag>` in `xml`, its character references decoded.
fn element(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&close)?;
    Some(unescape(xml[start..end].trim()))
}

/// XML's predefined entities and numeric character references decoded.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';') else { break };
        let name = &rest[1..semi];
        let c = match name {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The manifest's metadata (a `.nuspec` file's text).
pub fn nuspec(xml: &str) -> Option<Spec> {
    let xml = xml.trim_start_matches('\u{feff}');
    let meta = &xml[xml.find("<metadata")?..];
    let id = element(meta, "id")?;
    let version = element(meta, "version")?;
    let title = element(meta, "title").filter(|t| !t.is_empty()).unwrap_or_else(|| id.clone());
    Some(Spec {
        authors: element(meta, "authors").unwrap_or_default(),
        description: element(meta, "description").unwrap_or_default(),
        id,
        version,
        title,
    })
}

/// The package's manifest: its top-level `.nuspec` entry.
pub fn spec_entry(a: &Archive) -> Option<&Entry> {
    a.entries.iter().find(|e| !e.name.contains('/') && e.name.to_ascii_lowercase().ends_with(".nuspec"))
}

/// Where the app's files are in the package.
pub const APP_FOLDER: &str = "lib/net45/";

/// The package's app files: (entry, path below the app folder with `\`, decoded).
pub fn app_files(a: &Archive) -> Vec<(&Entry, String)> {
    a.entries
        .iter()
        .filter(|e| !e.is_dir())
        .filter_map(|e| {
            let rest = e
                .name
                .get(APP_FOLDER.len()..)
                .filter(|_| e.name[..APP_FOLDER.len()].eq_ignore_ascii_case(APP_FOLDER))?;
            let rel = poly_zip::part_name(rest).replace('/', "\\");
            (!rel.is_empty() && !rel.split('\\').any(|p| p.is_empty() || p == "." || p == "..")).then_some((e, rel))
        })
        .collect()
}

/// The app's program among the app files (paths below the app folder): `<id>.exe`, else
/// `<title>.exe`, else the only program in the folder itself that is not Squirrel's.
pub fn main_program(files: &[String], spec: &Spec) -> Option<String> {
    let top: Vec<&String> =
        files.iter().filter(|f| !f.contains('\\') && f.to_ascii_lowercase().ends_with(".exe")).collect();
    for want in [format!("{}.exe", spec.id), format!("{}.exe", spec.title)] {
        if let Some(f) = top.iter().find(|f| f.eq_ignore_ascii_case(&want)) {
            return Some((*f).clone());
        }
    }
    let own: Vec<&&String> = top.iter().filter(|f| !f.to_ascii_lowercase().contains("squirrel")).collect();
    (own.len() == 1).then(|| (*own[0]).clone())
}

/// A name made safe as one file name: the characters Windows refuses in a name replaced by `_`,
/// trailing dots and spaces dropped.
pub fn file_name_safe(s: &str) -> String {
    let t: String = s.chars().map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 32 { '_' } else { c }).collect();
    let t = t.trim_end_matches(['.', ' ']).trim();
    if t.is_empty() { "App".to_string() } else { t.to_string() }
}

/// The folders an install uses (DOS paths without a trailing backslash).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    /// `%LOCALAPPDATA%`.
    pub local_app_data: String,
    /// The user's Start menu programs (`%APPDATA%\Microsoft\Windows\Start Menu\Programs`).
    pub programs: String,
    /// The user's desktop.
    pub desktop: String,
}

/// The two shortcuts of an app: in the Start menu, in a folder named for its first author (none
/// without authors), and on the desktop, both named for its title.
pub fn shortcuts(places: &Places, title: &str, authors: &str) -> [String; 2] {
    let name = format!("{}.lnk", file_name_safe(title));
    let company = authors.split(',').next().unwrap_or("").trim();
    let start = if company.is_empty() {
        format!("{}\\{name}", places.programs)
    } else {
        format!("{}\\{}\\{name}", places.programs, file_name_safe(company))
    };
    [start, format!("{}\\{name}", places.desktop)]
}

/// The registry path of the app's `Uninstall` key, below `HKEY_CURRENT_USER`.
pub fn uninstall_key(id: &str) -> String {
    format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}", file_name_safe(id))
}

/// Poly's program that installs and uninstalls Squirrel apps.
pub const POLYSQUIRREL: &str = "C:\\Windows\\System32\\polysquirrel.exe";

/// A value of the `Uninstall` key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Number(u32),
}

/// An install, worked out before anything is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// `%LOCALAPPDATA%\<id>`.
    pub root: String,
    /// `<root>\app-<version>`.
    pub app_dir: String,
    /// The app's program, a full path.
    pub program: String,
    /// `<root>\Update.exe`.
    pub update: String,
    /// `<root>\packages`.
    pub packages: String,
    /// (index of the entry in the package, full path) of each app file.
    pub files: Vec<(usize, String)>,
    /// The Start menu and desktop shortcuts.
    pub shortcuts: [String; 2],
    /// The `Uninstall` key (below `HKEY_CURRENT_USER`) and its values.
    pub key: String,
    pub values: Vec<(&'static str, Value)>,
}

/// The install of package `pkg` with manifest `spec` into `places`, or why not.
pub fn plan(spec: &Spec, pkg: &Archive, places: &Places) -> Result<Plan, &'static str> {
    let files = app_files(pkg);
    if files.is_empty() {
        return Err("the package has no files in lib\\net45");
    }
    let rels: Vec<String> = files.iter().map(|(_, r)| r.clone()).collect();
    let program = main_program(&rels, spec).ok_or("the package's program could not be told from the others")?;
    let root = format!("{}\\{}", places.local_app_data, file_name_safe(&spec.id));
    let app_dir = format!("{root}\\app-{}", file_name_safe(&spec.version));
    let index = |e: &Entry| pkg.entries.iter().position(|x| core::ptr::eq(x, e)).unwrap_or(0);
    let files: Vec<(usize, String)> = files.iter().map(|(e, r)| (index(e), format!("{app_dir}\\{r}"))).collect();
    let size_kib = pkg.entries.iter().map(|e| e.size).sum::<u64>().div_ceil(1024);
    let program = format!("{app_dir}\\{program}");
    let id = file_name_safe(&spec.id);
    let values = alloc::vec![
        ("DisplayName", Value::Text(spec.title.clone())),
        ("DisplayVersion", Value::Text(spec.version.clone())),
        ("Publisher", Value::Text(spec.authors.clone())),
        ("InstallLocation", Value::Text(root.clone())),
        ("DisplayIcon", Value::Text(program.clone())),
        ("UninstallString", Value::Text(format!("\"{POLYSQUIRREL}\" --uninstall {id}"))),
        ("QuietUninstallString", Value::Text(format!("\"{POLYSQUIRREL}\" --uninstall {id}"))),
        ("EstimatedSize", Value::Number(u32::try_from(size_kib).unwrap_or(u32::MAX))),
        ("NoModify", Value::Number(1)),
        ("NoRepair", Value::Number(1)),
    ];
    Ok(Plan {
        update: format!("{root}\\Update.exe"),
        packages: format!("{root}\\packages"),
        shortcuts: shortcuts(places, &spec.title, &spec.authors),
        key: uninstall_key(&spec.id),
        values,
        files,
        program,
        app_dir,
        root,
    })
}

/// The folders of `path` (a full DOS path of a file) from the outermost down, to be made in order:
/// `C:\a\b\c.txt` gives `C:\a`, `C:\a\b`.
pub fn parent_folders(path: &str) -> Vec<&str> {
    path.match_indices('\\').skip(1).map(|(i, _)| &path[..i]).collect()
}

#[cfg(test)]
mod tests;
