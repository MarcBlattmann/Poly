//! A process's font table (ADR 0142, extended): the fonts a program adds
//! to itself with `AddFontMemResourceEx` and `AddFontResourceExW`, found by
//! family or full name when a `LOGFONTW` is mapped, and the families Poly
//! lists under their Windows names for `EnumFontFamiliesExW`
//! (`specs/windows/gdi32.dll/AddFontMemResourceEx.yaml`,
//! `AddFontResourceExW.yaml`, `RemoveFontResourceExW.yaml`,
//! `EnumFontFamiliesExW.yaml`, `CreateFontIndirectW.yaml`).

use crate::fontmap::Class;
use crate::outline::FaceStyle;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_font::{Font, OutlineFormat, name_id};

/// `AddFontResourceExW`'s flags (`wingdi.h`).
pub const FR_PRIVATE: u32 = 0x10;
pub const FR_NOT_ENUM: u32 = 0x20;

/// How a face got into the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// `AddFontMemResourceEx`: the handle it returned.
    Memory(u32),
    /// `AddFontResourceExW`: the file (its [`file_key`]) and the flags.
    File { path: Vec<u16>, flags: u32 },
}

/// A face the program added.
#[derive(Clone, Debug)]
pub struct Face {
    pub id: u32,
    pub data: Arc<Vec<u8>>,
    pub family: Vec<u16>,
    pub full: Vec<u16>,
    pub style_name: Vec<u16>,
    pub style: FaceStyle,
    pub fixed: bool,
    pub owner: Owner,
    pub enumerable: bool,
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `a` and `b` are the same name, regardless of letter case.
pub fn same_name(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| fold(x) == fold(y))
}

fn fold(u: u16) -> u16 {
    match char::from_u32(u32::from(u)).and_then(|c| c.to_lowercase().next()) {
        Some(l) if (l as u32) <= 0xFFFF => l as u16,
        _ => u,
    }
}

const SLASH: u16 = b'/' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const DOT: u16 = b'.' as u16;
const BLANK: u16 = b' ' as u16;

/// The `\\?\` prefix of a path, if it has one.
fn long_prefix(path: &[u16]) -> usize {
    let p: [u16; 4] = [BACKSLASH, BACKSLASH, b'?' as u16, BACKSLASH];
    if path.len() >= 4 && path[..4] == p { 4 } else { 0 }
}

/// What a font file is known by (`RemoveFontResourceExW.yaml`, spelling):
/// the file its name leads to, not the spelling of the name. `full_path`
/// is the name as a full path; the key is the same for another letter
/// case, forward slashes, a way through a directory and back with `..`
/// (the directory need not exist), the `\\?\` prefix and a dot after the
/// name.
pub fn file_key(full_path: &[u16]) -> Vec<u16> {
    let path = &full_path[long_prefix(full_path)..];
    let mut parts: Vec<&[u16]> = Vec::new();
    for (i, part) in path.split(|&u| u == BACKSLASH || u == SLASH).enumerate() {
        if i > 0 && part == [DOT, DOT] {
            // Never above the first part (the drive, or the root).
            if parts.len() > 1 {
                parts.pop();
            }
        } else if i > 0 && part == [DOT] {
            // The directory itself.
        } else {
            parts.push(part);
        }
    }
    // Dots and blanks after the name are not part of it.
    if let Some(last) = parts.last_mut() {
        let keep = last.iter().rposition(|&u| u != DOT && u != BLANK).map_or(0, |i| i + 1);
        if keep > 0 {
            *last = &last[..keep];
        }
    }
    let mut key = Vec::with_capacity(path.len());
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            key.push(BACKSLASH);
        }
        key.extend(part.iter().map(|&u| fold(u)));
    }
    key
}

/// What `RemoveFontResourceExW` makes of a name and flags before it looks
/// for the file (`RemoveFontResourceExW.yaml`, errors).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameCheck {
    /// FALSE with this error code.
    Error(u32),
    /// FALSE without an error code.
    Refused,
    /// The name may lead to a file: look it up.
    Look,
}

