//! The Open and Save dialogs (`GetOpenFileNameW`, `GetSaveFileNameW`; Learn, "OPENFILENAMEW
//! structure", "Open and Save As Dialog Boxes").
//!
//! One dialog serves both: a path, a list of the folders and files of the folder (folders
//! first, only files that match the chosen filter), the file name, the filter list and the
//! buttons. A hook gets what the style says: the dialog itself for the old style
//! (`WM_INITDIALOG`, then every message first), a child dialog of the dialog for
//! `OFN_EXPLORER` (`WM_INITDIALOG`, then `WM_NOTIFY` with `CDN_INITDONE`).

use crate::sys::{self, H};
use crate::util::*;
use alloc::boxed::Box;
use alloc::vec::Vec;
use poly_comdlg::cderr;
use poly_comdlg::check::{self, Facts, ofn};
use poly_comdlg::filter::{self, Filter};
use poly_comdlg::path::{self, Fill};
use poly_comdlg::template::{Class, Dialog, Item, style::*};

/// Offsets in `OPENFILENAMEW` on x64 (`commdlg.h`).
#[allow(dead_code)]
mod at {
    pub const SIZE: usize = 0;
    pub const OWNER: usize = 8;
    pub const INSTANCE: usize = 16;
    pub const FILTER: usize = 24;
    pub const CUSTOM_FILTER: usize = 32;
    pub const MAX_CUSTOM: usize = 40;
    pub const FILTER_INDEX: usize = 44;
    pub const FILE: usize = 48;
    pub const MAX_FILE: usize = 56;
    pub const FILE_TITLE: usize = 64;
    pub const MAX_FILE_TITLE: usize = 72;
    pub const INITIAL_DIR: usize = 80;
    pub const TITLE: usize = 88;
    pub const FLAGS: usize = 96;
    pub const FILE_OFFSET: usize = 100;
    pub const FILE_EXTENSION: usize = 102;
    pub const DEF_EXT: usize = 104;
    pub const CUST_DATA: usize = 112;
    pub const HOOK: usize = 120;
    pub const TEMPLATE: usize = 128;
}

/// Control ids (`dlgs.h`): file name `edt1`, file list `lst1`, file type `cmb1`, folder `cmb2`
/// and the labels `stc2` to `stc4`, read only `chx1`; the up button is ours.
const ID_NAME: i32 = 1152;
const ID_LIST: i32 = 1120;
const ID_TYPES: i32 = 1136;
const ID_FOLDER: i32 = 1137;
const ID_RO: i32 = 1040;
const ID_UP: i32 = 1200;
/// `CDN_INITDONE` (`commdlg.h`: `CDN_FIRST` is -601).
const CDN_INITDONE: i32 = -601;
/// `CDN_FILEOK`, `CDN_FOLDERCHANGE`, `CDN_SELCHANGE`, `CDN_TYPECHANGE`.
const CDN_FOLDERCHANGE: i32 = -603;
const CDN_SELCHANGE: i32 = -602;
const CDN_FILEOK: i32 = -606;
const CDN_TYPECHANGE: i32 = -607;

struct Entry {
    name: Vec<u16>,
    dir: bool,
}

struct State {
    ofn: *mut u8,
    save: bool,
    flags: u32,
    hdlg: H,
    hook: usize,
    child: H,
    cwd: Vec<u16>,
    filters: Vec<Filter>,
    cur_filter: usize,
    typed_pattern: Option<Vec<u16>>,
    entries: Vec<Entry>,
    multi: bool,
    def_ext: Vec<u16>,
    /// `lpstrCustomFilter` gave a first filter, numbered 0.
    has_custom: bool,
}

impl State {
    fn explorer(&self) -> bool {
        self.flags & ofn::EXPLORER != 0
    }
}

