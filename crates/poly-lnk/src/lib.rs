//! Windows shell links: the `.lnk` files of the Desktop and the Start menu.
//!
//! This crate is the logic of `shell32.dll`'s `ShellLink` object (`CLSID_ShellLink`), which
//! installers create to write their shortcuts, and the reader Poly's launcher uses to import the
//! shortcuts of `C:\ProgramData\Microsoft\Windows\Start Menu\Programs`. The DLL keeps only the COM
//! tables and the calls into the system ([`System`]).
//!
//! - [`Link`]: the data of one link, what `IShellLinkW`, `IPersistFile` and `IPropertyStore` read
//!   and change, with the sheets' rules: [`Link::set_path`] (canonical text, environment
//!   variables, the find data, the 259-character limit), [`Link::resolve`], the property store
//!   ([`props`]).
//! - [`Link::save`]: the bytes `IPersistFile::Save` writes (MS-SHLLINK): the header, the target's
//!   ID list ([`idlist`]), a `LinkInfo` for a target on a local disk, the string data, the extra
//!   data blocks and the serialized property store (MS-PROPSTORE, MS-OLEPS).
//! - [`parse`]: any `.lnk` back into a [`Link`]; `Link::target`, `icon`, `show_command`,
//!   `app_user_model_id` and `toast_activator_clsid` answer what a program that starts the
//!   shortcut needs, in the order the sheet's `reading_a_link` gives.
//! - [`text`]: the text rules (environment variables, relative paths, the buffer rules of the
//!   getters).
//!
//! Sources (clean room): the spec sheets `specs/windows/shell32.dll/ShellLink.yaml`,
//! `IShellLinkW.yaml`, `ShellLink.IPersistFile.yaml`, `ShellLink.PropertyStore.yaml` and their
//! visible tests; Microsoft's open specifications [MS-SHLLINK] (the file), [MS-PROPSTORE] (the
//! property store block) and [MS-OLEPS] (typed property values); Learn: `IShellLinkW`,
//! `IPersistFile`, `IPropertyStore`, `WIN32_FIND_DATAW`; the constants are `shobjidl_core.h`'s,
//! `propkey.h`'s, `KnownFolders.h`'s, `shlobj_core.h`'s and `winerror.h`'s. The layout of the shell's
//! file-entry items is not specified anywhere public: the one here is Poly's own, made to the
//! sheet's description (the name as ANSI text at offset 14, an extension block with the signature
//! 0xBEEF0004 holding the long name in UTF-16, the item sizes the sheet lists).

#![no_std]

extern crate alloc;

pub mod idlist;
pub mod props;
mod read;
pub mod text;
mod write;

use alloc::vec::Vec;
pub use poly_com::guid::Guid;
pub use props::{PropKey, PropValue, Store};
pub use read::{Error, parse};

/// Text in UTF-16 code units, without a NUL.
pub type Text = Vec<u16>;

/// `S_OK`, `S_FALSE` and the failures the link's methods return (`winerror.h`).
pub mod hr {
    pub const S_OK: i32 = 0;
    pub const S_FALSE: i32 = 1;
    pub const E_FAIL: i32 = 0x8000_4005_u32 as i32;
    pub const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
    /// `HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)`.
    pub const E_INSUFFICIENT_BUFFER: i32 = 0x8007_007A_u32 as i32;
}

/// `CLSID_ShellLink` (`shobjidl_core.h`): {00021401-0000-0000-C000-000000000046}, also the
/// `LinkCLSID` of every link file (MS-SHLLINK 2.1).
pub const CLSID_SHELL_LINK: Guid =
    Guid { data1: 0x0002_1401, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// `FILE_ATTRIBUTE_DIRECTORY` (`winnt.h`).
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// `SW_SHOWNORMAL`, `SW_SHOWMAXIMIZED`, `SW_SHOWMINNOACTIVE` (`winuser.h`): the show commands a
/// link file keeps (MS-SHLLINK 2.1; the sheet's `header` and `load_built`).
pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOWMAXIMIZED: i32 = 3;
pub const SW_SHOWMINNOACTIVE: i32 = 7;

/// What the system knows about a file: the parts of `WIN32_FIND_DATAW` (`minwinbase.h`) a link
/// keeps (in its header) and reports (`GetPath`'s find data).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileInfo {
    pub attributes: u32,
    /// `FILETIME`s as 64-bit numbers.
    pub creation: u64,
    pub access: u64,
    pub write: u64,
    pub size: u64,
}

