//! Accelerator tables: `LoadAcceleratorsW`, `CreateAcceleratorTableW`,
//! `CopyAcceleratorTableW`, `DestroyAcceleratorTable` and
//! `TranslateAcceleratorW` (`specs/windows/user32.dll/LoadAcceleratorsW.yaml`,
//! `TranslateAcceleratorW.yaml`). The tables are the process's; a handle is a
//! number that is never a valid pointer.

use super::{H, SpinLock, info, set_last_error};
use alloc::vec::Vec;
use poly_win32k::dlg::{self, Accel, Mods};
use poly_win32k::user::CMsg;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn FindResourceW(module: usize, name: *const u16, ty: *const u16) -> usize;
    fn LoadResource(module: usize, res: usize) -> usize;
    fn LockResource(data: usize) -> *const u8;
    fn SizeofResource(module: usize, res: usize) -> u32;
}

const ERROR_INVALID_ACCEL_HANDLE: u32 = 1403;
const ERROR_INVALID_PARAMETER: u32 = 87;
const RT_ACCELERATOR: usize = 9;
const WM_COMMAND: u32 = 0x0111;
const WM_SYSCOMMAND: u32 = 0x0112;

struct Table {
    handle: usize,
    entries: Vec<Accel>,
    /// The resource it was loaded from (0: made by `CreateAcceleratorTableW`).
    resource: usize,
}

static TABLES: SpinLock<(usize, Vec<Table>)> = SpinLock::new((0x0007_A000, Vec::new()));

fn add(entries: Vec<Accel>, resource: usize) -> usize {
    TABLES.with(|t| {
        if resource != 0
            && let Some(x) = t.1.iter().find(|x| x.resource == resource)
        {
            return x.handle;
        }
        t.0 += 8;
        let handle = t.0;
        t.1.push(Table { handle, entries, resource });
        handle
    })
}

fn entries_of(h: usize) -> Option<Vec<Accel>> {
    TABLES.with(|t| t.1.iter().find(|x| x.handle == h).map(|x| x.entries.clone()))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadAcceleratorsW(inst: usize, name: *const u16) -> usize {
    // SAFETY: arguments of the kernel32 resource functions.
    unsafe {
        let r = FindResourceW(inst, name, RT_ACCELERATOR as *const u16);
        if r == 0 {
            return 0;
        }
        let data = LoadResource(inst, r);
        let p = LockResource(data);
        if p.is_null() {
            return 0;
        }
        let bytes = core::slice::from_raw_parts(p, SizeofResource(inst, r) as usize);
        add(dlg::parse_accelerators(bytes), r)
    }
}

/// `CreateAcceleratorTableW(lpaccel, cAccel)`: `ACCEL` is 6 bytes (`fVirt`, one byte of
/// padding, `key`, `cmd`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateAcceleratorTableW(accel: *const u8, count: i32) -> usize {
    if count <= 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if accel.is_null() {
        return 0;
    }
    let mut v = Vec::new();
    for i in 0..count as usize {
        // SAFETY: the caller's array of `count` ACCEL.
        let (f, key, cmd) = unsafe {
            let p = accel.add(i * 6);
            (*p, (p.add(2) as *const u16).read_unaligned(), (p.add(4) as *const u16).read_unaligned())
        };
        v.push(Accel { flags: f & !dlg::ACCEL_LAST, key, cmd });
    }
    add(v, 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CopyAcceleratorTableW(table: usize, out: *mut u8, count: i32) -> i32 {
    let Some(e) = entries_of(table) else {
        set_last_error(ERROR_INVALID_ACCEL_HANDLE);
        return 0;
    };
    if out.is_null() {
        return e.len() as i32;
    }
    let n = e.len().min(count.max(0) as usize);
    for (i, a) in e.iter().take(n).enumerate() {
        // SAFETY: the caller's array of `count` ACCEL.
        unsafe {
            let p = out.add(i * 6);
            *p = a.flags;
            *p.add(1) = 0;
            (p.add(2) as *mut u16).write_unaligned(a.key);
            (p.add(4) as *mut u16).write_unaligned(a.cmd);
        }
    }
    n as i32
}

#[unsafe(no_mangle)]
pub extern "system" fn DestroyAcceleratorTable(table: usize) -> i32 {
    TABLES.with(|t| {
        match t.1.iter().position(|x| x.handle == table) {
            Some(i) => {
                // A loaded table lives as long as its module: destroying it changes nothing.
                if t.1[i].resource == 0 {
                    t.1.remove(i);
                }
                1
            }
            None => {
                set_last_error(ERROR_INVALID_ACCEL_HANDLE);
                0
            }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn TranslateAcceleratorW(hwnd: H, table: usize, msg: *const CMsg) -> i32 {
    if msg.is_null() || info(hwnd).is_none() {
        return 0;
    }
    let Some(entries) = entries_of(table) else { return 0 };
    // SAFETY: the caller's MSG.
    let m = unsafe { *msg };
    let down = |vk: i32| super::GetKeyState(vk) < 0;
    let mods = Mods { shift: down(0x10), ctrl: down(0x11), alt: down(0x12) };
    let Some(a) = dlg::match_accel(&entries, m.message, m.w as u16, mods) else { return 0 };
    let id = usize::from(a.cmd);
    if u32::from(a.cmd) >= 0xF000 {
        super::SendMessageW(hwnd, WM_SYSCOMMAND, id, 0);
    } else {
        super::SendMessageW(hwnd, WM_COMMAND, (1 << 16) | id, 0);
    }
    1
}
