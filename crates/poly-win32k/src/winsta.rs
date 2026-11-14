//! Window stations and desktops (ADR 0186): the names a session has, the answers of
//! `GetUserObjectInformationW` and the shape of `STARTUPINFO.lpDesktop`, as host-tested logic.
//! The objects themselves (handles, descriptors, lifetime) are the NT personality's; this
//! module only holds what can be decided from names and numbers.
//!
//! The rules are the sheets under `specs/windows/user32.dll/` (`CreateWindowStationW`,
//! `SetProcessWindowStation`, `CreateDesktopW`, `GetThreadDesktop`, `GetUserObjectInformationW`)
//! and Learn's "Window Stations", "Desktops", "Window Station Security and Access Rights",
//! "Desktop Security and Access Rights" with the constants of `winuser.h`.

use alloc::vec::Vec;

// Window station rights (`winuser.h`).
pub const WINSTA_ENUMDESKTOPS: u32 = 0x0001;
pub const WINSTA_READATTRIBUTES: u32 = 0x0002;
pub const WINSTA_ACCESSCLIPBOARD: u32 = 0x0004;
pub const WINSTA_CREATEDESKTOP: u32 = 0x0008;
pub const WINSTA_WRITEATTRIBUTES: u32 = 0x0010;
pub const WINSTA_ACCESSGLOBALATOMS: u32 = 0x0020;
pub const WINSTA_EXITWINDOWS: u32 = 0x0040;
pub const WINSTA_ENUMERATE: u32 = 0x0100;
pub const WINSTA_READSCREEN: u32 = 0x0200;
/// The specific rights of a window station (`WINSTA_ALL_ACCESS`, without the standard ones).
pub const WINSTA_ALL: u32 = 0x037F;

// Desktop rights (`winuser.h`).
pub const DESKTOP_READOBJECTS: u32 = 0x0001;
pub const DESKTOP_CREATEWINDOW: u32 = 0x0002;
pub const DESKTOP_CREATEMENU: u32 = 0x0004;
pub const DESKTOP_HOOKCONTROL: u32 = 0x0008;
pub const DESKTOP_JOURNALRECORD: u32 = 0x0010;
pub const DESKTOP_JOURNALPLAYBACK: u32 = 0x0020;
pub const DESKTOP_ENUMERATE: u32 = 0x0040;
pub const DESKTOP_WRITEOBJECTS: u32 = 0x0080;
pub const DESKTOP_SWITCHDESKTOP: u32 = 0x0100;
/// The specific rights of a desktop.
pub const DESKTOP_ALL: u32 = 0x01FF;

/// What a process started on a named station and desktop must be granted to run: every specific
/// right (the sheet: a Low or Untrusted token needs both objects to carry a descriptor with a
/// label at its level).
pub const START_STATION_RIGHTS: u32 = WINSTA_ALL;
pub const START_DESKTOP_RIGHTS: u32 = DESKTOP_ALL;

/// `UOI_*` (`winuser.h`).
pub const UOI_FLAGS: i32 = 1;
pub const UOI_NAME: i32 = 2;
pub const UOI_TYPE: i32 = 3;
pub const UOI_USER_SID: i32 = 4;
pub const UOI_HEAPSIZE: i32 = 5;
pub const UOI_IO: i32 = 6;

/// `DF_ALLOWOTHERACCOUNTHOOK`, `CWF_CREATE_ONLY` (`winuser.h`).
pub const DF_ALLOWOTHERACCOUNTHOOK: u32 = 1;
pub const CWF_CREATE_ONLY: u32 = 1;

/// The station and desktop of the interactive session.
pub const DEFAULT_STATION: &str = "WinSta0";
pub const DEFAULT_DESKTOP: &str = "Default";

/// Win32 errors (`winerror.h`).
pub const ERROR_INVALID_FUNCTION: u32 = 1;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_BUSY: u32 = 170;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
/// `STATUS_DLL_INIT_FAILED`: how a process that cannot attach to its desktop ends.
pub const STATUS_DLL_INIT_FAILED: u32 = 0xC000_0142;

/// The size of `USEROBJECTFLAGS` (`fInherit`, `fReserved`, `dwFlags`).
pub const USEROBJECTFLAGS_SIZE: usize = 12;

