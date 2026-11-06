//! The trash, per the freedesktop.org Trash specification 1.0 (the home
//! trash only).
//!
//! The trash folder is `$XDG_DATA_HOME/Trash` (`$HOME/.local/share/Trash`
//! by default, [`trash_dir`]) with `files/` holding the trashed files and
//! `info/` one `<name>.trashinfo` per file:
//!
//! ```text
//! [Trash Info]
//! Path=/home/u/My%20File.txt
//! DeletionDate=2026-11-01T00:00:00
//! ```
//!
//! **Naming.** As the spec requires, the info file is created first, with
//! create-exclusive semantics ([`Fs::create_new`]), which reserves the name
//! atomically; then the file is renamed into `files/`. The first try is
//! the file's own name; when taken, a counter goes before the extension:
//! `photo 2.jpg`, `photo 3.jpg`, … (the name is never shown, the list shows
//! the original path). A name whose `files/` entry exists without an info
//! file (left by a crash) is skipped.
//!
//! **Path** is the absolute original path, percent-encoded as in RFC 2396
//! (everything but unreserved characters and `/` escaped as `%XX` of the
//! UTF-8 bytes).
//!
//! **DeletionDate** should be local time; the core is only given Unix
//! seconds, so it writes UTC (offset 0), converted with a days-to-civil
//! calendar computation (proleptic Gregorian).
//!
//! **Other disks.** Files on another file system than the home trash would
//! need that disk's `$topdir/.Trash-$uid`; that is out of scope: the rename
//! fails with [`FsError::CrossDevice`], the info file is removed again, and
//! the error is returned so the UI can offer to delete permanently.
//!
//! The trash folder is created with mode `0700` where the file system has
//! permission bits; `files/` and `info/` are created when missing.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::fs::{Fs, FsError, Kind, create_dir_all};
use crate::ops::{delete_permanently, validate_name};
use crate::path::{file_name, is_inside, join, normalize, parent, percent_decode, percent_encode, split_ext};

/// The home trash folder: `$XDG_DATA_HOME/Trash` if that variable is set
/// to an absolute path (the XDG Base Directory spec ignores relative
/// ones), else `$HOME/.local/share/Trash`.
pub fn trash_dir(home: &str, xdg_data_home: Option<&str>) -> String {
    match xdg_data_home.filter(|d| d.starts_with('/')) {
        Some(d) => join(d, "Trash"),
        None => join(home, ".local/share/Trash"),
    }
}

