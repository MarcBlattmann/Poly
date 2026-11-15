//! The ANSI side of user32: strings through the process's ANSI code page (1252, as
//! `GetACP` says) and the conversion of the messages whose parameters carry text.
//!
//! A window has a form: the form of the procedure it is called through. A window made through
//! an `A` function (or of a class registered with `RegisterClassA`/`ExA`) is an *ANSI window*:
//! its own procedure gets bytes. Our own window classes (`Button`, `Edit`, `#32770`, ...) work
//! on UTF-16 whatever `IsWindowUnicode` says. Messages are UTF-16 by default; `SendMessageA`
//! and its relatives mark the one they send as ANSI. Where the form of the message and the form
//! of the procedure differ [`enter`] converts the parameters and [`Conv::finish`] the result
//! (`specs/windows/user32.dll/{CreateDialogParamA,DefDlgProcA,DefDlgProcW,
//! SendDlgItemMessageA,SetDlgItemTextA,GetDlgItemTextA}.yaml`): a text arriving at an ANSI
//! procedure has the characters the code page lacks turned into `?`, buffers for text the other
//! form writes are twice the size when the sender uses UTF-16 (`WM_GETTEXT` shows the size
//! doubled), and the lengths stay what they are (one byte per unit in a single-byte code
//! page).

use super::{H, SYS, SpinLock, info};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_win32k::user::Env as _;
use poly_kernelbase::text::{from_ansi, to_ansi};

/// The form of a message or of a window procedure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Ansi,
    Wide,
}

/// The form of the message the next call of a window procedure on a thread carries (`SendMessageA`
/// and its relatives set it around the send; `call_proc` takes it). Messages are UTF-16 unless
/// the sender set it.
static FORMS: SpinLock<BTreeMap<u32, Form>> = SpinLock::new(BTreeMap::new());

/// Runs `run` with the calling thread's next message marked as `form`.
pub fn with_form<R>(form: Form, run: impl FnOnce() -> R) -> R {
    let tid = SYS.tid();
    FORMS.with(|m| m.insert(tid, form));
    let r = run();
    FORMS.with(|m| m.remove(&tid));
    r
}

/// The form of the message `call_proc` is about to deliver (and back to UTF-16).
pub fn take_form() -> Form {
    let tid = SYS.tid();
    FORMS.with(|m| m.remove(&tid)).unwrap_or(Form::Wide)
}

/// `from_ansi` over a NUL-terminated string.
///
/// # Safety
/// `p` is null or a NUL-terminated string.
pub unsafe fn wide_of_a(p: *const u8) -> Vec<u16> {
    if p.is_null() {
        return Vec::new();
    }
    let mut n = 0;
    // SAFETY: NUL-terminated.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: n bytes before the NUL.
    from_ansi(unsafe { core::slice::from_raw_parts(p, n) })
}

/// The bytes of a NUL-terminated string.
///
/// # Safety
/// `p` is null or a NUL-terminated string.
pub unsafe fn bytes_of(p: *const u8) -> Vec<u8> {
    if p.is_null() {
        return Vec::new();
    }
    let mut n = 0;
    // SAFETY: NUL-terminated.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: n bytes before the NUL.
    unsafe { core::slice::from_raw_parts(p, n) }.to_vec()
}

/// `to_ansi` of a UTF-16 string (a lone `?` for what the code page lacks).
pub fn ansi_of(units: &[u16]) -> Vec<u8> {
    to_ansi(units)
}

/// Copies at most `cap - 1` bytes of `text` and a NUL into `buf`; the number of bytes copied.
///
/// # Safety
/// `buf` has room for `cap` bytes.
pub unsafe fn put_a(buf: *mut u8, cap: usize, text: &[u8]) -> usize {
    if buf.is_null() || cap == 0 {
        return 0;
    }
    let n = text.len().min(cap - 1);
    // SAFETY: n + 1 <= cap.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), buf, n);
        *buf.add(n) = 0;
    }
    n
}

/// As [`put_a`] for UTF-16.
///
/// # Safety
/// `buf` has room for `cap` units.
pub unsafe fn put_w(buf: *mut u16, cap: usize, text: &[u16]) -> usize {
    if buf.is_null() || cap == 0 {
        return 0;
    }
    let n = text.len().min(cap - 1);
    // SAFETY: n + 1 <= cap.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), buf, n);
        *buf.add(n) = 0;
    }
    n
}