/// `ERROR_FILE_NOT_FOUND` and `ERROR_INVALID_PARAMETER` (`winerror.h`).
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
/// `MAX_PATH` (`minwindef.h`).
const MAX_PATH: usize = 260;

/// The checks on `RemoveFontResourceExW`'s name (without its NUL) and
/// flags: flag bits other than `FR_PRIVATE` and `FR_NOT_ENUM`, an empty
/// name and one of blanks only are error 87. A name with a wildcard is
/// neither expanded nor refused as a bad name: it is a file that is not
/// there (error 2), as is a name with a backslash after it. A name longer
/// than `MAX_PATH` or two names joined with `|` fail without an error
/// code.
pub fn remove_name_check(name: &[u16], flags: u32) -> NameCheck {
    if flags & !(FR_PRIVATE | FR_NOT_ENUM) != 0 || name.iter().all(|&u| u == BLANK) {
        return NameCheck::Error(ERROR_INVALID_PARAMETER);
    }
    if name.len() > MAX_PATH || name.contains(&(b'|' as u16)) {
        return NameCheck::Refused;
    }
    let rest = &name[long_prefix(name)..];
    let wildcard = rest.iter().any(|&u| u == b'*' as u16 || u == b'?' as u16);
    let ends_in_separator = rest.last().is_some_and(|&u| u == BACKSLASH || u == SLASH);
    if wildcard || ends_in_separator {
        return NameCheck::Error(ERROR_FILE_NOT_FOUND);
    }
    NameCheck::Look
}

/// What the table reads from a font file.
struct FaceInfo {
    family: Vec<u16>,
    full: Vec<u16>,
    style_name: Vec<u16>,
    style: FaceStyle,
    fixed: bool,
}

/// Read what the table needs from a font file; `None` if it is not a
/// usable TrueType (or OpenType) font: not complete, no outlines, or
/// without a family name.
fn read_face(data: &[u8]) -> Option<FaceInfo> {
    if !Font::directory_complete(data) {
        return None;
    }
    let font = Font::parse(data).ok()?;
    if font.outline_format() == OutlineFormat::None {
        return None;
    }
    let family = font.name(name_id::FAMILY).filter(|s| !s.is_empty())?;
    let style_name = font.name(name_id::SUBFAMILY).unwrap_or_else(|| alloc::string::String::from("Regular"));
    let full = font
        .name(name_id::FULL_NAME)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| alloc::format!("{family} {style_name}"));
    let st = font.style();
    let gm = font.gdi_metrics();
    let style = FaceStyle {
        bold: st.bold || st.weight_class >= 600,
        italic: st.italic || st.oblique,
        weight: i32::from(st.weight_class.max(1)),
    };
    let fixed = gm.panose_proportion == 9 || st.monospace;
    Some(FaceInfo { family: utf16(&family), full: utf16(&full), style_name: utf16(&style_name), style, fixed })
}

/// The face a name stands for and how it was found.
#[derive(Clone, Copy, Debug)]
pub struct Found<'a> {
    pub face: &'a Face,
    /// The request named this face (its full name, "PolyTest Sans Bold"),
    /// not the family.
    pub by_full_name: bool,
}

/// One face of an enumeration.
#[derive(Clone, Debug)]
pub struct EnumFace {
    pub family: Vec<u16>,
    pub full: Vec<u16>,
    pub style_name: Vec<u16>,
    pub weight: i32,
    pub italic: bool,
    pub fixed: bool,
    /// The class of a family Poly stands in for under a Windows name; a
    /// face the program added has none.
    pub class: Option<Class>,
}

impl EnumFace {
    /// `lfPitchAndFamily` of the face: `VARIABLE_PITCH` or `FIXED_PITCH`
    /// with the family.
    pub fn pitch_and_family(&self) -> u8 {
        match self.class {
            Some(Class::Sans) => 0x22,
            Some(Class::Serif) => 0x12,
            Some(Class::Mono) => 0x31,
            None if self.fixed => 0x31,
            None => 0x02,
        }
    }
}