/// `GetOpenFileNameW` / `GetSaveFileNameW`: TRUE when the user chose a file.
pub fn run(o: *mut u8, save: bool) -> i32 {
    if o.is_null() {
        crate::set_ext_error(cderr::INITIALIZATION);
        return 0;
    }
    let size: u32 = rd(o, at::SIZE);
    let flags: u32 = rd(o, at::FLAGS);
    let hook: usize = rd(o, at::HOOK);
    let template: usize = rd(o, at::TEMPLATE);
    let instance: usize = rd(o, at::INSTANCE);
    let facts = Facts {
        struct_size: size,
        flags,
        has_hook: hook != 0,
        has_template_name: template != 0,
        has_instance: instance != 0,
    };
    let n_max: u32 = if size >= 64 { rd(o, at::MAX_FILE) } else { 0 };
    if let Err(code) = check::open_file(&facts, n_max) {
        crate::set_ext_error(code);
        return 0;
    }
    // A template of the program's: it has to exist.
    if flags & ofn::ENABLETEMPLATE != 0 && !has_dialog_resource(instance, template as *const u16) {
        crate::set_ext_error(cderr::FINDRESFAILURE);
        return 0;
    }
    let owner: H = rd(o, at::OWNER);
    // SAFETY: a window handle query.
    if owner != 0 && unsafe { sys::IsWindow(owner) } == 0 {
        crate::set_ext_error(cderr::DIALOGFAILURE);
        return 0;
    }
    let mut st = Box::new(State {
        ofn: o,
        save,
        flags,
        hdlg: 0,
        hook,
        child: 0,
        cwd: Vec::new(),
        filters: Vec::new(),
        cur_filter: 0,
        typed_pattern: None,
        entries: Vec::new(),
        multi: flags & ofn::ALLOWMULTISELECT != 0,
        def_ext: Vec::new(),
        has_custom: false,
    });
    let title = {
        let t = rd::<*const u16>(o, at::TITLE);
        if t.is_null() { if save { "Save As" } else { "Open" } } else { "" }
    };
    let mut items = alloc::vec![
        Item {
            class: Class::Static,
            text: "Look &in:",
            id: 1091,
            style: WS_CHILD | WS_VISIBLE | SS_LEFT,
            ex_style: 0,
            x: 7,
            y: 8,
            cx: 40,
            cy: 9
        },
        Item {
            class: Class::Edit,
            text: "",
            id: ID_FOLDER as u16,
            style: WS_CHILD | WS_VISIBLE | WS_BORDER | ES_READONLY | ES_AUTOHSCROLL,
            ex_style: 0,
            x: 50,
            y: 6,
            cx: 200,
            cy: 12
        },
        Item {
            class: Class::Button,
            text: "&Up",
            id: ID_UP as u16,
            style: WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            ex_style: 0,
            x: 256,
            y: 5,
            cx: 37,
            cy: 14
        },
        Item {
            class: Class::ListBox,
            text: "",
            id: ID_LIST as u16,
            style: WS_CHILD
                | WS_VISIBLE
                | WS_BORDER
                | WS_VSCROLL
                | WS_TABSTOP
                | LBS_NOTIFY
                | LBS_NOINTEGRALHEIGHT
                | if st.multi { 0x800 } else { 0 },
            ex_style: 0,
            x: 7,
            y: 24,
            cx: 286,
            cy: 104
        },
        Item {
            class: Class::Static,
            text: "File &name:",
            id: 1090,
            style: WS_CHILD | WS_VISIBLE | SS_LEFT,
            ex_style: 0,
            x: 7,
            y: 135,
            cx: 45,
            cy: 9
        },
        Item {
            class: Class::Edit,
            text: "",
            id: ID_NAME as u16,
            style: WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
            ex_style: 0,
            x: 55,
            y: 133,
            cx: 165,
            cy: 12
        },
        Item {
            class: Class::Static,
            text: "Files of &type:",
            id: 1089,
            style: WS_CHILD | WS_VISIBLE | SS_LEFT,
            ex_style: 0,
            x: 7,
            y: 153,
            cx: 45,
            cy: 9
        },
        Item {
            class: Class::ListBox,
            text: "",
            id: ID_TYPES as u16,
            style: WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | WS_TABSTOP | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT,
            ex_style: 0,
            x: 55,
            y: 150,
            cx: 165,
            cy: 26
        },
        Item {
            class: Class::Button,
            text: if save { "&Save" } else { "&Open" },
            id: IDOK as u16,
            style: WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
            ex_style: 0,
            x: 230,
            y: 132,
            cx: 63,
            cy: 14
        },
        Item {
            class: Class::Button,
            text: "Cancel",
            id: IDCANCEL as u16,
            style: WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
            ex_style: 0,
            x: 230,
            y: 150,
            cx: 63,
            cy: 14
        },
    ];
    if flags & ofn::HIDEREADONLY == 0 {
        items.push(Item {
            class: Class::Button,
            text: "&Read only",
            id: ID_RO as u16,
            style: WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX,
            ex_style: 0,
            x: 7,
            y: 181,
            cx: 80,
            cy: 10,
        });
    }
    let dlg = Dialog {
        style: WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_CENTER | DS_3DLOOK,
        ex_style: 0,
        x: 0,
        y: 0,
        cx: 300,
        cy: 196,
        title,
        font: Some((8, "MS Shell Dlg")),
        items,
    };
    let bytes = dlg.build();
    let param = &mut *st as *mut State as isize;
    // An old-style template of the program's replaces the dialog: it has the same control ids.
    let result = if flags & ofn::ENABLETEMPLATE != 0 && flags & ofn::EXPLORER == 0 {
        // SAFETY: a resource dialog of the program; the procedure is ours.
        let r = unsafe { sys::DialogBoxParamW(instance, template as *const u16, owner, proc_, param) };
        if r == -1 { Err(cderr::DIALOGFAILURE) } else { Ok(r) }
    } else {
        run_modal(&bytes, owner, proc_, param)
    };
    match result {
        Ok(1) => {
            crate::set_ext_error(0);
            1
        }
        Ok(0) | Ok(2) => {
            // Cancelled, or a failure the dialog recorded (it set the error itself then).
            if !st_error_set() {
                crate::set_ext_error(0);
            }
            0
        }
        Ok(_) => {
            crate::set_ext_error(0);
            0
        }
        Err(code) => {
            crate::set_ext_error(code);
            0
        }
    }
}