pub const WM_CREATE: u32 = 0x0001;
pub const WM_SETTEXT: u32 = 0x000C;
pub const WM_GETTEXT: u32 = 0x000D;
pub const WM_GETTEXTLENGTH: u32 = 0x000E;
pub const WM_NCCREATE: u32 = 0x0081;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_DEADCHAR: u32 = 0x0103;
pub const WM_SYSCHAR: u32 = 0x0106;
pub const WM_SYSDEADCHAR: u32 = 0x0107;
pub const WM_IME_CHAR: u32 = 0x0286;
const EM_GETLINE: u32 = 0x00C4;
const EM_REPLACESEL: u32 = 0x00C2;
const LB_ADDSTRING: u32 = 0x0180;
const LB_INSERTSTRING: u32 = 0x0181;
const LB_SELECTSTRING: u32 = 0x018C;
const LB_DIR: u32 = 0x018D;
const LB_FINDSTRING: u32 = 0x018F;
const LB_GETTEXT: u32 = 0x0189;
const LB_FINDSTRINGEXACT: u32 = 0x01A2;
const CB_ADDSTRING: u32 = 0x0143;
const CB_DIR: u32 = 0x0145;
const CB_GETLBTEXT: u32 = 0x0148;
const CB_INSERTSTRING: u32 = 0x014A;
const CB_FINDSTRING: u32 = 0x014C;
const CB_SELECTSTRING: u32 = 0x014D;
const CB_FINDSTRINGEXACT: u32 = 0x0158;
const LB_GETTEXTLEN: u32 = 0x018A;
const CB_GETLBTEXTLEN: u32 = 0x0149;

/// What a message's parameters hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Nothing that depends on the form.
    None,
    /// `lParam` is a NUL-terminated string.
    Str,
    /// A buffer the receiver fills: `WM_GETTEXT`, `LB_GETTEXT`, `CB_GETLBTEXT`, `EM_GETLINE`.
    Buf,
    /// `wParam` is a character.
    Char,
    /// `lParam` is a `CREATESTRUCT`.
    Create,
}

fn kind(msg: u32, style: u32) -> Kind {
    let list_has_text = style & 0x0030 == 0 || style & 0x0040 != 0;
    let combo_has_text = style & 0x0030 == 0 || style & 0x0200 != 0;
    match msg {
        WM_SETTEXT | EM_REPLACESEL => Kind::Str,
        LB_ADDSTRING | LB_INSERTSTRING | LB_SELECTSTRING | LB_FINDSTRING | LB_FINDSTRINGEXACT | LB_DIR
            if list_has_text =>
        {
            Kind::Str
        }
        CB_ADDSTRING | CB_INSERTSTRING | CB_SELECTSTRING | CB_FINDSTRING | CB_FINDSTRINGEXACT | CB_DIR
            if combo_has_text =>
        {
            Kind::Str
        }
        WM_GETTEXT | EM_GETLINE => Kind::Buf,
        LB_GETTEXT | CB_GETLBTEXT => Kind::Buf,
        WM_CHAR | WM_DEADCHAR | WM_SYSCHAR | WM_SYSDEADCHAR | WM_IME_CHAR => Kind::Char,
        WM_CREATE | WM_NCCREATE => Kind::Create,
        _ => Kind::None,
    }
}

/// Whether a message of this kind needs converting at all (checked before a window is looked up).
pub fn is_text_message(msg: u32) -> bool {
    kind(msg, 0) != Kind::None || matches!(msg, LB_GETTEXTLEN | CB_GETLBTEXTLEN | WM_GETTEXTLENGTH)
}

/// The parameters of a message in another form, what to put back afterwards and what to keep alive.
pub struct Conv {
    pub w: usize,
    pub l: isize,
    undo: Undo,
    /// Keeps the converted strings and structures alive while the procedure runs.
    _hold: Vec<Vec<u8>>,
}

enum Undo {
    None,
    /// A buffer filled by the receiver in form `from` (`to` receives); copy it back to `buf`.
    Buf { to: Form, buf: usize, cap: usize, temp: usize, line: bool },
}

/// The form of the procedure window `h` is called through.
pub fn proc_form(h: H) -> Form {
    match info(h) {
        Some(i) if !i.unicode && i.wndproc > poly_win32k::dlg::sysproc::MAX => Form::Ansi,
        _ => Form::Wide,
    }
}

