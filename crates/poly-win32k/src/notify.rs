//! Notification registrations of `user32`: device (`RegisterDeviceNotificationW`), power setting
//! (`RegisterPowerSettingNotification`) and suspend/resume (`RegisterSuspendResumeNotification`),
//! and the pointer-device registrations (`RegisterPointerDeviceNotifications`). Sheets:
//! `specs/windows/user32.dll/{Register,Unregister}*Notification*.yaml`,
//! `RegisterPointerDeviceNotifications.yaml`, `GetPointerDevices.yaml`.
//!
//! Nothing ever arrives by itself on this machine (no device comes, no suspend happens, no pen or
//! touch device exists), except the one initial message of a power setting that has a value now.
//! A handle is a number that is never given out twice, so a released handle stays dead whatever is
//! registered afterwards, and the three kinds refuse each other's handles with their own error.

use alloc::vec::Vec;

/// The kind of a registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Device,
    Power,
    Suspend,
}

/// What is registered under a handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reg {
    pub kind: Kind,
    /// The recipient window (0: none, or one that does not exist).
    pub hwnd: u64,
}

/// The first handle value; each registration takes the next multiple of [`STEP`].
pub const FIRST: usize = 0x000A_0000;
pub const STEP: usize = 0x10;

pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INVALID_DATA: u32 = 13;
/// `ERROR_SERVICE_SPECIFIC_ERROR`: what `UnregisterDeviceNotification` says to a pointer that is
/// not a registration at all.
pub const ERROR_SERVICE_SPECIFIC_ERROR: u32 = 1066;
/// What `RegisterPowerSettingNotification` says to the service flag with a window handle.
pub const ERROR_SERVICE_NOT_ACTIVE: u32 = 1083;

/// `DEVICE_NOTIFY_SERVICE_HANDLE` (winuser.h).
pub const DEVICE_NOTIFY_SERVICE_HANDLE: u32 = 1;
/// `DBT_DEVTYP_DEVICEINTERFACE` (dbt.h).
pub const DBT_DEVTYP_DEVICEINTERFACE: u32 = 5;

/// An initial power-setting message that waits for its thread to look at its queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub handle: usize,
    pub hwnd: u64,
    /// The thread of the recipient window.
    pub tid: u32,
    pub guid: [u8; 16],
    pub value: u32,
}

/// The registrations of one process.
#[derive(Debug, Default)]
pub struct Registry {
    live: Vec<(usize, Reg)>,
    issued: usize,
    pending: Vec<Pending>,
    pointer: Vec<u64>,
}

impl Registry {
    pub const fn new() -> Registry {
        Registry { live: Vec::new(), issued: 0, pending: Vec::new(), pointer: Vec::new() }
    }

    /// Register: the new handle.
    pub fn add(&mut self, kind: Kind, hwnd: u64) -> usize {
        let h = FIRST + self.issued * STEP;
        self.issued += 1;
        self.live.push((h, Reg { kind, hwnd }));
        h
    }

    /// Whether `h` is a handle that was given out once (and may be gone since).
    fn was_issued(&self, h: usize) -> bool {
        h >= FIRST && (h - FIRST) % STEP == 0 && (h - FIRST) / STEP < self.issued
    }

    /// Release `h` with the function of `kind`. Another kind's handle, a released one and NULL fail
    /// with that function's error (6 for devices, 87 for the other two); a pointer that was never
    /// a handle is 1066 for devices (the other two crash on the oracle: 87 here).
    pub fn release(&mut self, kind: Kind, h: usize) -> Result<Reg, u32> {
        let at = self.live.iter().position(|&(x, r)| x == h && r.kind == kind);
        match at {
            Some(i) => {
                let (_, reg) = self.live.remove(i);
                self.pending.retain(|p| p.handle != h);
                Ok(reg)
            }
            None => Err(match kind {
                Kind::Device if h == 0 || self.was_issued(h) => ERROR_INVALID_HANDLE,
                Kind::Device => ERROR_SERVICE_SPECIFIC_ERROR,
                _ => ERROR_INVALID_PARAMETER,
            }),
        }
    }

    /// An initial power-setting message for `handle`, delivered when thread `tid` next looks.
    pub fn defer(&mut self, p: Pending) {
        self.pending.push(p);
    }