impl FileInfo {
    pub fn is_dir(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }
}

/// What the link needs from the system it runs on. `shell32` answers with Win32 calls; the tests
/// with a table.
pub trait System {
    /// The value of an environment variable (the name without `%`, any case).
    fn env(&self, name: &[u16]) -> Option<Text>;
    /// An existing file or directory: its last component as the file system spells it and its
    /// data; `None` when nothing is there.
    fn entry(&self, path: &[u16]) -> Option<(Text, FileInfo)>;
    /// Whether the drive `X:` exists.
    fn drive_exists(&self, letter: u16) -> bool;
    /// The serial number of the volume of drive `X:`.
    fn volume_serial(&self, letter: u16) -> u32;
    /// The computer's NetBIOS name (ANSI).
    fn machine_name(&self) -> Vec<u8>;
    /// The string form of the user's SID (`S-1-5-21-...`).
    fn user_sid(&self) -> Text;
}

/// What `SetPath` did with its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetPath {
    /// `S_OK`.
    Done,
    /// `E_INVALIDARG`: a quoted path followed by more text; the link has no path any more.
    InvalidArg,
    /// `E_FAIL`: 260 characters or more; the link has no path any more.
    TooLong,
}

impl SetPath {
    pub fn hresult(self) -> i32 {
        match self {
            SetPath::Done => hr::S_OK,
            SetPath::InvalidArg => hr::E_INVALIDARG,
            SetPath::TooLong => hr::E_FAIL,
        }
    }
}

/// The longest path `SetPath` keeps (`MAX_PATH` - 1, the sheet's `path_length`).
pub const MAX_LINK_PATH: usize = 259;

/// One shell link: what `IShellLinkW` sets and gets, what a `.lnk` file holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// The target as `GetPath` reports it (expanded, canonical); `None`: no path.
    pub path: Option<Text>,
    /// The target as it was given with environment variables (`SetPath` of `%SystemRoot%\...`, the
    /// file's `EnvironmentVariableDataBlock`): what `GetPath(SLGP_RAWPATH)` reports.
    pub env_path: Option<Text>,
    /// The target's data: `GetPath`'s find data, the file header's attributes, times and size.
    pub find: FileInfo,
    /// The `LocalBasePath` of a loaded file's `LinkInfo` (never reported by `GetPath`; the
    /// launcher's first choice for the target).
    pub link_info_path: Option<Text>,
    /// A loaded file's `RELATIVE_PATH` (relative to the link file's directory).
    pub relative_path: Option<Text>,
    /// `SetRelativePath`'s path: the file the relative path is made from (else the file saved).
    pub relative_source: Option<Text>,
    pub description: Option<Text>,
    pub working_dir: Option<Text>,
    pub arguments: Option<Text>,
    pub icon_path: Option<Text>,
    /// A loaded file's `IconEnvironmentDataBlock` text (with its variables): it wins over
    /// `icon_path` ([`Link::icon`]).
    pub icon_env: Option<Text>,
    pub icon_index: i32,
    /// What `SetShowCmd` stored (any value); a file keeps 1, 3 or 7.
    pub show_command: i32,
    pub hotkey: u16,
    /// The property store (`IPropertyStore`, the file's `PropertyStoreDataBlock`).
    pub properties: Store,
}

impl Default for Link {
    fn default() -> Link {
        Link::new()
    }
}

impl Link {
    /// A new link: no path, no text, show command 1, hot key 0, icon index 0 (the sheet's
    /// `fresh_link`).
    pub const fn new() -> Link {
        Link {
            path: None,
            env_path: None,
            find: FileInfo { attributes: 0, creation: 0, access: 0, write: 0, size: 0 },
            link_info_path: None,
            relative_path: None,
            relative_source: None,
            description: None,
            working_dir: None,
            arguments: None,
            icon_path: None,
            icon_env: None,
            icon_index: 0,
            show_command: SW_SHOWNORMAL,
            hotkey: 0,
            properties: Store::new(),
        }
    }

