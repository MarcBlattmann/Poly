//! CFBundle's path logic, apart from the file system: where a bundle's
//! parts are, which file a resource lookup finds, and the numeric version
//! (`CFBundleGetVersionNumber`). Layouts from Apple's public "Bundle
//! Programming Guide"; behaviour from the CFBundle spec sheets.

use alloc::string::String;
use alloc::vec::Vec;

/// The kind of directory a bundle is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// `X.app/Contents/{Info.plist, MacOS, Resources}` (also frameworks'
    /// and plug-ins' modern layout).
    Contents,
    /// A plain directory: resources in the directory itself, no Info.plist
    /// unless one lies there.
    Flat,
}

/// The languages a lookup tries after the unlocalized place, in order.
pub const LANGUAGES: [&str; 3] = ["Base", "en", "English"];

/// Join path parts with `/`.
pub fn join(a: &str, b: &str) -> String {
    let mut s = String::from(a.trim_end_matches('/'));
    if !b.is_empty() {
        s.push('/');
        s.push_str(b.trim_start_matches('/'));
    }
    if s.is_empty() {
        s.push('/');
    }
    s
}

/// The directory part of a path (`/a/b/c` gives `/a/b`; `/a` gives `/`).
pub fn parent(path: &str) -> String {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(0) => String::from("/"),
        Some(i) => String::from(&p[..i]),
        None => String::from("."),
    }
}

/// The last component of a path.
pub fn last_component(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    p.rsplit('/').next().unwrap_or(p)
}

/// Make `path` absolute against `cwd` and remove `.` and `..` components
/// and doubled slashes.
pub fn normalize(cwd: &str, path: &str) -> String {
    let full = if path.starts_with('/') { String::from(path) } else { join(cwd, path) };
    let mut parts: Vec<&str> = Vec::new();
    for c in full.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    let mut out = String::new();
    for p in parts {
        out.push('/');
        out.push_str(p);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// The bundle directory that holds an executable at `exe` (absolute):
/// `X.app` for `X.app/Contents/MacOS/name`, else the executable's
/// directory.
pub fn main_bundle_dir(exe: &str) -> (String, Layout) {
    let macos = parent(exe);
    if last_component(&macos) == "MacOS" {
        let contents = parent(&macos);
        if last_component(&contents) == "Contents" {
            return (parent(&contents), Layout::Contents);
        }
    }
    (macos, Layout::Flat)
}

/// The Info.plist of a bundle directory.
pub fn info_plist(dir: &str, layout: Layout) -> String {
    match layout {
        Layout::Contents => join(dir, "Contents/Info.plist"),
        Layout::Flat => join(dir, "Info.plist"),
    }
}

/// The resources directory of a bundle directory.
pub fn resources_dir(dir: &str, layout: Layout) -> String {
    match layout {
        Layout::Contents => join(dir, "Contents/Resources"),
        Layout::Flat => String::from(dir.trim_end_matches('/')),
    }
}

/// The resources directory relative to the bundle directory, as a URL path
/// (`Contents/Resources/`, or `./` for a flat bundle).
pub fn resources_relative(layout: Layout) -> &'static str {
    match layout {
        Layout::Contents => "Contents/Resources/",
        Layout::Flat => "./",
    }
}

/// The executable of a bundle directory, given `CFBundleExecutable`.
pub fn executable(dir: &str, layout: Layout, name: &str) -> String {
    match layout {
        Layout::Contents => join(dir, &alloc::format!("Contents/MacOS/{name}")),
        Layout::Flat => join(dir, name),
    }
}

/// The file name a lookup of `name` with `ext` looks for; `None` when the
/// pair can name nothing (an empty extension).
pub fn resource_file(name: &str, ext: Option<&str>) -> Option<String> {
    match ext {
        None => Some(String::from(name)),
        Some("") => None,
        Some(e) => {
            let e = e.trim_start_matches('.');
            Some(alloc::format!("{name}.{e}"))
        }
    }
}

/// The places a resource lookup tries, in order: `resources[/sub]/file`,
/// then each language directory.
pub fn resource_candidates(resources: &str, sub: Option<&str>, file: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let with_sub = |d: String| match sub {
        Some(s) if !s.is_empty() => join(&d, s),
        _ => d,
    };
    dirs.push(with_sub(String::from(resources)));
    for lang in LANGUAGES {
        dirs.push(with_sub(join(resources, &alloc::format!("{lang}.lproj"))));
    }
    dirs.into_iter().map(|d| join(&d, file)).collect()
}

/// `CFBundleGetVersionNumber` of a version string `major[.minor[.bug]][stage[rev]]`
/// (Apple's documented `CFBundleVersion` form; stage `d`, `a`, `b` or `fc`):
/// major as two BCD digits, minor and bug-fix a nibble each, the stage
/// (development 0x20, alpha 0x40, beta 0x60, release 0x80) and the
/// pre-release revision. 0 when the string has another form.
pub fn version_number(v: &str) -> u32 {
    let b = v.trim().as_bytes();
    let mut i = 0;
    let num = |i: &mut usize, max_digits: usize| -> Option<u32> {
        let start = *i;
        let mut n = 0u32;
        while *i < b.len() && b[*i].is_ascii_digit() && *i - start < max_digits {
            n = n * 10 + u32::from(b[*i] - b'0');
            *i += 1;
        }
        (*i > start).then_some(n)
    };
    let Some(major) = num(&mut i, 2) else { return 0 };
    let (mut minor, mut bug) = (0, 0);
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let Some(m) = num(&mut i, 1) else { return 0 };
        minor = m;
        if i < b.len() && b[i] == b'.' {
            i += 1;
            let Some(x) = num(&mut i, 1) else { return 0 };
            bug = x;
        }
    }
    let (mut stage, mut rev) = (0x80u32, 0u32);
    if i < b.len() {
        let (s, len) = match b[i] {
            b'd' => (0x20, 1),
            b'a' => (0x40, 1),
            b'b' => (0x60, 1),
            b'f' if b.get(i + 1) == Some(&b'c') => (0x80, 2),
            _ => return 0,
        };
        stage = s;
        i += len;
        match num(&mut i, 3) {
            Some(r) if r <= 255 => rev = r,
            Some(_) => return 0,
            None => {}
        }
        if stage == 0x80 && rev == 0 {
            return 0;
        }
    }
    if i != b.len() {
        return 0;
    }
    let bcd = ((major / 10) << 4) | (major % 10);
    (bcd << 24) | (minor << 20) | (bug << 16) | (stage << 8) | rev
}

