//! The state of the Common Item Dialog's Save object (`CLSID_FileSaveDialog`,
//! `IFileSaveDialog`) before and after it is shown, and how it becomes the `OPENFILENAMEW`
//! of Poly's one file dialog (ADR 0223).
//!
//! Sources (clean room): the spec sheets `specs/windows/comdlg32.dll/IFileSaveDialog.*.yaml`
//! (defaults, what each setter stores and refuses); Learn: "IFileDialog", "IFileSaveDialog",
//! "_FILEOPENDIALOGOPTIONS", "COMDLG_FILTERSPEC", "OPENFILENAMEW"; the constants are
//! `shobjidl_core.h`'s, `commdlg.h`'s and `winerror.h`'s.

use alloc::vec::Vec;

pub const S_OK: u32 = 0;
pub const S_FALSE: u32 = 1;
pub const E_FAIL: u32 = 0x8000_4005;
pub const E_UNEXPECTED: u32 = 0x8000_FFFF;
pub const E_INVALIDARG: u32 = 0x8007_0057;
/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`: what `Show` returns when the user cancels.
pub const CANCELLED: u32 = 0x8007_04C7;

/// `FOS_*` (`shobjidl_core.h`).
pub mod fos {
    pub const OVERWRITEPROMPT: u32 = 0x2;
    pub const STRICTFILETYPES: u32 = 0x4;
    pub const NOCHANGEDIR: u32 = 0x8;
    pub const PICKFOLDERS: u32 = 0x20;
    pub const FORCEFILESYSTEM: u32 = 0x40;
    pub const ALLNONSTORAGEITEMS: u32 = 0x80;
    pub const NOVALIDATE: u32 = 0x100;
    pub const ALLOWMULTISELECT: u32 = 0x200;
    pub const PATHMUSTEXIST: u32 = 0x800;
    pub const FILEMUSTEXIST: u32 = 0x1000;
    pub const CREATEPROMPT: u32 = 0x2000;
    pub const SHAREAWARE: u32 = 0x4000;
    pub const NOREADONLYRETURN: u32 = 0x8000;
    pub const NOTESTFILECREATE: u32 = 0x1_0000;
    pub const HIDEMRUPLACES: u32 = 0x2_0000;
    pub const HIDEPINNEDPLACES: u32 = 0x4_0000;
    pub const NODEREFERENCELINKS: u32 = 0x10_0000;
    pub const OKBUTTONNEEDSINTERACTION: u32 = 0x20_0000;
    pub const DONTADDTORECENT: u32 = 0x200_0000;
    pub const FORCESHOWHIDDEN: u32 = 0x1000_0000;
    pub const DEFAULTNOMINIMODE: u32 = 0x2000_0000;
    pub const FORCEPREVIEWPANEON: u32 = 0x4000_0000;
    pub const SUPPORTSTREAMABLEITEMS: u32 = 0x8000_0000;
    /// Every option the header defines.
    pub const ALL: u32 = OVERWRITEPROMPT
        | STRICTFILETYPES
        | NOCHANGEDIR
        | PICKFOLDERS
        | FORCEFILESYSTEM
        | ALLNONSTORAGEITEMS
        | NOVALIDATE
        | ALLOWMULTISELECT
        | PATHMUSTEXIST
        | FILEMUSTEXIST
        | CREATEPROMPT
        | SHAREAWARE
        | NOREADONLYRETURN
        | NOTESTFILECREATE
        | HIDEMRUPLACES
        | HIDEPINNEDPLACES
        | NODEREFERENCELINKS
        | OKBUTTONNEEDSINTERACTION
        | DONTADDTORECENT
        | FORCESHOWHIDDEN
        | DEFAULTNOMINIMODE
        | FORCEPREVIEWPANEON
        | SUPPORTSTREAMABLEITEMS;
}

/// The options of a new Save dialog (the sheet's `options`).
pub const DEFAULT_SAVE_OPTIONS: u32 =
    fos::OVERWRITEPROMPT | fos::NOCHANGEDIR | fos::PATHMUSTEXIST | fos::NOREADONLYRETURN;

/// The `FOS_*` options that are `OFN_*` flags of the same value (`commdlg.h` gives them the
/// same bits) and mean the same in Poly's dialog.
const SAME_AS_OFN: u32 = fos::OVERWRITEPROMPT
    | fos::NOCHANGEDIR
    | fos::NOVALIDATE
    | fos::ALLOWMULTISELECT
    | fos::PATHMUSTEXIST
    | fos::FILEMUSTEXIST
    | fos::CREATEPROMPT
    | fos::SHAREAWARE
    | fos::NOREADONLYRETURN
    | fos::NOTESTFILECREATE
    | fos::NODEREFERENCELINKS
    | fos::DONTADDTORECENT
    | fos::FORCESHOWHIDDEN;
/// `OFN_HIDEREADONLY`, `OFN_EXPLORER` (`commdlg.h`).
const OFN_HIDEREADONLY: u32 = 0x4;
const OFN_EXPLORER: u32 = 0x8_0000;

/// One `COMDLG_FILTERSPEC`: the type's name and its patterns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileType {
    pub name: Vec<u16>,
    pub spec: Vec<u16>,
}

/// What the dialog holds.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub options: u32,
    /// `None` before any `SetFileName`.
    pub name: Option<Vec<u16>>,
    /// `None` before `SetFileTypes`.
    pub types: Option<Vec<FileType>>,
    /// 1-based, 0 for none chosen.
    pub type_index: u32,
    pub default_ext: Option<Vec<u16>>,
    pub title: Option<Vec<u16>>,
    pub folder: Option<Vec<u16>>,
    pub default_folder: Option<Vec<u16>>,
    /// The path the user chose (`None`: not shown, or cancelled).
    pub result: Option<Vec<u16>>,
}

impl State {
    pub fn new_save() -> State {
        State { options: DEFAULT_SAVE_OPTIONS, ..State::default() }
    }

    /// `SetOptions`: stored exactly, unless it has a bit the header does not define or asks a
    /// Save dialog to pick folders (`E_INVALIDARG`, the options stay).
    pub fn set_options(&mut self, options: u32) -> u32 {
        if options & !fos::ALL != 0 || options & fos::PICKFOLDERS != 0 {
            return E_INVALIDARG;
        }
        self.options = options;
        S_OK
    }

    /// `SetFileName`: `S_FALSE` when the name is already the one held.
    pub fn set_file_name(&mut self, name: &[u16]) -> u32 {
        if self.name.as_deref() == Some(name) {
            return S_FALSE;
        }
        self.name = Some(name.to_vec());
        S_OK
    }

    /// `GetFileName`: `E_FAIL` and an empty name before any `SetFileName`.
    pub fn file_name(&self) -> (u32, &[u16]) {
        match &self.name {
            Some(n) => (S_OK, n),
            None => (E_FAIL, &[]),
        }
    }

    /// `SetFileTypes`: no entries is `E_INVALIDARG`; only once per dialog (`E_UNEXPECTED`).
    pub fn set_file_types(&mut self, types: Option<Vec<FileType>>) -> u32 {
        let Some(types) = types.filter(|t| !t.is_empty()) else { return E_INVALIDARG };
        if self.types.is_some() {
            return E_UNEXPECTED;
        }
        self.types = Some(types);
        S_OK
    }

    /// `SetFileTypeIndex`: 1-based; 0 and an index past the end leave the index as it is.
    pub fn set_type_index(&mut self, index: u32) -> u32 {
        let count = self.types.as_ref().map_or(0, Vec::len);
        if index >= 1 && index as usize <= count {
            self.type_index = index;
        }
        S_OK
    }

    /// The `lpstrFilter` list of the types: `name\0spec\0` each, and one more `\0`.
    pub fn filter(&self) -> Option<Vec<u16>> {
        let types = self.types.as_ref()?;
        let mut out = Vec::new();
        for t in types {
            out.extend_from_slice(&t.name);
            out.push(0);
            out.extend_from_slice(&t.spec);
            out.push(0);
        }
        out.push(0);
        Some(out)
    }

    /// The default extension `OPENFILENAMEW` takes: without a leading dot (empty: none).
    pub fn ofn_default_ext(&self) -> Option<Vec<u16>> {
        let ext = self.default_ext.as_deref()?;
        let ext = ext.strip_prefix(&[u16::from(b'.')]).unwrap_or(ext);
        (!ext.is_empty()).then(|| ext.to_vec())
    }

    /// The `OFN_*` flags of the options, for Poly's file dialog.
    pub fn ofn_flags(&self) -> u32 {
        (self.options & SAME_AS_OFN) | OFN_EXPLORER | OFN_HIDEREADONLY
    }

    /// The folder the dialog opens in: `SetFolder`'s, else `SetDefaultFolder`'s, else `fallback`.
    pub fn start_folder<'a>(&'a self, fallback: &'a [u16]) -> &'a [u16] {
        self.folder.as_deref().or(self.default_folder.as_deref()).unwrap_or(fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn options_as_the_sheet_saw_them() {
        let mut s = State::new_save();
        assert_eq!(s.options, 0x880A);
        assert_eq!(s.set_options(0x880E), S_OK);
        assert_eq!(s.options, 0x880E);
        for one in [2, 4, 8, 0x40, 0x100, 0x1000, 0x200_0000, 0x1000_0000, 0] {
            assert_eq!(s.set_options(one), S_OK);
            assert_eq!(s.options, one);
        }
        s.set_options(0x880A);
        for bad in [0x20, 0xFFFF_FFFF, 0x1234_5678] {
            assert_eq!(s.set_options(bad), E_INVALIDARG);
            assert_eq!(s.options, 0x880A);
        }
    }

    #[test]
    fn file_names() {
        let mut s = State::new_save();
        assert_eq!(s.file_name(), (E_FAIL, &[][..]));
        assert_eq!(s.set_file_name(&w("poly-download.txt")), S_OK);
        assert_eq!(s.file_name(), (S_OK, &w("poly-download.txt")[..]));
        assert_eq!(s.set_file_name(&w("poly-download.txt")), S_FALSE);
        assert_eq!(s.set_file_name(&[]), S_OK);
        assert_eq!(s.file_name(), (S_OK, &[][..]));
        assert_eq!(s.set_file_name(&w("a\\b.txt")), S_OK);
    }

    #[test]
    fn file_types_and_their_index() {
        let mut s = State::new_save();
        assert_eq!(s.type_index, 0);
        let two = alloc::vec![
            FileType { name: w("Text Document"), spec: w("*.txt") },
            FileType { name: w("All files"), spec: w("*.*") },
        ];
        assert_eq!(s.set_file_types(Some(two.clone())), S_OK);
        assert_eq!(s.type_index, 0);
        s.set_type_index(2);
        assert_eq!(s.type_index, 2);
        s.set_type_index(3);
        s.set_type_index(0);
        assert_eq!(s.type_index, 2);
        assert_eq!(s.set_file_types(None), E_INVALIDARG);
        assert_eq!(s.set_file_types(Some(two)), E_UNEXPECTED);
        let f = s.filter().unwrap();
        assert_eq!(String::from_utf16(&f).unwrap(), "Text Document\0*.txt\0All files\0*.*\0\0");
    }

    #[test]
    fn what_poly_s_dialog_gets() {
        let mut s = State::new_save();
        assert_eq!(s.ofn_flags(), 0x880A | OFN_EXPLORER | OFN_HIDEREADONLY);
        s.default_ext = Some(w(".txt"));
        assert_eq!(s.ofn_default_ext(), Some(w("txt")));
        s.default_ext = Some(Vec::new());
        assert_eq!(s.ofn_default_ext(), None);
        let docs = w("C:\\Users\\poly\\Documents");
        assert_eq!(s.start_folder(&docs), &docs[..]);
        s.default_folder = Some(w("C:\\d"));
        assert_eq!(s.start_folder(&docs), &w("C:\\d")[..]);
        s.folder = Some(w("C:\\Users\\poly\\Downloads"));
        assert_eq!(s.start_folder(&docs), &w("C:\\Users\\poly\\Downloads")[..]);
    }

    use alloc::string::String;
}