    /// The next initial message waiting for thread `tid` (taken off the list).
    pub fn take_for(&mut self, tid: u32) -> Option<Pending> {
        let i = self.pending.iter().position(|p| p.tid == tid)?;
        Some(self.pending.remove(i))
    }

    /// Whether anything waits for thread `tid`.
    pub fn waiting_for(&self, tid: u32) -> bool {
        self.pending.iter().any(|p| p.tid == tid)
    }

    /// Register window `hwnd` for pointer-device messages: `false` if it is registered already.
    /// Windows that no longer exist (`alive` says no) are forgotten first, so a new window that
    /// gets the handle value of a destroyed one registers again.
    pub fn add_pointer(&mut self, hwnd: u64, alive: impl Fn(u64) -> bool) -> bool {
        self.pointer.retain(|&w| alive(w));
        if self.pointer.contains(&hwnd) {
            return false;
        }
        self.pointer.push(hwnd);
        true
    }
}

/// A GUID in its memory layout (`GUID`: Data1, Data2, Data3 little-endian, Data4 as is).
pub const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}

/// `GUID_CONSOLE_DISPLAY_STATE` (Learn: "Power Setting GUIDs").
pub const GUID_CONSOLE_DISPLAY_STATE: [u8; 16] =
    guid(0x6FE6_9556, 0x704A, 0x47A0, [0x8F, 0x24, 0xC2, 0x8D, 0x93, 0x6F, 0xDA, 0x47]);
/// `GUID_MONITOR_POWER_ON`.
pub const GUID_MONITOR_POWER_ON: [u8; 16] =
    guid(0x0273_1015, 0x4510, 0x4526, [0x99, 0xE6, 0xE5, 0xA1, 0x7E, 0xBD, 0x1A, 0xEA]);
/// `GUID_SESSION_DISPLAY_STATUS`.
pub const GUID_SESSION_DISPLAY_STATUS: [u8; 16] =
    guid(0x2B84_C20E, 0xAD23, 0x4DDF, [0x93, 0xDB, 0x05, 0xFF, 0xBD, 0x7E, 0xFC, 0xA5]);
/// `GUID_SESSION_USER_PRESENCE`.
pub const GUID_SESSION_USER_PRESENCE: [u8; 16] =
    guid(0x3C0F_4548, 0xC03F, 0x4C4D, [0xB9, 0xF2, 0x23, 0x7E, 0xDE, 0x68, 0x63, 0x76]);
/// `GUID_ACDC_POWER_SOURCE`.
pub const GUID_ACDC_POWER_SOURCE: [u8; 16] =
    guid(0x5D3E_9A59, 0xE9D5, 0x4B00, [0xA6, 0xBD, 0xFF, 0x34, 0xFF, 0x51, 0x65, 0x48]);

/// The value a power setting has now (what the initial `PBT_POWERSETTINGCHANGE` carries): a
/// display that is on (1), a user that is present (0), the machine on AC power (0); `None`
/// for settings with no value here (no battery) and for GUIDs that are no power setting.
pub fn power_value(guid: &[u8; 16]) -> Option<u32> {
    if *guid == GUID_CONSOLE_DISPLAY_STATE || *guid == GUID_MONITOR_POWER_ON || *guid == GUID_SESSION_DISPLAY_STATUS {
        Some(1)
    } else if *guid == GUID_SESSION_USER_PRESENCE || *guid == GUID_ACDC_POWER_SOURCE {
        Some(0)
    } else {
        None
    }
}