/// Creates the trash folder, `files/` and `info/` if missing.
pub fn ensure(fs: &dyn Fs, trash_dir: &str) -> Result<(), FsError> {
    if !fs.exists(trash_dir) {
        create_dir_all(fs, trash_dir)?;
        match fs.set_mode(trash_dir, 0o700) {
            Ok(()) | Err(FsError::Unsupported) => {}
            Err(e) => return Err(e),
        }
    }
    for sub in ["files", "info"] {
        match fs.create_dir(&join(trash_dir, sub)) {
            Ok(()) | Err(FsError::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// `YYYY-MM-DDThh:mm:ss` for Unix seconds `t` (UTC).
pub fn format_date(t: u64) -> String {
    let days = t / 86_400;
    let secs = t % 86_400;
    let (y, m, d) = civil_from_days(days);
    alloc::format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// The Gregorian date `days` after 1970-01-01: shift the epoch to
/// 0000-03-01 so leap days end each 4/100/400-year cycle, find the 400-year
/// era, the year of the era, the day of the year (March-based) and the
/// month from it.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468; // days from 0000-03-01
    let era = z / 146_097;
    let doe = z % 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // March = 0
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

/// The contents of the info file for `path` deleted at `now`.
pub fn info_text(path: &str, now: u64) -> String {
    alloc::format!("[Trash Info]\nPath={}\nDeletionDate={}\n", percent_encode(path), format_date(now))
}

/// The `n`th name to try for `name`: `name`, then `stem 2.ext`, …
fn candidate(name: &str, n: u32) -> String {
    if n == 1 {
        return name.into();
    }
    let (stem, ext) = split_ext(name);
    alloc::format!("{stem} {n}{ext}")
}

/// How many names [`trash`] tries.
const MAX_TRIES: u32 = 10_000;

/// Moves `path` to the trash at `trash_dir` and returns its name in the
/// trash (see the module docs).
pub fn trash(fs: &dyn Fs, trash_dir: &str, path: &str, now_unix_secs: u64) -> Result<String, FsError> {
    let path = normalize(path);
    let trash_dir = normalize(trash_dir);
    if !path.starts_with('/') || path == "/" || is_inside(&path, &trash_dir) || is_inside(&trash_dir, &path) {
        return Err(FsError::Invalid);
    }
    fs.lstat(&path)?;
    ensure(fs, &trash_dir)?;
    let (files, info) = (join(&trash_dir, "files"), join(&trash_dir, "info"));
    let base = file_name(&path);
    let text = info_text(&path, now_unix_secs);
    for n in 1..=MAX_TRIES {
        let name = candidate(base, n);
        if validate_name(&alloc::format!("{name}.trashinfo")).is_err() {
            return Err(FsError::Invalid);
        }
        let info_path = join(&info, &alloc::format!("{name}.trashinfo"));
        match fs.create_new(&info_path, text.as_bytes()) {
            Ok(()) => {}
            Err(FsError::AlreadyExists) => continue,
            Err(e) => return Err(e),
        }
        let target = join(&files, &name);
        if fs.exists(&target) {
            // An orphan without an info file: leave it, try the next name.
            let _ = fs.remove_file(&info_path);
            continue;
        }
        return match fs.rename(&path, &target) {
            Ok(()) => Ok(name),
            Err(e) => {
                let _ = fs.remove_file(&info_path);
                Err(e)
            }
        };
    }
    Err(FsError::AlreadyExists)
}

/// A file in the trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashItem {
    /// Its name in `files/` (what [`restore`] and [`delete`] take).
    pub name: String,
    /// Where it was, decoded.
    pub original_path: String,
    /// `DeletionDate` as written (`YYYY-MM-DDThh:mm:ss`).
    pub deleted: String,
    /// What it is (not following links).
    pub kind: Kind,
}

/// `Path` and `DeletionDate` of an info file.
fn parse_info(text: &str) -> Option<(String, String)> {
    let mut inside = false;
    let (mut path, mut date) = (None, String::new());
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[Trash Info]";
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(v) = line.strip_prefix("Path=") {
            path = Some(percent_decode(v));
        } else if let Some(v) = line.strip_prefix("DeletionDate=") {
            date = v.to_string();
        }
    }
    Some((path?, date))
}

/// The items in the trash, newest first (then by name). Info files
/// without a file, unreadable ones and files without an info file are
/// left out.
pub fn list(fs: &dyn Fs, trash_dir: &str) -> Result<Vec<TrashItem>, FsError> {
    let info = join(trash_dir, "info");
    let entries = match fs.read_dir(&info) {
        Ok(e) => e,
        Err(FsError::NotFound) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for e in entries {
        let Some(name) = e.name.strip_suffix(".trashinfo") else { continue };
        if name.is_empty() {
            continue;
        }
        let Ok(meta) = fs.lstat(&join(trash_dir, &alloc::format!("files/{name}"))) else { continue };
        let Ok(text) = fs.read_to_string(&join(&info, &e.name)) else { continue };
        let Some((original_path, deleted)) = parse_info(&text) else { continue };
        out.push(TrashItem { name: name.into(), original_path, deleted, kind: meta.kind });
    }
    out.sort_by(|a, b| b.deleted.cmp(&a.deleted).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

fn item_paths(trash_dir: &str, name: &str) -> Result<(String, String), FsError> {
    validate_name(name).map_err(|_| FsError::Invalid)?;
    Ok((join(trash_dir, &alloc::format!("files/{name}")), join(trash_dir, &alloc::format!("info/{name}.trashinfo"))))
}

/// Puts trashed `name` back where it was and returns that path. Refused
/// with [`FsError::AlreadyExists`] when something has taken the place;
/// missing parent folders are created.
pub fn restore(fs: &dyn Fs, trash_dir: &str, name: &str) -> Result<String, FsError> {
    let (file, info) = item_paths(trash_dir, name)?;
    let (original, _) = parse_info(&fs.read_to_string(&info)?).ok_or(FsError::Invalid)?;
    let original = normalize(&original);
    if !original.starts_with('/') || original == "/" {
        return Err(FsError::Invalid);
    }
    fs.lstat(&file)?;
    if fs.exists(&original) {
        return Err(FsError::AlreadyExists);
    }
    if let Some(dir) = parent(&original) {
        create_dir_all(fs, dir)?;
    }
    fs.rename(&file, &original)?;
    let _ = fs.remove_file(&info);
    Ok(original)
}

/// Deletes trashed `name` for good.
pub fn delete(fs: &dyn Fs, trash_dir: &str, name: &str) -> Result<(), FsError> {
    let (file, info) = item_paths(trash_dir, name)?;
    match delete_permanently(fs, &file) {
        Ok(()) | Err(FsError::NotFound) => {}
        Err(e) => return Err(e),
    }
    match fs.remove_file(&info) {
        Ok(()) | Err(FsError::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Deletes everything in the trash (including orphans in `files/` and
/// `info/`).
pub fn empty(fs: &dyn Fs, trash_dir: &str) -> Result<(), FsError> {
    for sub in ["files", "info"] {
        let dir = join(trash_dir, sub);
        let entries = match fs.read_dir(&dir) {
            Ok(e) => e,
            Err(FsError::NotFound) => continue,
            Err(e) => return Err(e),
        };
        for e in entries {
            delete_permanently(fs, &join(&dir, &e.name))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfs::{MemFs, StdFs};

    #[test]
    fn dates() {
        assert_eq!(format_date(0), "1970-01-01T00:00:00");
        assert_eq!(format_date(1_793_491_200), "2026-11-01T00:00:00");
        assert_eq!(format_date(951_782_400 + 3661), "2000-02-29T01:01:01");
        assert_eq!(format_date(1_709_251_199), "2024-02-29T23:59:59");
        assert_eq!(format_date(4_107_542_400), "2100-03-01T00:00:00");
        // No panic at the far end.
        let _ = format_date(u64::MAX);
    }

    #[test]
    fn dirs() {
        assert_eq!(trash_dir("/home/u", None), "/home/u/.local/share/Trash");
        assert_eq!(trash_dir("/home/u", Some("/data")), "/data/Trash");
        assert_eq!(trash_dir("/home/u", Some("relative")), "/home/u/.local/share/Trash");
    }

    #[test]
    fn trash_list_restore_on_disk() {
        let fs = StdFs::new();
        let t = "/home/u/.local/share/Trash";
        crate::fs::create_dir_all(&fs, "/home/u/docs").unwrap();
        fs.write("/home/u/docs/My File ü.txt", b"one").unwrap();
        let name = trash(&fs, t, "/home/u/docs/My File ü.txt", 1_793_491_200).unwrap();
        assert_eq!(name, "My File ü.txt");
        assert!(!fs.exists("/home/u/docs/My File ü.txt"));
        assert_eq!(
            fs.read_head(&alloc::format!("{t}/info/My File ü.txt.trashinfo"), 1000).unwrap(),
            b"[Trash Info]\nPath=/home/u/docs/My%20File%20%C3%BC.txt\nDeletionDate=2026-11-01T00:00:00\n"
        );
        assert_eq!(fs.read_to_string(&alloc::format!("{t}/files/My File ü.txt")).unwrap(), "one");

        // A second file of the same name gets a numbered name.
        fs.write("/home/u/docs/My File ü.txt", b"two").unwrap();
        assert_eq!(trash(&fs, t, "/home/u/docs/My File ü.txt", 0).unwrap(), "My File ü 2.txt");
        fs.create_dir("/home/u/docs/sub").unwrap();
        fs.write("/home/u/docs/sub/x", b"x").unwrap();
        assert_eq!(trash(&fs, t, "/home/u/docs/sub", 5).unwrap(), "sub");

        let items = list(&fs, t).unwrap();
        let got: Vec<(&str, &str, &str, Kind)> =
            items.iter().map(|i| (i.name.as_str(), i.original_path.as_str(), i.deleted.as_str(), i.kind)).collect();
        assert_eq!(
            got,
            [
                ("My File ü.txt", "/home/u/docs/My File ü.txt", "2026-11-01T00:00:00", Kind::File),
                ("sub", "/home/u/docs/sub", "1970-01-01T00:00:05", Kind::Dir),
                ("My File ü 2.txt", "/home/u/docs/My File ü.txt", "1970-01-01T00:00:00", Kind::File),
            ]
        );

        // Restoring onto a taken place is refused; the other one goes back.
        assert_eq!(restore(&fs, t, "My File ü.txt").unwrap(), "/home/u/docs/My File ü.txt");
        assert_eq!(restore(&fs, t, "My File ü 2.txt"), Err(FsError::AlreadyExists));
        assert_eq!(fs.read_to_string("/home/u/docs/My File ü.txt").unwrap(), "one");
        // Missing parents are created.
        crate::ops::delete_permanently(&fs, "/home/u/docs").unwrap();
        assert_eq!(restore(&fs, t, "sub").unwrap(), "/home/u/docs/sub");
        assert_eq!(fs.read_to_string("/home/u/docs/sub/x").unwrap(), "x");
        assert_eq!(list(&fs, t).unwrap().len(), 1);
        assert_eq!(restore(&fs, t, "../../etc"), Err(FsError::Invalid));

        empty(&fs, t).unwrap();
        assert!(list(&fs, t).unwrap().is_empty());
        assert!(fs.read_dir(&alloc::format!("{t}/files")).unwrap().is_empty());
    }

    #[test]
    fn refused_and_cross_device() {
        let fs = MemFs::new().with(&[("/home/u/a", "A"), ("/media/usb/b", "B")]);
        fs.add_device("/media/usb");
        let t = trash_dir("/home/u", None);
        assert_eq!(trash(&fs, &t, "/media/usb/b", 0), Err(FsError::CrossDevice));
        assert!(fs.exists("/media/usb/b"));
        assert!(fs.read_dir(&alloc::format!("{t}/info")).unwrap().is_empty(), "the info file is removed again");
        assert_eq!(fs.stat(&t).unwrap().mode, 0o700);
        assert_eq!(trash(&fs, &t, "/home/u/nope", 0), Err(FsError::NotFound));
        assert_eq!(trash(&fs, &t, "/", 0), Err(FsError::Invalid));
        assert_eq!(trash(&fs, &t, "/home/u", 0), Err(FsError::Invalid), "contains the trash");
        assert_eq!(trash(&fs, &t, &alloc::format!("{t}/files"), 0), Err(FsError::Invalid));
    }

    #[test]
    fn orphans_and_names() {
        let fs = MemFs::new().with(&[("/h/a.tar.gz", "1"), ("/h/T/files/a.tar.gz", "orphan"), ("/h/T/info/", "")]);
        assert_eq!(trash(&fs, "/h/T", "/h/a.tar.gz", 0).unwrap(), "a 2.tar.gz");
        assert_eq!(fs.read_to_string("/h/T/files/a.tar.gz").unwrap(), "orphan");
        assert!(!fs.exists("/h/T/info/a.tar.gz.trashinfo"));
        // Broken info files and orphans are not listed.
        fs.write("/h/T/info/bad.trashinfo", b"nonsense").unwrap();
        fs.write("/h/T/files/bad", b"").unwrap();
        fs.write("/h/T/info/gone.trashinfo", b"[Trash Info]\nPath=/x\n").unwrap();
        let names: Vec<String> = list(&fs, "/h/T").unwrap().into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["a 2.tar.gz"]);
        delete(&fs, "/h/T", "a 2.tar.gz").unwrap();
        assert!(list(&fs, "/h/T").unwrap().is_empty());
        assert_eq!(list(&fs, "/nowhere").unwrap(), []);
    }
}