/// Whether the dialog left a specific error (set through [`fail`]) for `run` to keep.
static FAILED: poly_win32_gui::SpinLock<bool> = poly_win32_gui::SpinLock::new(false);

fn st_error_set() -> bool {
    FAILED.with(|f| core::mem::replace(f, false))
}

/// End the dialog with an error: the code is recorded and the dialog ends with 0.
fn fail(hdlg: H, code: u32) {
    crate::set_ext_error(code);
    FAILED.with(|f| *f = true);
    // SAFETY: the dialog.
    unsafe { sys::EndDialog(hdlg, 0) };
}

fn state<'a>(hdlg: H) -> Option<&'a mut State> {
    // SAFETY: the user data holds the pointer `WM_INITDIALOG` stored; it lives as long as the dialog.
    let p = unsafe { sys::GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut State;
    // SAFETY: see above.
    unsafe { p.as_mut() }
}

/// The dialog procedure.
unsafe extern "system" fn proc_(hdlg: H, msg: u32, w: usize, l: isize) -> isize {
    if msg == WM_INITDIALOG {
        // SAFETY: `l` is the `State` `run` made.
        unsafe { sys::SetWindowLongPtrW(hdlg, GWLP_USERDATA, l) };
        let Some(st) = state(hdlg) else { return 0 };
        st.hdlg = hdlg;
        init(st);
        // `init` put the focus in the file name field: FALSE keeps it there (Learn,
        // "WM_INITDIALOG": TRUE would move it to the first control, the Up button), so Enter
        // means Save as in Windows' dialog.
        return 0;
    }
    let Some(st) = state(hdlg) else { return 0 };
    if !st.explorer() && st.hook != 0 {
        let r = call_hook(st.hook, hdlg, msg, w, l);
        if r != 0 {
            return r as isize;
        }
    }
    if msg == WM_COMMAND {
        command(st, loword(w) as i32, hiword(w), l as H);
        return 1;
    }
    0
}

/// The hook's window of an Explorer-style dialog: the child dialog forwards everything to the hook.
unsafe extern "system" fn child_proc(hchild: H, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: the parent dialog's state, as in `state`.
    let parent = unsafe { sys::GetParent(hchild) };
    let Some(st) = state(parent) else {
        // Before WM_INITDIALOG finished for the parent we still know the pointer in `l`.
        return 0;
    };
    if msg == WM_INITDIALOG {
        st.child = hchild;
        let r = call_hook(st.hook, hchild, msg, w, st.ofn as isize);
        return r as isize;
    }
    call_hook(st.hook, hchild, msg, w, l) as isize
}

/// Tell an Explorer-style hook something happened (`WM_NOTIFY` with an `OFNOTIFYW`).
fn notify(st: &State, code: i32) -> usize {
    if st.child == 0 || st.hook == 0 {
        return 0;
    }
    // OFNOTIFYW: NMHDR (hwndFrom, idFrom, code), lpOFN, pszFile: 40 bytes on x64.
    let mut n = [0u8; 40];
    wr(n.as_mut_ptr(), 0, st.hdlg);
    wr(n.as_mut_ptr(), 8, 0usize);
    wr(n.as_mut_ptr(), 16, code);
    wr(n.as_mut_ptr(), 24, st.ofn);
    wr(n.as_mut_ptr(), 32, 0usize);
    call_hook(st.hook, st.child, WM_NOTIFY, 0, n.as_ptr() as isize)
}

fn init(st: &mut State) {
    let o = st.ofn;
    let hdlg = st.hdlg;
    // The window title.
    let t = rd::<*const u16>(o, at::TITLE);
    if !t.is_null() {
        // SAFETY: the program's NUL-terminated title.
        unsafe { sys::SetWindowTextW(hdlg, t) };
    }
    st.def_ext = read_wstr(rd::<*const u16>(o, at::DEF_EXT), 16);
    // The filters: the program's list, then the custom filter.
    let raw = read_double_nul(rd::<*const u16>(o, at::FILTER));
    st.filters = filter::parse(&raw);
    let custom = rd::<*mut u16>(o, at::CUSTOM_FILTER);
    let n_custom: u32 = rd(o, at::MAX_CUSTOM);
    if !custom.is_null() && n_custom > 0 {
        let d = read_wstr(custom, n_custom as usize);
        // The pattern follows the description's NUL.
        let p = read_wstr(
            // SAFETY: the buffer holds `description NUL pattern NUL` when it has been used.
            unsafe { custom.add(d.len() + 1) },
            n_custom as usize,
        );
        if !d.is_empty() && !p.is_empty() {
            st.has_custom = true;
            st.filters.insert(0, Filter { description: d, patterns: p });
        }
    }
    for f in &st.filters {
        let mut text = f.description.clone();
        if !f.patterns.is_empty() && !f.description.contains(&(b'(' as u16)) {
            text.extend_from_slice(&sys::wz(" (")[..2]);
            text.extend_from_slice(&f.patterns);
            text.push(b')' as u16);
        }
        lb_add(hdlg, ID_TYPES, &text, 0);
    }
    let fi: u32 = rd(o, at::FILTER_INDEX);
    // Without a custom filter the pairs are numbered from 1; with one it is number 0 and
    // the program's pairs follow it.
    st.cur_filter = if st.has_custom {
        (fi as usize).min(st.filters.len() - 1)
    } else if fi >= 1 && (fi as usize) <= st.filters.len() {
        fi as usize - 1
    } else {
        0
    };
    if !st.filters.is_empty() {
        lb_sel(hdlg, ID_TYPES, st.cur_filter);
    }
    // The folder and the name: the program's initial directory, or the path in the buffer, or here.
    let file_buf = rd::<*mut u16>(o, at::FILE);
    let n_max: u32 = rd(o, at::MAX_FILE);
    let initial = if file_buf.is_null() { Vec::new() } else { read_wstr(file_buf, n_max as usize) };
    let mut cwd = read_wstr(rd::<*const u16>(o, at::INITIAL_DIR), 32768);
    let mut name = initial.clone();
    if !initial.is_empty() && path::name_start(&initial) > 0 {
        let start = path::name_start(&initial);
        if cwd.is_empty() {
            cwd = initial[..start].to_vec();
            if cwd.len() > 3 {
                cwd.pop();
            }
        }
        name = initial[start..].to_vec();
    }
    if cwd.is_empty() {
        cwd = current_dir();
    }
    let here = current_dir();
    cwd = path::resolve(&here, &cwd);
    set_text(hdlg, ID_NAME, &name);
    if st.flags & ofn::READONLY != 0 {
        // SAFETY: a plain check box.
        unsafe { sys::CheckDlgButton(hdlg, ID_RO, BST_CHECKED) };
    }
    st.cwd = cwd;
    refresh(st);
    // The Explorer-style hook's child dialog (or the program's template of it).
    if st.explorer() && st.hook != 0 {
        make_child(st);
        notify(st, CDN_INITDONE);
    } else if st.hook != 0 {
        call_hook(st.hook, hdlg, WM_INITDIALOG, 0, st.ofn as isize);
    }
    // SAFETY: a window handle.
    unsafe { sys::SetFocus(sys::GetDlgItem(hdlg, ID_NAME)) };
}

fn make_child(st: &mut State) {
    let o = st.ofn;
    let param = st.ofn as isize;
    if st.flags & ofn::ENABLETEMPLATE != 0 {
        let instance: usize = rd(o, at::INSTANCE);
        let name: usize = rd(o, at::TEMPLATE);
        // SAFETY: the program's resource dialog as a child of ours.
        st.child = unsafe { sys::CreateDialogParamW(instance, name as *const u16, st.hdlg, child_proc, param) };
        return;
    }
    let child = Dialog {
        style: WS_CHILD | DS_CONTROL,
        ex_style: 0,
        x: 0,
        y: 0,
        cx: 0,
        cy: 0,
        title: "",
        font: Some((8, "MS Shell Dlg")),
        items: Vec::new(),
    };
    let bytes = child.build();
    // SAFETY: an in-memory template of ours.
    st.child = unsafe {
        sys::CreateDialogIndirectParamW(crate::util::dll_instance(), bytes.as_ptr(), st.hdlg, child_proc, param)
    };
}

fn current_dir() -> Vec<u16> {
    let mut buf = alloc::vec![0u16; 1024];
    // SAFETY: a buffer of 1024 units.
    let n = unsafe { sys::GetCurrentDirectoryW(1024, buf.as_mut_ptr()) } as usize;
    buf.truncate(n.min(1023));
    if buf.is_empty() {
        buf = sys::wz("C:\\")[..3].to_vec();
    }
    buf
}

fn wz(v: &[u16]) -> Vec<u16> {
    let mut w = v.to_vec();
    w.push(0);
    w
}

/// The attributes of `path`, `None` when it does not exist.
fn attrs(p: &[u16]) -> Option<u32> {
    let w = wz(p);
    // SAFETY: a NUL-terminated path.
    let a = unsafe { sys::GetFileAttributesW(w.as_ptr()) };
    if a == u32::MAX { None } else { Some(a) }
}

fn is_dir(p: &[u16]) -> bool {
    attrs(p).is_some_and(|a| a & 0x10 != 0)
}

/// The folders and the files that match `patterns` in `dir`, folders first, sorted.
fn list_dir(dir: &[u16], patterns: Option<&[u16]>, hidden: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    if dir.is_empty() {
        // The drives.
        // SAFETY: no preconditions.
        let mask = unsafe { sys::GetLogicalDrives() };
        for i in 0..26u16 {
            if mask & (1 << i) != 0 {
                out.push(Entry { name: alloc::vec![b'A' as u16 + i, b':' as u16, b'\\' as u16], dir: true });
            }
        }
        return out;
    }
    let mut pat = path::join(dir, &[b'*' as u16]);
    pat.push(0);
    let mut data = alloc::vec![0u8; 592];
    // SAFETY: a NUL-terminated pattern and a WIN32_FIND_DATAW buffer.
    let h = unsafe { sys::FindFirstFileW(pat.as_ptr(), data.as_mut_ptr()) };
    if h == usize::MAX || h == 0 {
        return out;
    }
    loop {
        let attr: u32 = rd(data.as_ptr(), 0);
        let mut name = Vec::new();
        for i in 0..260 {
            let c: u16 = rd(data.as_ptr(), 44 + 2 * i);
            if c == 0 {
                break;
            }
            name.push(c);
        }
        let dot = name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16];
        let hid = attr & 0x2 != 0 || attr & 0x4 != 0;
        if !dot && (hidden || !hid) {
            let d = attr & 0x10 != 0;
            if d || patterns.is_none_or(|p| filter::matches(p, &name)) {
                out.push(Entry { name, dir: d });
            }
        }
        // SAFETY: the search handle and buffer from above.
        if unsafe { sys::FindNextFileW(h, data.as_mut_ptr()) } == 0 {
            break;
        }
    }
    // SAFETY: the search handle.
    unsafe { sys::FindClose(h) };
    out.sort_by(|a, b| {
        b.dir.cmp(&a.dir).then_with(|| {
            let la: Vec<u16> = a.name.iter().map(|&c| filter::lower(c)).collect();
            let lb: Vec<u16> = b.name.iter().map(|&c| filter::lower(c)).collect();
            la.cmp(&lb)
        })
    });
    out
}