/// The families Poly lists as installed, under their Windows names; each
/// has Regular, Bold, Italic and Bold Italic faces (DejaVu underneath).
pub const INSTALLED: &[(&str, Class)] = &[
    ("Arial", Class::Sans),
    ("Segoe UI", Class::Sans),
    ("Tahoma", Class::Sans),
    ("Verdana", Class::Sans),
    ("Calibri", Class::Sans),
    ("Candara", Class::Sans),
    ("Corbel", Class::Sans),
    ("Trebuchet MS", Class::Sans),
    ("Microsoft Sans Serif", Class::Sans),
    ("Century Gothic", Class::Sans),
    ("Times New Roman", Class::Serif),
    ("Georgia", Class::Serif),
    ("Cambria", Class::Serif),
    ("Constantia", Class::Serif),
    ("Palatino Linotype", Class::Serif),
    ("Book Antiqua", Class::Serif),
    ("Garamond", Class::Serif),
    ("Courier New", Class::Mono),
    ("Consolas", Class::Mono),
    ("Lucida Console", Class::Mono),
];

/// Other names the font mapper knows, with their Windows spelling (the
/// raster fonts and the dialog font aliases).
const OTHER_NAMES: &[&str] = &[
    "MS Shell Dlg",
    "MS Shell Dlg 2",
    "MS Sans Serif",
    "MS Serif",
    "System",
    "Terminal",
    "Fixedsys",
    "Courier",
    "Small Fonts",
    "Times",
    "Helvetica",
    "MS UI Gothic",
    "MS Gothic",
    "MS Mincho",
];

/// The Windows spelling of a name Poly knows ("arial" is "Arial").
pub fn canonical_name(name: &[u16]) -> Option<&'static str> {
    INSTALLED.iter().map(|&(n, _)| n).chain(OTHER_NAMES.iter().copied()).find(|n| same_name(&utf16(n), name))
}

/// The faces a family has: Regular, Bold, Italic, Bold Italic.
const STYLES: [(&str, i32, bool); 4] =
    [("Regular", 400, false), ("Bold", 700, false), ("Italic", 400, true), ("Bold Italic", 700, true)];

/// The fonts added to a process.
#[derive(Debug, Default)]
pub struct FontTable {
    faces: Vec<Face>,
    next_id: u32,
    next_handle: u32,
    /// Counts every change: a logical font mapped before it changed is
    /// mapped again.
    pub generation: u32,
}

/// The handle `AddFontMemResourceEx` returns for the first font.
const FIRST_HANDLE: u32 = 0x0A00_0001;

impl FontTable {
    pub fn new() -> FontTable {
        FontTable { faces: Vec::new(), next_id: 1, next_handle: FIRST_HANDLE, generation: 0 }
    }

    fn push(&mut self, data: Vec<u8>, owner: Owner, enumerable: bool) -> bool {
        let Some(FaceInfo { family, full, style_name, style, fixed }) = read_face(&data) else { return false };
        let id = self.next_id;
        self.next_id += 1;
        self.faces.push(Face { id, data: Arc::new(data), family, full, style_name, style, fixed, owner, enumerable });
        self.generation += 1;
        true
    }

    /// `AddFontMemResourceEx`: the handle, or `None` for data that is no
    /// font. The data is copied.
    pub fn add_memory(&mut self, data: &[u8]) -> Option<u32> {
        let handle = self.next_handle;
        if !self.push(data.to_vec(), Owner::Memory(handle), false) {
            return None;
        }
        self.next_handle += 0x10;
        Some(handle)
    }

