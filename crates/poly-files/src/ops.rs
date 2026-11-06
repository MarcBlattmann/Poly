//! File operations: new folder, rename, copy/cut and paste, delete.
//!
//! - [`new_folder_name`]: "New Folder", then "New Folder 2", "New Folder 3"…
//! - [`validate_name`]: what a single file name may be on Poly (not empty,
//!   not `.` or `..`, no `/` or NUL, at most 255 bytes).
//! - [`rename`] never replaces another file: an existing name is
//!   [`FsError::AlreadyExists`] (changing only the case of a name is
//!   allowed, for case-insensitive disks).
//! - [`copy_name`]: the name a pasted copy gets when its name is taken:
//!   "photo (copy).jpg", "photo (copy 2).jpg"; copying a copy does not stack
//!   ("photo (copy) (copy).jpg" is never made). Folders and `archive.tar.gz`
//!   keep their whole extension.
//! - [`paste`]: a **copy** is recursive (folders created, files with
//!   [`Fs::copy_file`]); symbolic links are copied as what they point to
//!   (the [`Fs`] trait has no way to make links), so a link to a folder
//!   copies the folder, at most [`MAX_DEPTH`] levels deep. A taken name
//!   gets a copy name, so pasting into the same folder duplicates. A
//!   **cut** is a rename; across disks ([`FsError::CrossDevice`]) it copies
//!   and then deletes the original (only once the copy is complete; a
//!   failed copy is removed again). A cut onto a taken name fails for that
//!   item with [`FsError::AlreadyExists`] instead of replacing or renaming,
//!   so the UI can ask; a cut into the folder it came from does nothing.
//!   Pasting a folder into itself or below itself is refused before
//!   anything is touched ([`OpError::IntoItself`]). One failing item does
//!   not stop the others; [`PasteReport`] lists both.
//! - [`delete_permanently`]: recursive, never following links (a link is
//!   removed, not what it points to).
//!
//! Checks such as "does the name exist" happen before the operation;
//! another program racing us between check and rename could still be
//! replaced, as the [`Fs`] trait has no "rename unless exists".

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::fs::{Fs, FsError, Kind};
use crate::path::{file_name, is_inside, join, normalize, parent, split_ext};

/// How deep [`paste`] and [`delete_permanently`] go.
pub const MAX_DEPTH: usize = 128;

/// The base name of new folders.
pub const NEW_FOLDER: &str = "New Folder";

/// Why a name is not allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    /// `.` or `..`.
    Dots,
    Slash,
    Nul,
    /// More than 255 bytes.
    TooLong,
}

impl NameError {
    /// A short sentence for the UI.
    pub fn message(self) -> &'static str {
        match self {
            NameError::Empty => "The name cannot be empty",
            NameError::Dots => "\".\" and \"..\" are not allowed",
            NameError::Slash => "The name cannot contain \"/\"",
            NameError::Nul => "The name cannot contain a NUL character",
            NameError::TooLong => "The name is too long",
        }
    }
}

/// Why an operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpError {
    Fs(FsError),
    Name(NameError),
    /// Pasting this folder into itself or a folder below it.
    IntoItself(String),
}

impl From<FsError> for OpError {
    fn from(e: FsError) -> Self {
        OpError::Fs(e)
    }
}

impl From<NameError> for OpError {
    fn from(e: NameError) -> Self {
        OpError::Name(e)
    }
}

/// Whether `name` can name a file.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        Err(NameError::Empty)
    } else if name == "." || name == ".." {
        Err(NameError::Dots)
    } else if name.contains('/') {
        Err(NameError::Slash)
    } else if name.contains('\0') {
        Err(NameError::Nul)
    } else if name.len() > 255 {
        Err(NameError::TooLong)
    } else {
        Ok(())
    }
}

/// The first of "New Folder", "New Folder 2", … not in `existing`.
pub fn new_folder_name<'a>(existing: impl IntoIterator<Item = &'a str>) -> String {
    let taken: Vec<&str> = existing.into_iter().collect();
    if !taken.contains(&NEW_FOLDER) {
        return NEW_FOLDER.into();
    }
    (2u32..).map(|n| alloc::format!("{NEW_FOLDER} {n}")).find(|c| !taken.contains(&c.as_str())).unwrap_or_default()
}