fn refresh(st: &mut State) {
    let hdlg = st.hdlg;
    set_text(hdlg, ID_FOLDER, &st.cwd);
    let patterns: Option<Vec<u16>> = match &st.typed_pattern {
        Some(p) => Some(p.clone()),
        None => st.filters.get(st.cur_filter).map(|f| f.patterns.clone()),
    };
    st.entries = list_dir(&st.cwd, patterns.as_deref(), st.flags & ofn::FORCESHOWHIDDEN != 0);
    lb_reset(hdlg, ID_LIST);
    for (i, e) in st.entries.iter().enumerate() {
        let mut text = Vec::new();
        if e.dir {
            if st.cwd.is_empty() {
                text.extend_from_slice(&e.name);
            } else {
                text.push(b'[' as u16);
                text.extend_from_slice(&e.name);
                text.push(b']' as u16);
            }
        } else {
            text.extend_from_slice(&e.name);
        }
        lb_add(hdlg, ID_LIST, &text, i as isize);
    }
}

fn navigate(st: &mut State, dir: Vec<u16>) {
    st.cwd = dir;
    st.typed_pattern = None;
    refresh(st);
    notify(st, CDN_FOLDERCHANGE);
}

fn command(st: &mut State, id: i32, code: u32, _from: H) {
    let hdlg = st.hdlg;
    match id {
        IDCANCEL => {
            crate::set_ext_error(0);
            // SAFETY: the dialog.
            unsafe { sys::EndDialog(hdlg, 0) };
        }
        IDOK => ok(st),
        ID_UP => {
            let up = path::parent(&st.cwd).unwrap_or_default();
            navigate(st, up);
        }
        ID_TYPES if code == LBN_SELCHANGE => {
            if let Some(i) = lb_cur(hdlg, ID_TYPES) {
                st.cur_filter = i;
                st.typed_pattern = None;
                refresh(st);
                notify(st, CDN_TYPECHANGE);
            }
        }
        ID_LIST if code == LBN_SELCHANGE => selection_to_edit(st),
        ID_LIST if code == LBN_DBLCLK => {
            if let Some(i) = lb_cur(hdlg, ID_LIST) {
                if st.entries.get(i).is_some_and(|e| e.dir) {
                    let dir = if st.cwd.is_empty() {
                        st.entries[i].name.clone()
                    } else {
                        path::resolve(&st.cwd, &st.entries[i].name)
                    };
                    navigate(st, dir);
                } else {
                    selection_to_edit(st);
                    ok(st);
                }
            }
        }
        _ => {}
    }
}