/// A four-character code as a number (`'APPL'` is 0x4150504c); `None` when
/// the text is not four bytes.
pub fn four_cc(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    (b.len() == 4).then(|| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(main_bundle_dir("/x/T.app/Contents/MacOS/t"), ("/x/T.app".into(), Layout::Contents));
        assert_eq!(main_bundle_dir("/bin/prog"), ("/bin".into(), Layout::Flat));
        assert_eq!(main_bundle_dir("/prog"), ("/".into(), Layout::Flat));
        assert_eq!(normalize("/home/u", "T.app/./Contents/../Contents/"), "/home/u/T.app/Contents");
        assert_eq!(normalize("/", "/"), "/");
        assert_eq!(join("/a/", "b"), "/a/b");
        assert_eq!(parent("/a/b/"), "/a");
        assert_eq!(last_component("/a/b.txt"), "b.txt");
        assert_eq!(resources_dir("/x/T.app", Layout::Contents), "/x/T.app/Contents/Resources");
        assert_eq!(executable("/x/T.app", Layout::Contents, "t"), "/x/T.app/Contents/MacOS/t");
    }

    #[test]
    fn resources() {
        assert_eq!(resource_file("hello", Some("txt")).as_deref(), Some("hello.txt"));
        assert_eq!(resource_file("hello.txt", None).as_deref(), Some("hello.txt"));
        assert_eq!(resource_file("hello", Some("")), None);
        let c = resource_candidates("/r", Some("sub"), "deep.txt");
        assert_eq!(c[0], "/r/sub/deep.txt");
        assert_eq!(c[2], "/r/en.lproj/sub/deep.txt");
        assert_eq!(resource_candidates("/r", None, "x")[0], "/r/x");
    }

    #[test]
    fn versions() {
        assert_eq!(version_number("42"), 1_107_329_024);
        assert_eq!(version_number("1.2.3"), 0x0123_8000);
        assert_eq!(version_number("2.1b4"), 0x0210_6004);
        assert_eq!(version_number("1.0d1"), 0x0100_2001);
        assert_eq!(version_number("1.2.3.4"), 0);
        assert_eq!(version_number("abc"), 0);
        assert_eq!(version_number(""), 0);
        assert_eq!(four_cc("APPL"), Some(0x4150_504c));
        assert_eq!(four_cc("????"), Some(0x3f3f_3f3f));
    }
}