    /// `SetPath(text)` (the sheet's `path_*` rules): surrounding quotes go (a quoted path followed
    /// by more text is refused), `/` becomes `\`, `.` and `..` components collapse; text with
    /// `%VAR%` is kept for `SLGP_RAWPATH` and expanded for the path; an existing target takes the
    /// file system's letter case and gives the find data (not when the text had to be made
    /// canonical); 260 characters or more are refused. The empty text removes the path. An
    /// existing target adds `System.VolumeId` to the store.
    pub fn set_path(&mut self, given: &[u16], sys: &dyn System) -> SetPath {
        self.path = None;
        self.env_path = None;
        self.find = FileInfo::default();
        let Some((canonical, changed)) = text::canonical_target(given) else { return SetPath::InvalidArg };
        if canonical.is_empty() {
            return SetPath::Done;
        }
        if canonical.len() > MAX_LINK_PATH {
            return SetPath::TooLong;
        }
        let has_env = canonical.contains(&u16::from(b'%'));
        let expanded = if has_env { text::expand_env(&canonical, |n| sys.env(n)) } else { canonical.clone() };
        if expanded.len() > MAX_LINK_PATH {
            return SetPath::TooLong;
        }
        let mut path = expanded;
        if !changed && let Some((spelled, info)) = text::spell(&path, sys) {
            path = spelled;
            self.find = info;
            if let Some(letter) = text::drive_letter(&path) {
                self.properties.set(&props::PKEY_VOLUME_ID, PropValue::Clsid(volume_guid(sys.volume_serial(letter))));
            }
        }
        if has_env {
            self.env_path = Some(canonical);
        }
        self.path = Some(path);
        SetPath::Done
    }

    /// `SetIDList(list)`: the target of an item list ([`idlist::path_of`]); `None` (or a list
    /// without a path) removes the path.
    pub fn set_id_list_path(&mut self, path: Option<Text>, sys: &dyn System) {
        self.env_path = None;
        self.path = None;
        self.find = FileInfo::default();
        if let Some(p) = path.filter(|p| !p.is_empty()) {
            if let Some((spelled, info)) = text::spell(&p, sys) {
                self.find = info;
                self.path = Some(spelled);
            } else {
                self.path = Some(p);
            }
        }
    }

    /// The text `GetPath` reports: `SLGP_RAWPATH` (`raw`) gives the text with its variables when
    /// there is one; the other flags the path. `None`: `S_FALSE` and an empty string.
    pub fn reported_path(&self, raw: bool) -> Option<&[u16]> {
        if raw && let Some(e) = &self.env_path {
            return Some(e);
        }
        self.path.as_deref()
    }

    /// `Resolve(hwnd, SLR_NO_UI | ...)`: a link without a target is `S_OK`; a target given only
    /// with variables gets its expanded path; an existing target is `S_OK`, a missing one
    /// `S_FALSE` (the path stays). Nothing is searched.
    pub fn resolve(&mut self, sys: &dyn System) -> i32 {
        if self.path.is_none()
            && let Some(e) = &self.env_path
        {
            let expanded = text::expand_env(e, |n| sys.env(n));
            match text::spell(&expanded, sys) {
                Some((spelled, info)) => {
                    self.path = Some(spelled);
                    self.find = info;
                }
                None => self.path = Some(expanded),
            }
        }
        match &self.path {
            None => hr::S_OK,
            Some(p) if sys.entry(p).is_some() => hr::S_OK,
            Some(_) => hr::S_FALSE,
        }
    }

    /// The bytes `IPersistFile::Save(file)` writes (`file`: the full path of the link file,
    /// `None` for a stream). Nothing of the object changes.
    pub fn save(&self, file: Option<&[u16]>, sys: &dyn System) -> Vec<u8> {
        write::save(self, file, sys)
    }

    /// `IPersistFile::Load`: the link a file's bytes hold, as the object reports it: the icon of
    /// an `IconEnvironmentDataBlock` expanded. The caller has checked that the file opened.
    pub fn load(data: &[u8], sys: &dyn System) -> Result<Link, Error> {
        let mut link = parse(data)?;
        if let Some(e) = link.icon_env.take() {
            link.icon_path = Some(text::expand_env(&e, |n| sys.env(n)));
        }
        Ok(link)
    }

    // ----- what a launcher reads (the sheet's `reading_a_link`) ---------------------------------

    /// The target: the `LinkInfo` local base path when there is one, else the ID list's path,
    /// else the `EnvironmentVariableDataBlock` text (possibly with `%VAR%`: expand it with
    /// [`text::expand_env`]). The relative path alone is not enough (`None`).
    pub fn target(&self) -> Option<&[u16]> {
        self.link_info_path.as_deref().or(self.path.as_deref()).or(self.env_path.as_deref())
    }