    /// `RemoveFontMemResourceEx`: false for a handle that is not (or no
    /// longer) one.
    pub fn remove_memory(&mut self, handle: u32) -> bool {
        let Some(i) = self.faces.iter().position(|f| f.owner == Owner::Memory(handle)) else { return false };
        self.faces.remove(i);
        self.generation += 1;
        true
    }

    /// `AddFontResourceExW` with the file's bytes: whether it was a font.
    /// `path` is the file's full path, in any spelling.
    pub fn add_file(&mut self, path: &[u16], flags: u32, data: Vec<u8>) -> bool {
        let enumerable = flags & FR_NOT_ENUM == 0;
        self.push(data, Owner::File { path: file_key(path), flags }, enumerable)
    }

    /// `RemoveFontResourceExW`: one add of that file (its full path, in any
    /// spelling) with exactly those flags. Adds of one file with different
    /// flags are counted apart, adds under different spellings together;
    /// the family stays until the last add is gone.
    pub fn remove_file(&mut self, path: &[u16], flags: u32) -> bool {
        let key = file_key(path);
        let Some(i) = self.faces.iter().position(|f| match &f.owner {
            Owner::File { path: p, flags: fl } => *fl == flags && *p == key,
            Owner::Memory(_) => false,
        }) else {
            return false;
        };
        self.faces.remove(i);
        self.generation += 1;
        true
    }

    /// Number of faces in the table.
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    pub fn get(&self, id: u32) -> Option<&Face> {
        self.faces.iter().find(|f| f.id == id)
    }

    /// The face for a family or full name and the style asked for: of a
    /// family's faces the one that matches the bold and italic request best
    /// (bold first); a request for a face's full name gets that face.
    pub fn find(&self, name: &[u16], bold: bool, italic: bool) -> Option<Found<'_>> {
        let family: Vec<&Face> = self.faces.iter().filter(|f| same_name(&f.family, name)).collect();
        if !family.is_empty() {
            let score = |f: &&Face| {
                let mut s = 0;
                if f.style.bold == bold {
                    s += 4;
                }
                if f.style.italic == italic {
                    s += 2;
                }
                s
            };
            // The first best one: faces added earlier win a tie.
            let mut best = family[0];
            for f in &family[1..] {
                if score(f) > score(&best) {
                    best = f;
                }
            }
            return Some(Found { face: best, by_full_name: false });
        }
        self.faces.iter().find(|f| same_name(&f.full, name)).map(|face| Found { face, by_full_name: true })
    }

    /// The faces `EnumFontFamiliesExW` lists for a face name (empty: all) and
    /// a character set: the faces the program added (not those from memory,
    /// not those added with `FR_NOT_ENUM`) and Poly's installed families. A
    /// character set other than ANSI (0) and DEFAULT (1) lists nothing:
    /// the fonts are Western.
    pub fn enumerate(&self, name: &[u16], charset: u8) -> Vec<EnumFace> {
        let mut out = Vec::new();
        if charset > 1 {
            return out;
        }
        let wanted = |family: &[u16]| name.is_empty() || same_name(family, name);
        for f in self.faces.iter().filter(|f| f.enumerable && matches!(f.owner, Owner::File { .. })) {
            if wanted(&f.family) {
                out.push(EnumFace {
                    family: f.family.clone(),
                    full: f.full.clone(),
                    style_name: f.style_name.clone(),
                    weight: if f.style.bold { f.style.weight.max(700) } else { f.style.weight.min(599) },
                    italic: f.style.italic,
                    fixed: f.fixed,
                    class: None,
                });
            }
        }
        for &(family, class) in INSTALLED {
            let fam = utf16(family);
            if !wanted(&fam) {
                continue;
            }
            for (style, weight, italic) in STYLES {
                let full = if style == "Regular" { family.into() } else { alloc::format!("{family} {style}") };
                out.push(EnumFace {
                    family: fam.clone(),
                    full: utf16(&full),
                    style_name: utf16(style),
                    weight,
                    italic,
                    fixed: class == Class::Mono,
                    class: Some(class),
                });
            }
        }
        out
    }
}