/// Creates a new folder in `dir` with the first free "New Folder" name and
/// returns its name.
pub fn new_folder(fs: &dyn Fs, dir: &str) -> Result<String, FsError> {
    let names: Vec<String> = fs.read_dir(dir)?.into_iter().map(|e| e.name).collect();
    let name = new_folder_name(names.iter().map(String::as_str));
    fs.create_dir(&join(dir, &name))?;
    Ok(name)
}

/// `stem` without a trailing " (copy)" or " (copy N)".
fn strip_copy(stem: &str) -> &str {
    if let Some(s) = stem.strip_suffix(" (copy)") {
        return s;
    }
    if let Some(i) = stem.rfind(" (copy ") {
        let tail = &stem[i + 7..];
        if let Some(n) = tail.strip_suffix(')')
            && !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit())
        {
            return &stem[..i];
        }
    }
    stem
}

/// A name for a copy of `name` that is not in `existing` (see the module
/// docs). `name` itself is returned if it is free.
pub fn copy_name<'a>(existing: impl IntoIterator<Item = &'a str>, name: &str, is_dir: bool) -> String {
    let taken: Vec<&str> = existing.into_iter().collect();
    if !taken.contains(&name) {
        return name.into();
    }
    let (stem, ext) = if is_dir { (name, "") } else { split_ext(name) };
    let base = strip_copy(stem);
    (1u32..)
        .map(|n| if n == 1 { alloc::format!("{base} (copy){ext}") } else { alloc::format!("{base} (copy {n}){ext}") })
        .find(|c| !taken.contains(&c.as_str()))
        .unwrap_or_default()
}

/// Renames `old` in folder `dir` to `new` and returns the new path.
pub fn rename(fs: &dyn Fs, dir: &str, old: &str, new: &str) -> Result<String, OpError> {
    validate_name(new)?;
    validate_name(old)?;
    let (from, to) = (join(dir, old), join(dir, new));
    if old == new {
        return Ok(to);
    }
    fs.lstat(&from)?;
    if fs.exists(&to) {
        // On a case-insensitive disk "a" → "A" finds "a" itself; allowed
        // unless the listing really has another entry spelled `new`.
        let case_only = old.to_lowercase() == new.to_lowercase();
        let really_there = fs.read_dir(dir)?.iter().any(|e| e.name == new);
        if !case_only || really_there {
            return Err(OpError::Fs(FsError::AlreadyExists));
        }
    }
    fs.rename(&from, &to)?;
    Ok(to)
}

/// Copy or cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipOp {
    Copy,
    Cut,
}

/// What was copied or cut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clipboard {
    pub op: ClipOp,
    pub paths: Vec<String>,
}

/// What a paste did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PasteReport {
    /// The new paths, in clipboard order (for a cut into the same folder,
    /// the unchanged path).
    pub done: Vec<String>,
    /// The sources that failed and why.
    pub failed: Vec<(String, FsError)>,
}

/// Pastes `clip` into folder `dest` (see the module docs). After a cut the
/// UI should empty the clipboard.
pub fn paste(fs: &dyn Fs, clip: &Clipboard, dest: &str) -> Result<PasteReport, OpError> {
    let dest = normalize(dest);
    if fs.stat(&dest)?.kind != Kind::Dir {
        return Err(OpError::Fs(FsError::NotADirectory));
    }
    for src in &clip.paths {
        let src = normalize(src);
        if fs.lstat(&src).is_ok_and(|m| m.kind == Kind::Dir) && is_inside(&dest, &src) {
            return Err(OpError::IntoItself(src));
        }
    }
    let mut report = PasteReport::default();
    for src in &clip.paths {
        let src = normalize(src);
        let result = match clip.op {
            ClipOp::Copy => paste_copy(fs, &src, &dest),
            ClipOp::Cut => paste_cut(fs, &src, &dest),
        };
        match result {
            Ok(p) => report.done.push(p),
            Err(e) => report.failed.push((src, e)),
        }
    }
    Ok(report)
}

fn names(fs: &dyn Fs, dir: &str) -> Result<Vec<String>, FsError> {
    Ok(fs.read_dir(dir)?.into_iter().map(|e| e.name).collect())
}