/// Converts the parameters of message `msg` from form `from` to form `to`.
pub fn enter(h: H, from: Form, to: Form, msg: u32, w: usize, l: isize) -> Conv {
    let plain = |w, l| Conv { w, l, undo: Undo::None, _hold: Vec::new() };
    if from == to {
        return plain(w, l);
    }
    // The list and combo box messages hold text only for boxes that keep strings.
    let style = if (0x0140..=0x01B0).contains(&msg) { info(h).map_or(0, |i| i.style) } else { 0 };
    match kind(msg, style) {
        Kind::None => plain(w, l),
        Kind::Str => {
            if l == 0 {
                return plain(w, l);
            }
            let bytes: Vec<u8> = match to {
                Form::Wide => {
                    // SAFETY: the sender's NUL-terminated string.
                    let units = unsafe { wide_of_a(l as *const u8) };
                    let mut b: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
                    b.extend_from_slice(&[0, 0]);
                    b
                }
                Form::Ansi => {
                    // SAFETY: the sender's NUL-terminated UTF-16 string.
                    let units = unsafe { super::wide(l as *const u16, -1) };
                    let mut b = ansi_of(&units);
                    b.push(0);
                    b
                }
            };
            let p = bytes.as_ptr() as isize;
            Conv { w, l: p, undo: Undo::None, _hold: alloc::vec![bytes] }
        }
        Kind::Char => {
            let c = match to {
                Form::Wide => from_ansi(&[w as u8])[0] as usize,
                Form::Ansi => ansi_of(&[w as u16])[0] as usize,
            };
            plain(c, l)
        }
        Kind::Buf => {
            if l == 0 {
                return plain(w, l);
            }
            let line = msg == EM_GETLINE;
            // The size the sender gave: characters for `WM_GETTEXT`, in the first word for
            // `EM_GETLINE`; the item's length (asked of the window) for the list messages.
            let cap = match msg {
                WM_GETTEXT => w,
                // SAFETY: the sender's buffer starts with the size word.
                EM_GETLINE => unsafe { (l as *const u16).read_unaligned() as usize },
                _ => 4096,
            };
            // The receiver writes at most `size` units; a UTF-16 sender gives an ANSI receiver
            // twice the size (and its own size is what the receiver's string must fit in).
            let temp_cap = match (from, to) {
                (Form::Wide, Form::Ansi) => cap * 2,
                _ => cap,
            };
            let unit = if to == Form::Wide { 2 } else { 1 };
            let mut buf = alloc::vec![0u8; (temp_cap + 2) * unit];
            if line {
                let n = (temp_cap as u16).to_le_bytes();
                buf[0] = n[0];
                buf[1] = n[1];
            }
            let p = buf.as_mut_ptr() as usize;
            let nw = match msg {
                WM_GETTEXT => temp_cap,
                _ => w,
            };
            Conv {
                w: nw,
                l: p as isize,
                undo: Undo::Buf { to, buf: l as usize, cap, temp: p, line },
                _hold: alloc::vec![buf],
            }
        }
        Kind::Create => {
            if l == 0 {
                return plain(w, l);
            }
            // The CREATESTRUCT is the same in both forms but for what its two string pointers
            // point at.
            let mut hold = Vec::new();
            // SAFETY: the sender's CREATESTRUCT.
            let mut cs = unsafe { (l as *const poly_win32k::user::CreateStructW).read_unaligned() };
            for field in [&mut cs.name, &mut cs.class] {
                if *field > 0xFFFF {
                    let bytes: Vec<u8> = match to {
                        Form::Wide => {
                            // SAFETY: a NUL-terminated string.
                            let u = unsafe { wide_of_a(*field as *const u8) };
                            let mut b: Vec<u8> = u.iter().flat_map(|c| c.to_le_bytes()).collect();
                            b.extend_from_slice(&[0, 0]);
                            b
                        }
                        Form::Ansi => {
                            // SAFETY: a NUL-terminated UTF-16 string.
                            let u = unsafe { super::wide(*field as *const u16, -1) };
                            let mut b = ansi_of(&u);
                            b.push(0);
                            b
                        }
                    };
                    *field = bytes.as_ptr() as usize;
                    hold.push(bytes);
                }
            }
            let mut copy = alloc::vec![0u8; core::mem::size_of::<poly_win32k::user::CreateStructW>()];
            // SAFETY: a CreateStructW of the right size.
            unsafe { (copy.as_mut_ptr() as *mut poly_win32k::user::CreateStructW).write_unaligned(cs) };
            let p = copy.as_ptr() as isize;
            hold.push(copy);
            Conv { w, l: p, undo: Undo::None, _hold: hold }
        }
    }
}

impl Conv {
    /// Takes the result of the receiver back to the sender's form (and copies the text it wrote).
    pub fn finish(self, from: Form, result: isize) -> isize {
        match self.undo {
            Undo::None => result,
            Undo::Buf { to, buf, cap, temp, line } => {
                let n = result.max(0) as usize;
                // The text the receiver wrote, in the receiver's form, converted into the
                // sender's buffer (`cap` units of the sender's form).
                match to {
                    Form::Wide => {
                        // SAFETY: the receiver wrote `n` units (or fewer) into our buffer.
                        let units = unsafe { core::slice::from_raw_parts(temp as *const u16, n.min(cap.max(1) * 2)) };
                        let bytes = ansi_of(units);
                        if from == Form::Ansi {
                            // SAFETY: the sender's buffer holds `cap` bytes.
                            let m = unsafe { put_a(buf as *mut u8, cap, &bytes) };
                            if line {
                                // EM_GETLINE does not terminate.
                                return n as isize;
                            }
                            return m as isize;
                        }
                    }
                    Form::Ansi => {
                        // SAFETY: the receiver wrote `n` bytes (or fewer) into our buffer.
                        let bytes = unsafe { core::slice::from_raw_parts(temp as *const u8, n.min(cap.max(1) * 2)) };
                        let units = from_ansi(bytes);
                        if from == Form::Wide {
                            // SAFETY: the sender's buffer holds `cap` units.
                            let m = unsafe { put_w(buf as *mut u16, cap, &units) };
                            if line {
                                return n as isize;
                            }
                            return m as isize;
                        }
                    }
                }
                result
            }
        }
    }
}