crate::wire_enum! {
    /// One operation on window stations and desktops (`user32` → NT personality).
    pub enum WinstaOp {
        /// `CreateWindowStationW` (`sd` is the address of the caller's security descriptor, 0 for none).
        1 => CreateStation { name: Vec<u16>, flags: u32, access: u32, sd: u64 },
        /// `OpenWindowStationW`.
        2 => OpenStation { name: Vec<u16>, inherit: bool, access: u32 },
        /// `CloseWindowStation`.
        3 => CloseStation { handle: u64 },
        /// `GetProcessWindowStation`.
        4 => ProcessStation {},
        /// `SetProcessWindowStation`.
        5 => SetProcessStation { handle: u64 },
        /// `CreateDesktopW` (`device` is whether the name of a device was given).
        6 => CreateDesktop { name: Vec<u16>, has_device: bool, flags: u32, access: u32, sd: u64 },
        /// `OpenDesktopW`.
        7 => OpenDesktop { name: Vec<u16>, flags: u32, inherit: bool, access: u32 },
        /// `CloseDesktop`.
        8 => CloseDesktop { handle: u64 },
        /// `GetThreadDesktop`.
        9 => ThreadDesktop { tid: u32 },
        /// `SetThreadDesktop`.
        10 => SetThreadDesktop { handle: u64 },
        /// `GetUserObjectInformationW`.
        11 => GetInfo { handle: u64, index: i32, capacity: u32 },
        /// `SetUserObjectInformationW`.
        12 => SetInfo { handle: u64, index: i32, data: Vec<u8> },
        /// `user32` attaches to the process's station and desktop (its `DllMain`).
        13 => Connect {},
    }
}

crate::wire_enum! {
    /// The answer to a [`WinstaOp`].
    pub enum WinstaRes {
        /// Done; the last error is left as it was.
        1 => Done {},
        /// A handle (the last error is left as it was).
        2 => Handle { handle: u64 },
        /// Failed with this last error.
        3 => Err { code: u32 },
        /// Failed with the last error left as it was (`NULL` from `GetThreadDesktop`).
        4 => Silent {},
        /// A handle, and the last error set to 0 (`CreateWindowStationW`).
        5 => HandleClear { handle: u64 },
        /// The bytes of `GetUserObjectInformationW` (and their size).
        6 => Info { data: Vec<u8> },
        /// `ERROR_INSUFFICIENT_BUFFER` and the size that was needed.
        7 => Short { needed: u32 },
    }
}

/// `STARTUPINFO.lpDesktop` as a process was given it: the station and the desktop it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopSpec<'a> {
    /// The window station, if the text names one (`station\desktop`).
    pub station: Option<&'a [u16]>,
    /// The desktop; empty when no name was given.
    pub desktop: &'a [u16],
}

/// Split `lpDesktop`: nothing (a NULL or empty string) names neither; `desktop` alone names
/// a desktop of the process's own station; `station\desktop` names both.
pub fn parse_spec(spec: &[u16]) -> DesktopSpec<'_> {
    let spec = match spec.iter().position(|&c| c == 0) {
        Some(n) => &spec[..n],
        None => spec,
    };
    match spec.iter().position(|&c| c == u16::from(b'\\')) {
        Some(i) => DesktopSpec { station: Some(&spec[..i]), desktop: &spec[i + 1..] },
        None => DesktopSpec { station: None, desktop: spec },
    }
}

/// Whether two names are the same (ASCII and simple Unicode case folding, as the object
/// namespace's names are compared).
pub fn same_name(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| fold(x) == fold(y))
}

fn fold(c: u16) -> u16 {
    match char::from_u32(u32::from(c)).map(|ch| ch.to_uppercase().next()) {
        Some(Some(u)) if (u as u32) < 0x1_0000 => u as u16,
        _ => c,
    }
}

/// The text as UTF-16.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Whether a window station name `CreateWindowStationW` takes is acceptable: a name with a
/// backslash in it is a path that does not exist (3), and nothing at all is refused with
/// `ERROR_ACCESS_DENIED` (5).
pub fn check_station_name(name: &[u16]) -> Result<(), u32> {
    if name.is_empty() {
        return Err(ERROR_ACCESS_DENIED);
    }
    if name.contains(&u16::from(b'\\')) {
        return Err(ERROR_PATH_NOT_FOUND);
    }
    Ok(())
}