/// What `RegisterDeviceNotificationW` says to a filter (`size`, `device_type` from its header; the
/// filter is not NULL) and flags: `Ok` or the last error. A type other than the device interface
/// is `ERROR_INVALID_DATA`; the service flag with a window handle is `ERROR_INVALID_HANDLE`.
pub fn check_device_filter(device_type: u32, flags: u32) -> Result<(), u32> {
    if flags & DEVICE_NOTIFY_SERVICE_HANDLE != 0 {
        return Err(ERROR_INVALID_HANDLE);
    }
    if device_type != DBT_DEVTYP_DEVICEINTERFACE {
        return Err(ERROR_INVALID_DATA);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unregister*Notification.yaml: a handle is dead after its release and stays dead; the
    /// kinds refuse each other with their own error and keep the handle valid.
    #[test]
    fn handles_die_once_and_kinds_refuse_each_other() {
        let mut r = Registry::new();
        let d = r.add(Kind::Device, 1);
        let p = r.add(Kind::Power, 1);
        let s = r.add(Kind::Suspend, 1);
        assert!(d != p && p != s);
        assert_eq!(r.release(Kind::Device, p), Err(6));
        assert_eq!(r.release(Kind::Power, d), Err(87));
        assert_eq!(r.release(Kind::Suspend, p), Err(87));
        assert_eq!(r.release(Kind::Device, s), Err(6));
        assert_eq!(r.release(Kind::Device, d), Ok(Reg { kind: Kind::Device, hwnd: 1 }));
        assert_eq!(r.release(Kind::Device, d), Err(6));
        assert_eq!(r.release(Kind::Power, p).map(|x| x.kind), Ok(Kind::Power));
        assert_eq!(r.release(Kind::Power, p), Err(87));
        assert_eq!(r.release(Kind::Suspend, 0), Err(87));
        assert_eq!(r.release(Kind::Device, 0), Err(6));
        // A new registration does not revive a dead handle.
        let n = r.add(Kind::Power, 2);
        assert_ne!(n, p);
        assert_eq!(r.release(Kind::Power, p), Err(87));
        // A pointer that was never a handle.
        assert_eq!(r.release(Kind::Device, 0xDEAD_BEEF), Err(1066));
        assert_eq!(r.release(Kind::Device, FIRST + 1), Err(1066));
        assert_eq!(r.release(Kind::Suspend, s), Ok(Reg { kind: Kind::Suspend, hwnd: 1 }));
    }

    /// RegisterPowerSettingNotification.yaml `initial_message`: one message per registration, for
    /// the thread of the window, cancelled by the release.
    #[test]
    fn initial_messages_wait_for_their_thread_and_a_release_cancels_them() {
        let mut r = Registry::new();
        let a = r.add(Kind::Power, 10);
        let b = r.add(Kind::Power, 10);
        for (h, tid) in [(a, 7), (b, 7)] {
            r.defer(Pending { handle: h, hwnd: 10, tid, guid: GUID_MONITOR_POWER_ON, value: 1 });
        }
        assert!(r.waiting_for(7) && !r.waiting_for(8));
        assert_eq!(r.take_for(8), None);
        r.release(Kind::Power, a).unwrap();
        assert_eq!(r.take_for(7).map(|p| p.handle), Some(b));
        assert_eq!(r.take_for(7), None);
    }

    #[test]
    fn power_settings_with_a_value() {
        assert_eq!(power_value(&GUID_CONSOLE_DISPLAY_STATE), Some(1));
        assert_eq!(power_value(&GUID_MONITOR_POWER_ON), Some(1));
        assert_eq!(power_value(&GUID_SESSION_DISPLAY_STATUS), Some(1));
        assert_eq!(power_value(&GUID_SESSION_USER_PRESENCE), Some(0));
        assert_eq!(power_value(&GUID_ACDC_POWER_SOURCE), Some(0));
        // The USB device interface class is no power setting.
        let usb = guid(0xA5DC_BF10, 0x6530, 0x11D2, [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED]);
        assert_eq!(power_value(&usb), None);
        assert_eq!(GUID_ACDC_POWER_SOURCE[..4], [0x59, 0x9A, 0x3E, 0x5D]);
    }

    #[test]
    fn device_filters() {
        assert_eq!(check_device_filter(5, 0), Ok(()));
        assert_eq!(check_device_filter(5, 4), Ok(()));
        assert_eq!(check_device_filter(5, 0x80), Ok(()));
        for t in [0, 3, 99] {
            assert_eq!(check_device_filter(t, 0), Err(13));
        }
        assert_eq!(check_device_filter(5, 1), Err(6));
    }

    /// RegisterPointerDeviceNotifications.yaml: once per window; a new window with a dead one's
    /// handle value registers again.
    #[test]
    fn pointer_registrations_are_once_per_window() {
        let mut r = Registry::new();
        assert!(r.add_pointer(5, |_| true));
        assert!(!r.add_pointer(5, |_| true));
        assert!(r.add_pointer(6, |_| true));
        assert!(r.add_pointer(5, |w| w != 5), "5 died and came back");
        assert!(!r.add_pointer(5, |_| true));
    }
}