fn paste_copy(fs: &dyn Fs, src: &str, dest: &str) -> Result<String, FsError> {
    let meta = fs.stat(src)?;
    let name = file_name(src);
    if name.is_empty() {
        return Err(FsError::Invalid);
    }
    let taken = names(fs, dest)?;
    let name = copy_name(taken.iter().map(String::as_str), name, meta.kind == Kind::Dir);
    let to = join(dest, &name);
    if let Err(e) = copy_tree(fs, src, &to, 0) {
        // `to` was a free name, so whatever is there now is ours.
        let _ = delete_permanently(fs, &to);
        return Err(e);
    }
    Ok(to)
}

fn paste_cut(fs: &dyn Fs, src: &str, dest: &str) -> Result<String, FsError> {
    fs.lstat(src)?;
    if parent(src) == Some(dest) {
        return Ok(src.to_string());
    }
    let to = join(dest, file_name(src));
    if fs.exists(&to) {
        return Err(FsError::AlreadyExists);
    }
    match fs.rename(src, &to) {
        Err(FsError::CrossDevice) => {
            if let Err(e) = copy_tree(fs, src, &to, 0) {
                let _ = delete_permanently(fs, &to);
                return Err(e);
            }
            delete_permanently(fs, src)?;
            Ok(to)
        }
        Err(e) => Err(e),
        Ok(()) => Ok(to),
    }
}

/// Copies `from` (links followed) to the new name `to`.
pub fn copy_tree(fs: &dyn Fs, from: &str, to: &str, depth: usize) -> Result<(), FsError> {
    if depth > MAX_DEPTH {
        return Err(FsError::Other);
    }
    let meta = fs.stat(from)?;
    match meta.kind {
        Kind::Dir => {
            fs.create_dir(to)?;
            let _ = fs.set_mode(to, meta.mode);
            let mut children = fs.read_dir(from)?;
            children.sort_by(|a, b| a.name.cmp(&b.name));
            for c in children {
                copy_tree(fs, &join(from, &c.name), &join(to, &c.name), depth + 1)?;
            }
            Ok(())
        }
        Kind::File => {
            if fs.exists(to) {
                return Err(FsError::AlreadyExists);
            }
            fs.copy_file(from, to)
        }
        Kind::Symlink | Kind::Other => Err(FsError::Unsupported),
    }
}

/// Deletes `path` and, for a folder, everything in it. Links are removed,
/// not followed.
pub fn delete_permanently(fs: &dyn Fs, path: &str) -> Result<(), FsError> {
    let path = normalize(path);
    if path == "/" {
        return Err(FsError::Invalid);
    }
    let kind = fs.lstat(&path)?.kind;
    delete_kind(fs, &path, kind, 0)
}