/// Show the selected file or files in the file name box.
fn selection_to_edit(st: &mut State) {
    let hdlg = st.hdlg;
    let mut names: Vec<Vec<u16>> = Vec::new();
    if st.multi {
        // SAFETY: plain messages to the list box.
        let count = unsafe { sys::SendDlgItemMessageW(hdlg, ID_LIST, LB_GETSELCOUNT, 0, 0) };
        if count > 0 {
            let mut idx = alloc::vec![0i32; count as usize];
            // SAFETY: LB_GETSELITEMS writes up to `count` ints.
            unsafe {
                sys::SendDlgItemMessageW(hdlg, ID_LIST, LB_GETSELITEMS, count as usize, idx.as_mut_ptr() as isize)
            };
            for i in idx {
                if let Some(e) = st.entries.get(i as usize)
                    && !e.dir
                {
                    names.push(e.name.clone());
                }
            }
        }
    } else if let Some(i) = lb_cur(hdlg, ID_LIST)
        && let Some(e) = st.entries.get(i)
        && !e.dir
    {
        names.push(e.name.clone());
    }
    if names.is_empty() {
        return;
    }
    let mut text = Vec::new();
    if names.len() == 1 {
        text = names.remove(0);
    } else {
        for (i, n) in names.iter().enumerate() {
            if i > 0 {
                text.push(b' ' as u16);
            }
            text.push(b'"' as u16);
            text.extend_from_slice(n);
            text.push(b'"' as u16);
        }
    }
    set_text(hdlg, ID_NAME, &text);
    notify(st, CDN_SELCHANGE);
}