/// The kind of object `GetUserObjectInformationW` was asked about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Station,
    Desktop,
}

impl Kind {
    /// `UOI_TYPE`'s text.
    pub fn type_name(self) -> &'static str {
        match self {
            Kind::Station => "WindowStation",
            Kind::Desktop => "Desktop",
        }
    }
}

/// The desktop heap in kilobytes `UOI_HEAPSIZE` reports (a desktop of the interactive session).
pub const HEAP_SIZE_KB: u32 = 3072;

/// What `GetUserObjectInformationW` says about an object, in the order the sheet's checks come: a
/// station needs `WINSTA_READATTRIBUTES` (error 5), an index that is not one of the six is 87, and
/// `UOI_HEAPSIZE` and `UOI_IO` are for desktops only (1). The bytes are the whole answer; the
/// caller compares their length with the buffer it was given (122 with the length needed).
pub fn object_info(kind: Kind, name: &[u16], flags: [u32; 3], granted: u32, index: i32) -> Result<Vec<u8>, u32> {
    if kind == Kind::Station && granted & WINSTA_READATTRIBUTES == 0 {
        return Err(ERROR_ACCESS_DENIED);
    }
    let text = |s: &[u16]| -> Vec<u8> { s.iter().chain(core::iter::once(&0)).flat_map(|c| c.to_le_bytes()).collect() };
    match index {
        UOI_FLAGS => Ok(flags.iter().flat_map(|f| f.to_le_bytes()).collect()),
        UOI_NAME => Ok(text(name)),
        UOI_TYPE => Ok(text(&wide(kind.type_name()))),
        UOI_USER_SID => Ok(Vec::new()),
        UOI_HEAPSIZE | UOI_IO if kind == Kind::Station => Err(ERROR_INVALID_FUNCTION),
        UOI_HEAPSIZE => Ok(HEAP_SIZE_KB.to_le_bytes().to_vec()),
        UOI_IO => Ok(0u32.to_le_bytes().to_vec()),
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

/// `SetUserObjectInformationW` on `flags`: only `UOI_FLAGS` can be set, with the 12 bytes of
/// `USEROBJECTFLAGS` (anything else is 87).
pub fn set_info(flags: &mut [u32; 3], index: i32, data: &[u8]) -> Result<(), u32> {
    if index != UOI_FLAGS || data.len() != USEROBJECTFLAGS_SIZE {
        return Err(ERROR_INVALID_PARAMETER);
    }
    for (f, c) in flags.iter_mut().zip(data.chunks_exact(4)) {
        *f = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    }
    Ok(())
}

/// The window stations and desktops of a session, by name. `O` is the object manager's id of
/// the object behind each; lifetime is the object manager's (an entry goes when its object
/// does, [`Registry::forget`]).
#[derive(Debug)]
pub struct Registry<O> {
    stations: Vec<Station<O>>,
}

/// One window station.
#[derive(Debug)]
pub struct Station<O> {
    pub object: O,
    /// The name as it was created (case kept).
    pub name: Vec<u16>,
    pub desktops: Vec<Desktop<O>>,
    /// `UOI_FLAGS`: `fInherit`, `fReserved`, `dwFlags`.
    pub flags: [u32; 3],
}

/// One desktop.
#[derive(Debug)]
pub struct Desktop<O> {
    pub object: O,
    pub name: Vec<u16>,
    pub flags: [u32; 3],
}

impl<O> Default for Registry<O> {
    fn default() -> Self {
        Registry { stations: Vec::new() }
    }
}

/// Where an object is in the registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Station(usize),
    Desktop(usize, usize),
}

impl<O: Copy + PartialEq> Registry<O> {
    pub fn new() -> Self {
        Registry { stations: Vec::new() }
    }

    pub fn station_by_name(&self, name: &[u16]) -> Option<&Station<O>> {
        self.stations.iter().find(|s| same_name(&s.name, name))
    }

    pub fn add_station(&mut self, object: O, name: &[u16]) {
        self.stations.push(Station { object, name: name.to_vec(), desktops: Vec::new(), flags: [0; 3] });
    }

    /// The desktop `name` of the station `station`.
    pub fn desktop_by_name(&self, station: O, name: &[u16]) -> Option<&Desktop<O>> {
        self.stations.iter().find(|s| s.object == station)?.desktops.iter().find(|d| same_name(&d.name, name))
    }

    pub fn add_desktop(&mut self, station: O, object: O, name: &[u16]) -> bool {
        let Some(s) = self.stations.iter_mut().find(|s| s.object == station) else { return false };
        s.desktops.push(Desktop { object, name: name.to_vec(), flags: [0; 3] });
        true
    }

    /// Where `object` is, if it is a station or a desktop.
    pub fn place(&self, object: O) -> Option<Place> {
        for (i, s) in self.stations.iter().enumerate() {
            if s.object == object {
                return Some(Place::Station(i));
            }
            if let Some(j) = s.desktops.iter().position(|d| d.object == object) {
                return Some(Place::Desktop(i, j));
            }
        }
        None
    }

    pub fn station(&self, i: usize) -> Option<&Station<O>> {
        self.stations.get(i)
    }

    pub fn station_mut(&mut self, i: usize) -> Option<&mut Station<O>> {
        self.stations.get_mut(i)
    }

    /// The station that holds desktop `object`.
    pub fn station_of_desktop(&self, object: O) -> Option<O> {
        match self.place(object)? {
            Place::Desktop(i, _) => Some(self.stations[i].object),
            Place::Station(_) => None,
        }
    }

    /// `object` was deleted: forget it (a station takes its desktops' entries with it).
    pub fn forget(&mut self, object: O) {
        self.stations.retain(|s| s.object != object);
        for s in &mut self.stations {
            s.desktops.retain(|d| d.object != object);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{decode, encode};

    #[test]
    fn lp_desktop_forms() {
        let s = |t: &str| wide(t);
        let a = s("polyws_child0\\polydesk_child0");
        assert_eq!(parse_spec(&a), DesktopSpec { station: Some(&a[..13]), desktop: &a[14..] });
        let b = s("polydesk_child0");
        assert_eq!(parse_spec(&b), DesktopSpec { station: None, desktop: &b[..] });
        assert_eq!(parse_spec(&[]), DesktopSpec { station: None, desktop: &[] });
        assert_eq!(parse_spec(&[0]), DesktopSpec { station: None, desktop: &[] });
        // A text with a NUL in it ends there.
        let c = [u16::from(b'a'), 0, u16::from(b'b')];
        assert_eq!(parse_spec(&c).desktop, &c[..1]);
    }

    #[test]
    fn station_names() {
        assert_eq!(check_station_name(&[]), Err(ERROR_ACCESS_DENIED));
        assert_eq!(check_station_name(&wide("pa\\b")), Err(ERROR_PATH_NOT_FOUND));
        assert_eq!(check_station_name(&wide("polyws_create")), Ok(()));
        assert_eq!(check_station_name(&wide(&"x".repeat(299))), Ok(()));
        assert!(same_name(&wide("polyws_create"), &wide("POLYWS_CREATE")));
        assert!(!same_name(&wide("a"), &wide("ab")));
    }

    #[test]
    fn user_object_information_of_a_station() {
        let n = wide("polyws_create");
        let name = object_info(Kind::Station, &n, [0; 3], WINSTA_ALL, UOI_NAME).unwrap();
        assert_eq!(name.len(), 28, "(13 + 1) * 2");
        let ty = object_info(Kind::Station, &n, [0; 3], WINSTA_ALL, UOI_TYPE).unwrap();
        assert_eq!(ty.len(), 28, "WindowStation and the NUL");
        assert_eq!(object_info(Kind::Station, &n, [0; 3], WINSTA_ALL, UOI_FLAGS).unwrap(), [0u8; 12]);
        assert_eq!(object_info(Kind::Station, &n, [0; 3], WINSTA_ALL, UOI_USER_SID).unwrap().len(), 0);
        assert_eq!(object_info(Kind::Station, &n, [0; 3], WINSTA_ALL, UOI_HEAPSIZE), Err(ERROR_INVALID_FUNCTION));
        assert_eq!(object_info(Kind::Station, &n, [0; 3], WINSTA_ALL, UOI_IO), Err(ERROR_INVALID_FUNCTION));
        assert_eq!(object_info(Kind::Station, &n, [0; 3], WINSTA_ALL, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(object_info(Kind::Station, &n, [0; 3], WINSTA_ALL, 99), Err(ERROR_INVALID_PARAMETER));
        // READATTRIBUTES decides for a station, whatever the index.
        assert_eq!(object_info(Kind::Station, &n, [0; 3], WINSTA_ENUMDESKTOPS, UOI_NAME), Err(ERROR_ACCESS_DENIED));
        assert!(object_info(Kind::Station, &n, [0; 3], WINSTA_READATTRIBUTES, UOI_NAME).is_ok());
        assert_eq!(object_info(Kind::Station, &n, [0; 3], 0, UOI_FLAGS), Err(ERROR_ACCESS_DENIED));
    }

    #[test]
    fn user_object_information_of_a_desktop() {
        let n = wide("polydesk_one");
        assert_eq!(object_info(Kind::Desktop, &n, [0; 3], DESKTOP_WRITEOBJECTS, UOI_NAME).unwrap().len(), 26);
        assert_eq!(object_info(Kind::Desktop, &n, [0; 3], DESKTOP_ENUMERATE, UOI_FLAGS).unwrap().len(), 12);
        assert_eq!(object_info(Kind::Desktop, &n, [0; 3], 0, UOI_TYPE).unwrap().len(), 16, "Desktop and the NUL");
        assert_eq!(object_info(Kind::Desktop, &n, [0; 3], 0, UOI_IO).unwrap(), [0, 0, 0, 0]);
        assert_eq!(object_info(Kind::Desktop, &n, [0; 3], 0, UOI_HEAPSIZE).unwrap().len(), 4);
        assert_eq!(object_info(Kind::Desktop, &n, [0; 3], 0, 99), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn flags_can_be_set_and_nothing_else() {
        let mut f = [0u32; 3];
        let mut d = [0u8; 12];
        d[0] = 1;
        assert_eq!(set_info(&mut f, UOI_FLAGS, &d), Ok(()));
        assert_eq!(f, [1, 0, 0]);
        assert_eq!(set_info(&mut f, UOI_NAME, &d), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set_info(&mut f, UOI_FLAGS, &d[..11]), Err(ERROR_INVALID_PARAMETER));
        let bytes = object_info(Kind::Station, &[], f, WINSTA_ALL, UOI_FLAGS).unwrap();
        assert_eq!(bytes[0], 1);
    }

    #[test]
    fn the_registry_finds_by_name_and_forgets_with_the_object() {
        let mut r = Registry::<u32>::new();
        assert!(r.is_empty());
        r.add_station(10, &wide("WinSta0"));
        r.add_station(11, &wide("polyws"));
        assert!(r.add_desktop(11, 20, &wide("polydesk")));
        assert!(!r.add_desktop(99, 21, &wide("x")));
        assert_eq!(r.station_by_name(&wide("POLYWS")).map(|s| s.object), Some(11));
        assert_eq!(r.desktop_by_name(11, &wide("PolyDesk")).map(|d| d.object), Some(20));
        assert!(r.desktop_by_name(10, &wide("polydesk")).is_none(), "a desktop belongs to its station");
        assert_eq!(r.place(20), Some(Place::Desktop(1, 0)));
        assert_eq!(r.station_of_desktop(20), Some(11));
        assert_eq!(r.station_of_desktop(11), None);
        r.forget(20);
        assert!(r.desktop_by_name(11, &wide("polydesk")).is_none());
        r.forget(11);
        assert!(r.station_by_name(&wide("polyws")).is_none());
        assert_eq!(r.place(10), Some(Place::Station(0)));
    }

    #[test]
    fn operations_round_trip() {
        for op in [
            WinstaOp::CreateStation { name: wide("a"), flags: 1, access: 0x37F, sd: 0x1000 },
            WinstaOp::CreateDesktop { name: wide("d"), has_device: false, flags: 0, access: 1, sd: 0 },
            WinstaOp::GetInfo { handle: 5, index: 2, capacity: 8 },
            WinstaOp::SetInfo { handle: 5, index: 1, data: alloc::vec![1; 12] },
            WinstaOp::Connect {},
        ] {
            assert_eq!(decode::<WinstaOp>(&encode(&op)), Some(op));
        }
        for res in [WinstaRes::Done {}, WinstaRes::Handle { handle: 4 }, WinstaRes::Short { needed: 26 }] {
            assert_eq!(decode::<WinstaRes>(&encode(&res)), Some(res));
        }
    }
}