fn delete_kind(fs: &dyn Fs, path: &str, kind: Kind, depth: usize) -> Result<(), FsError> {
    if kind != Kind::Dir {
        return fs.remove_file(path);
    }
    if depth > MAX_DEPTH {
        return Err(FsError::Other);
    }
    for c in fs.read_dir(path)? {
        delete_kind(fs, &join(path, &c.name), c.kind, depth + 1)?;
    }
    fs.remove_dir(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfs::{MemFs, StdFs};
    use alloc::vec;

    #[test]
    fn names_for_new_folders_and_copies() {
        assert_eq!(new_folder_name([]), "New Folder");
        assert_eq!(new_folder_name(["New Folder", "New Folder 2", "x"]), "New Folder 3");
        assert_eq!(new_folder_name(["New Folder 2"]), "New Folder");
        assert_eq!(copy_name(["a.txt"], "a.txt", false), "a (copy).txt");
        assert_eq!(copy_name(["a.txt", "a (copy).txt"], "a.txt", false), "a (copy 2).txt");
        assert_eq!(copy_name(["a.txt", "a (copy).txt"], "a (copy).txt", false), "a (copy 2).txt");
        assert_eq!(copy_name(["b.tar.gz"], "b.tar.gz", false), "b (copy).tar.gz");
        assert_eq!(copy_name(["my.dir"], "my.dir", true), "my.dir (copy)");
        assert_eq!(copy_name([".bashrc"], ".bashrc", false), ".bashrc (copy)");
        assert_eq!(copy_name(["free"], "x", false), "x");
        assert_eq!(strip_copy("x (copy 12)"), "x");
        assert_eq!(strip_copy("x (copy z)"), "x (copy z)");
    }

    #[test]
    fn validating() {
        assert_eq!(validate_name(""), Err(NameError::Empty));
        assert_eq!(validate_name(".."), Err(NameError::Dots));
        assert_eq!(validate_name("a/b"), Err(NameError::Slash));
        assert_eq!(validate_name("a\0"), Err(NameError::Nul));
        assert_eq!(validate_name(&"x".repeat(256)), Err(NameError::TooLong));
        assert_eq!(validate_name(&"x".repeat(255)), Ok(()));
        assert_eq!(validate_name("...hidden ü"), Ok(()));
    }

    #[test]
    fn new_folders_and_renames_on_disk() {
        let fs = StdFs::new();
        fs.create_dir("/d").unwrap();
        assert_eq!(new_folder(&fs, "/d").unwrap(), "New Folder");
        assert_eq!(new_folder(&fs, "/d").unwrap(), "New Folder 2");
        fs.write("/d/a.txt", b"a").unwrap();
        assert_eq!(rename(&fs, "/d", "New Folder", "a.txt"), Err(OpError::Fs(FsError::AlreadyExists)));
        assert_eq!(rename(&fs, "/d", "a.txt", "x/y"), Err(OpError::Name(NameError::Slash)));
        assert_eq!(rename(&fs, "/d", "a.txt", "b.txt").unwrap(), "/d/b.txt");
        assert_eq!(rename(&fs, "/d", "b.txt", "B.txt").unwrap(), "/d/B.txt", "case only");
        assert_eq!(rename(&fs, "/d", "nope", "z"), Err(OpError::Fs(FsError::NotFound)));
        assert_eq!(fs.read_to_string("/d/B.txt").unwrap(), "a");
    }

    #[test]
    fn case_only_rename_does_not_replace_another_file() {
        let fs = MemFs::new().with(&[("/d/a", "1"), ("/d/A", "2")]);
        assert_eq!(rename(&fs, "/d", "a", "A"), Err(OpError::Fs(FsError::AlreadyExists)));
    }

    fn tree(fs: &StdFs, dir: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![String::from(dir)];
        while let Some(d) = stack.pop() {
            for e in fs.read_dir(&d).unwrap() {
                let p = join(&d, &e.name);
                if e.kind == Kind::Dir {
                    stack.push(p.clone());
                    out.push(alloc::format!("{}/", &p[dir.len()..]));
                } else {
                    out.push(alloc::format!("{}={}", &p[dir.len()..], fs.read_to_string(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn copy_paste_of_a_nested_tree() {
        let fs = StdFs::new();
        for d in ["/src", "/src/p", "/src/p/q", "/src/p/q/r", "/dst"] {
            fs.create_dir(d).unwrap();
        }
        fs.write("/src/p/a.txt", b"A").unwrap();
        fs.write("/src/p/q/b", b"B").unwrap();
        fs.write("/src/p/q/r/c.md", b"C").unwrap();
        fs.write("/src/top.txt", b"T").unwrap();
        let clip = Clipboard { op: ClipOp::Copy, paths: vec!["/src/p".into(), "/src/top.txt".into()] };
        let r = paste(&fs, &clip, "/dst").unwrap();
        assert_eq!(r.done, ["/dst/p", "/dst/top.txt"]);
        assert!(r.failed.is_empty());
        assert_eq!(
            tree(&fs, "/dst"),
            ["/p/", "/p/a.txt=A", "/p/q/", "/p/q/b=B", "/p/q/r/", "/p/q/r/c.md=C", "/top.txt=T"]
        );
        // Again: copies get copy names.
        let r = paste(&fs, &clip, "/dst").unwrap();
        assert_eq!(r.done, ["/dst/p (copy)", "/dst/top (copy).txt"]);
        // Into the same folder: a duplicate.
        let r = paste(&fs, &Clipboard { op: ClipOp::Copy, paths: vec!["/src/top.txt".into()] }, "/src").unwrap();
        assert_eq!(r.done, ["/src/top (copy).txt"]);
        // The source is untouched.
        assert_eq!(fs.read_to_string("/src/p/q/r/c.md").unwrap(), "C");
    }

    #[test]
    fn paste_into_itself_is_refused() {
        let fs = StdFs::new();
        fs.create_dir("/a").unwrap();
        fs.create_dir("/a/b").unwrap();
        for op in [ClipOp::Copy, ClipOp::Cut] {
            let clip = Clipboard { op, paths: vec!["/a".into()] };
            assert_eq!(paste(&fs, &clip, "/a/b"), Err(OpError::IntoItself("/a".into())));
            assert_eq!(paste(&fs, &clip, "/a"), Err(OpError::IntoItself("/a".into())));
        }
        assert!(fs.exists("/a/b") && !fs.exists("/a/b/a"));
        let not_dir = Clipboard { op: ClipOp::Copy, paths: vec![] };
        fs.write("/f", b"").unwrap();
        assert_eq!(paste(&fs, &not_dir, "/f"), Err(OpError::Fs(FsError::NotADirectory)));
    }

    #[test]
    fn cut_paste() {
        let fs = StdFs::new();
        fs.create_dir("/a").unwrap();
        fs.create_dir("/b").unwrap();
        fs.create_dir("/a/sub").unwrap();
        fs.write("/a/sub/x", b"X").unwrap();
        fs.write("/a/f", b"F").unwrap();
        fs.write("/b/f", b"other").unwrap();
        let clip = Clipboard { op: ClipOp::Cut, paths: vec!["/a/sub".into(), "/a/f".into(), "/a/missing".into()] };
        let r = paste(&fs, &clip, "/b").unwrap();
        assert_eq!(r.done, ["/b/sub"]);
        assert_eq!(r.failed, [("/a/f".into(), FsError::AlreadyExists), ("/a/missing".into(), FsError::NotFound)]);
        assert_eq!(fs.read_to_string("/b/sub/x").unwrap(), "X");
        assert!(!fs.exists("/a/sub"));
        assert_eq!(fs.read_to_string("/b/f").unwrap(), "other", "never replaced");
        // A cut into the folder it is in does nothing.
        let r = paste(&fs, &Clipboard { op: ClipOp::Cut, paths: vec!["/a/f".into()] }, "/a").unwrap();
        assert_eq!(r.done, ["/a/f"]);
        assert_eq!(fs.read_to_string("/a/f").unwrap(), "F");
    }

    #[test]
    fn cut_across_devices_copies_then_deletes() {
        let fs = MemFs::new().with(&[("/home/u/dir/a", "A"), ("/home/u/dir/sub/b", "B"), ("/media/usb/", "")]);
        fs.add_device("/media/usb");
        fs.set_mode("/home/u/dir/a", 0o755).unwrap();
        let clip = Clipboard { op: ClipOp::Cut, paths: vec!["/home/u/dir".into()] };
        let r = paste(&fs, &clip, "/media/usb").unwrap();
        assert_eq!(r.done, ["/media/usb/dir"]);
        assert!(!fs.exists("/home/u/dir"));
        assert_eq!(fs.read_to_string("/media/usb/dir/sub/b").unwrap(), "B");
        assert_eq!(fs.stat("/media/usb/dir/a").unwrap().mode, 0o755, "modes carried over");
    }

    #[test]
    fn symlinks_are_copied_as_targets_and_deleted_as_links() {
        let fs = MemFs::new().with(&[("/t/real/f", "R"), ("/src/", ""), ("/dst/", "")]);
        fs.symlink("/src/link", "/t/real");
        fs.symlink("/src/loop", "/src");
        let clip = Clipboard { op: ClipOp::Copy, paths: vec!["/src/link".into(), "/src/loop".into()] };
        let r = paste(&fs, &clip, "/dst").unwrap();
        assert_eq!(r.done, ["/dst/link"]);
        assert_eq!(r.failed, [("/src/loop".into(), FsError::Other)], "a link loop ends at the depth limit");
        assert!(!fs.exists("/dst/loop"), "the partial copy is removed");
        assert_eq!(fs.read_to_string("/dst/link/f").unwrap(), "R");
        delete_permanently(&fs, "/src").unwrap();
        assert_eq!(fs.read_to_string("/t/real/f").unwrap(), "R", "the target survives");
        assert!(!fs.exists("/src"));
        assert_eq!(delete_permanently(&fs, "/"), Err(FsError::Invalid));
    }

    #[test]
    fn deleting_on_disk() {
        let fs = StdFs::new();
        fs.create_dir("/d").unwrap();
        fs.create_dir("/d/e").unwrap();
        fs.write("/d/e/f", b"").unwrap();
        delete_permanently(&fs, "/d").unwrap();
        assert!(!fs.exists("/d"));
        assert_eq!(delete_permanently(&fs, "/d"), Err(FsError::NotFound));
    }
}