/// `"a b" "c"` as names; plain text is one name.
fn parse_names(text: &[u16]) -> Vec<Vec<u16>> {
    let q = b'"' as u16;
    let t: Vec<u16> = {
        let s = text.iter().position(|&c| c != b' ' as u16).unwrap_or(text.len());
        text[s..].to_vec()
    };
    if t.first() != Some(&q) {
        return alloc::vec![t];
    }
    let mut out = Vec::new();
    let mut cur: Option<Vec<u16>> = None;
    for &c in &t {
        match (&mut cur, c == q) {
            (None, true) => cur = Some(Vec::new()),
            (Some(v), true) => {
                out.push(core::mem::take(v));
                cur = None;
            }
            (Some(v), false) => v.push(c),
            (None, false) => {}
        }
    }
    out
}

fn message(st: &State, text: &str, caption: &str, kind: u32) -> i32 {
    let (t, c) = (sys::wz(text), sys::wz(caption));
    // SAFETY: NUL-terminated strings.
    unsafe { sys::MessageBoxW(st.hdlg, t.as_ptr(), c.as_ptr(), kind) }
}

fn ok(st: &mut State) {
    let hdlg = st.hdlg;
    let text = get_text(hdlg, ID_NAME);
    let trimmed: &[u16] = {
        let s = text.iter().position(|&c| c != b' ' as u16).unwrap_or(text.len());
        &text[s..]
    };
    if trimmed.is_empty() {
        // SAFETY: no preconditions.
        unsafe { sys::MessageBeep(0) };
        return;
    }
    // A wildcard sets the pattern of the list; a path with one goes there first.
    if trimmed.iter().any(|&c| c == b'*' as u16 || c == b'?' as u16) {
        let at = path::name_start(trimmed);
        if at > 0 {
            let dir = path::resolve(&st.cwd, &trimmed[..at]);
            if is_dir(&dir) {
                st.cwd = dir;
            }
        }
        st.typed_pattern = Some(trimmed[at..].to_vec());
        set_text(hdlg, ID_NAME, &trimmed[at..]);
        refresh(st);
        return;
    }
    let names = parse_names(trimmed);
    let o = st.ofn;
    let buf = rd::<*mut u16>(o, at::FILE);
    let n_max: u32 = rd(o, at::MAX_FILE);
    let must_exist = !st.save && st.flags & ofn::FILEMUSTEXIST != 0;
    let mut paths: Vec<Vec<u16>> = Vec::new();
    for n in &names {
        let mut p = path::resolve(&st.cwd, n);
        if is_dir(&p) && names.len() == 1 {
            navigate(st, p);
            set_text(hdlg, ID_NAME, &[]);
            return;
        }
        if st.flags & ofn::NOVALIDATE == 0 && path::invalid_name(path::file_title(&p)) {
            message(st, "The file name is not valid.", "Poly", MB_ICONERROR);
            return;
        }
        if !st.def_ext.is_empty() && attrs(&p).is_none() {
            let with = path::with_default_extension(&p, &st.def_ext);
            if with != p {
                if !st.save && attrs(&with).is_none() {
                    // Open: the name as typed, the default extension only when that finds a file.
                } else {
                    p = with;
                }
            }
        }
        paths.push(p);
    }
    // Checks on every file.
    for p in &paths {
        let exists = attrs(p).is_some();
        if st.flags & ofn::NOVALIDATE == 0 {
            if (st.flags & ofn::PATHMUSTEXIST != 0 || must_exist) && path::parent(p).is_some_and(|d| !is_dir(&d)) {
                message(st, "The path does not exist.", "Poly", MB_ICONWARNING);
                return;
            }
            if must_exist && !exists {
                message(st, "The file was not found. Check the file name and try again.", "Poly", MB_ICONWARNING);
                return;
            }
        }
        if !st.save && !exists && st.flags & ofn::CREATEPROMPT != 0 {
            if message(st, "The file does not exist. Create it?", "Poly", MB_YESNO | MB_ICONQUESTION) != IDYES {
                return;
            }
        }
        if st.save && exists && st.flags & ofn::OVERWRITEPROMPT != 0 {
            if message(st, "The file already exists. Replace it?", "Poly", MB_YESNO | MB_ICONWARNING) != IDYES {
                return;
            }
        }
    }
    // Hand the result back.
    let first = paths[0].clone();
    let single = paths.len() == 1;
    let dir = if single { Vec::new() } else { st.cwd.clone() };
    let fill = if buf.is_null() {
        Fill::Done { offset: 0, ext: 0 }
    } else {
        // SAFETY: the program's buffer of `nMaxFile` units.
        let slice = unsafe { core::slice::from_raw_parts_mut(buf, n_max as usize) };
        if single {
            path::fill_single(slice, &first)
        } else {
            let rel: Vec<Vec<u16>> = paths.iter().map(|p| path::file_title(p).to_vec()).collect();
            if st.explorer() {
                path::fill_multi_explorer(slice, &dir, &rel)
            } else {
                path::fill_multi_old(slice, &dir, &rel)
            }
        }
    };
    match fill {
        Fill::TooSmall(needed) => {
            if !buf.is_null() {
                // SAFETY: the buffer has at least one unit.
                unsafe { *buf = needed as u16 };
            }
            fail(hdlg, cderr::FNERR_BUFFERTOOSMALL);
            return;
        }
        Fill::Done { offset, ext } => {
            wr(o, at::FILE_OFFSET, offset);
            wr(o, at::FILE_EXTENSION, ext);
        }
    }
    let title_buf = rd::<*mut u16>(o, at::FILE_TITLE);
    let n_title: u32 = rd(o, at::MAX_FILE_TITLE);
    write_wstr(title_buf, n_title as usize, path::file_title(&first));
    let mut flags: u32 = rd(o, at::FLAGS);
    flags &= !ofn::READONLY;
    // SAFETY: a plain check box query.
    if unsafe { sys::IsDlgButtonChecked(hdlg, ID_RO) } == BST_CHECKED {
        flags |= ofn::READONLY;
    }
    wr(o, at::FLAGS, flags);
    if !st.filters.is_empty() {
        let index = if st.has_custom { st.cur_filter } else { st.cur_filter + 1 };
        wr(o, at::FILTER_INDEX, index as u32);
    }
    if st.flags & ofn::NOCHANGEDIR == 0 {
        let w = wz(&st.cwd);
        // SAFETY: a NUL-terminated path.
        unsafe { sys::SetCurrentDirectoryW(w.as_ptr()) };
    }
    if notify(st, CDN_FILEOK) != 0 {
        return;
    }
    crate::set_ext_error(0);
    // SAFETY: the dialog.
    unsafe { sys::EndDialog(hdlg, 1) };
}