    /// The command-line arguments (empty when none).
    pub fn arguments(&self) -> &[u16] {
        self.arguments.as_deref().unwrap_or(&[])
    }

    /// The working directory (empty when none; may hold `%VAR%`).
    pub fn working_dir(&self) -> &[u16] {
        self.working_dir.as_deref().unwrap_or(&[])
    }

    /// The description (the comment; empty when none).
    pub fn description(&self) -> &[u16] {
        self.description.as_deref().unwrap_or(&[])
    }

    /// The icon file and index: the `IconEnvironmentDataBlock`'s path when there is one, else the
    /// `ICON_LOCATION` string (either may hold `%VAR%`); `None` when the link names no icon file.
    pub fn icon(&self) -> Option<(&[u16], i32)> {
        self.icon_env.as_deref().or(self.icon_path.as_deref()).filter(|p| !p.is_empty()).map(|p| (p, self.icon_index))
    }

    /// The show command a started program gets: 1 (normal), 3 (maximized) or 7 (minimized);
    /// anything else is 1.
    pub fn show_command(&self) -> i32 {
        file_show_command(self.show_command)
    }

    /// `System.AppUserModel.ID`, when the store has it as text.
    pub fn app_user_model_id(&self) -> Option<&[u16]> {
        match self.properties.get(&props::PKEY_APP_USER_MODEL_ID) {
            Some(PropValue::Text(t)) => Some(t),
            _ => None,
        }
    }

    /// `System.AppUserModel.ToastActivatorCLSID`, when the store has it as a CLSID.
    pub fn toast_activator_clsid(&self) -> Option<Guid> {
        match self.properties.get(&props::PKEY_TOAST_ACTIVATOR_CLSID) {
            Some(PropValue::Clsid(g)) => Some(*g),
            _ => None,
        }
    }

    // ----- the property store as IPropertyStore shows it -----------------------------------------

    /// `IPropertyStore::GetCount`: the stored entries and, with a description, `System.Comment`.
    pub fn property_count(&self) -> usize {
        self.properties.len() + usize::from(self.comment().is_some())
    }

    /// `IPropertyStore::GetAt`.
    pub fn property_key_at(&self, index: usize) -> Option<PropKey> {
        match index.checked_sub(self.properties.len()) {
            None => self.properties.key_at(index),
            Some(0) if self.comment().is_some() => Some(props::PKEY_COMMENT),
            Some(_) => None,
        }
    }

    /// `IPropertyStore::GetValue`: `None` is `VT_EMPTY`.
    pub fn property(&self, key: &PropKey) -> Option<PropValue> {
        if *key == props::PKEY_COMMENT {
            return self.comment().map(|c| PropValue::Text(c.to_vec()));
        }
        self.properties.get(key).cloned()
    }

    /// `IPropertyStore::SetValue` (the sheet's `set_get`, `empty`, `toast`, `any_key`): any key
    /// takes any value; `System.Comment` is the description; the text form of a CLSID set into
    /// `ToastActivatorCLSID` is stored as the CLSID and answered with `S_FALSE`.
    pub fn set_property(&mut self, key: &PropKey, value: PropValue) -> i32 {
        if *key == props::PKEY_COMMENT {
            self.description = match value {
                PropValue::Text(t) => Some(t),
                _ => None,
            };
            return hr::S_OK;
        }
        if *key == props::PKEY_TOAST_ACTIVATOR_CLSID
            && let PropValue::Text(t) = &value
        {
            if let Some(g) = Guid::parse(t) {
                self.properties.set(key, PropValue::Clsid(g));
            }
            return hr::S_FALSE;
        }
        self.properties.set(key, value);
        hr::S_OK
    }

    fn comment(&self) -> Option<&[u16]> {
        self.description.as_deref().filter(|d| !d.is_empty())
    }
}

/// The show command a file keeps: 3 and 7 as they are, anything else 1.
pub fn file_show_command(show: i32) -> i32 {
    if show == SW_SHOWMAXIMIZED || show == SW_SHOWMINNOACTIVE { show } else { SW_SHOWNORMAL }
}

/// The value of `System.VolumeId` for a volume: a GUID made from its serial number (Poly's
/// volumes have no GUID names; the value only has to tell volumes apart).
pub fn volume_guid(serial: u32) -> Guid {
    Guid { data1: serial, data2: 0x504F, data3: 0x4C59, data4: *b"VOLUMEID" }
}

#[cfg(test)]
mod tests;
