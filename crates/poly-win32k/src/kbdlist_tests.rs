//! The layout list against the visible tests' recorded results
//! (`specs/windows/user32.dll/{LoadKeyboardLayoutW,UnloadKeyboardLayout,ActivateKeyboardLayout}.tests`).

extern crate std;

use super::*;
use std::vec::Vec;

const ORACLE: u32 = 0x0407_2000;
const T: u32 = 100;

fn klid(s: &str) -> Option<u32> {
    parse_klid(&s.encode_utf16().collect::<Vec<_>>())
}

fn load(set: &mut LayoutSet, k: &str, flags: u32) -> u32 {
    let h = klid_to_hkl(klid(k), flags);
    set.load(h);
    h
}

#[test]
fn the_oracle_starts_with_three_layouts() {
    let set = LayoutSet::new(ORACLE);
    assert_eq!(set.list(), &[0x0807_0809, 0x0407_2000, 0x0407_0809]);
    assert_eq!(set.thread_hkl(T), ORACLE);
}

#[test]
fn hkl_values_per_the_sheet() {
    let h = |k: &str| klid_to_hkl(klid(k), KLF_NOTELLSHELL);
    assert_eq!(h("00000409"), 0x0409_0409);
    assert_eq!(h("00000407"), 0x0407_0407);
    assert_eq!(h("0000040C"), 0x040C_040C);
    assert_eq!(h("0000040c"), 0x040C_040C);
    assert_eq!(h("00000807"), 0x0807_0807);
    assert_eq!(h("00000809"), 0x0809_0809);
    assert_eq!(h("00000C0C"), 0x0C0C_0C0C);
    assert_eq!(h("0000100C"), 0x100C_100C);
    assert_eq!(h("00000804"), 0x0804_0804);
    assert_eq!(h("00010409"), 0xF002_0409);
    assert_eq!(h("00020409"), 0xF001_0409);
    assert_eq!(h("0001040C"), 0xF0CE_040C);
    // Unknown or malformed names are the US layout.
    for bad in ["00000fff", "409", "zzzzzzzz", "FFFFFFFF", "d0010809", "00002000"] {
        assert_eq!(h(bad), US, "{bad}");
    }
    // The substitutes of the user's installed layouts.
    let sub = |k: &str| klid_to_hkl(klid(k), KLF_SUBSTITUTE_OK | KLF_NOTELLSHELL);
    assert_eq!(sub("d0010809"), 0x0807_0809);
    assert_eq!(sub("00000809"), 0x0407_0809);
    assert_eq!(sub("00002000"), 0x0407_2000);
}

#[test]
fn names_follow_the_device_identifier() {
    let n = |h| std::string::String::from_utf8(hkl_to_klid(h).to_vec()).unwrap();
    assert_eq!(n(0x0409_0409), "00000409");
    assert_eq!(n(0x0807_0807), "00000807");
    assert_eq!(n(0x0807_0809), "00000807");
    assert_eq!(n(0x0407_0809), "00000407");
    assert_eq!(n(0x0407_2000), "00000407");
    assert_eq!(n(0x040C_040C), "0000040C");
    assert_eq!(n(0xF002_0409), "00010409");
}

#[test]
fn load_appends_and_unload_removes_as_lkl_basic_and_ukl_basic() {
    let mut set = LayoutSet::new(ORACLE);
    let us = load(&mut set, "00000409", KLF_NOTELLSHELL);
    assert_eq!(set.list(), &[0x0807_0809, 0x0407_2000, 0x0407_0809, 0x0409_0409]);
    // Loading it again changes nothing.
    assert_eq!(load(&mut set, "00000409", KLF_NOTELLSHELL), us);
    assert_eq!(set.list().len(), 4);
    assert_eq!(set.thread_hkl(T), ORACLE);
    let ch = load(&mut set, "00000807", KLF_NOTELLSHELL);
    assert_eq!(set.list().len(), 5);
    for odd in ["00000fff", "409", "zzzzzzzz", "FFFFFFFF", "0000040c"] {
        load(&mut set, odd, KLF_NOTELLSHELL);
    }
    assert_eq!(set.list().len(), 6);
    // Unloading what was loaded brings the list back (the US layout, loaded often, goes once).
    assert!(set.unload(T, 0x040C_040C).is_some());
    assert!(set.unload(T, ch).is_some());
    assert_eq!(set.unload(T, us), Some(None));
    assert_eq!(set.list(), &[0x0807_0809, 0x0407_2000, 0x0407_0809]);
    // Again, the thread's own layout, an installed one and a made-up one: not loaded.
    assert_eq!(set.unload(T, us), None);
    assert_eq!(set.unload(T, ORACLE), None);
    assert_eq!(set.unload(T, 0x0807_0809), None);
    assert_eq!(set.unload(T, 0x1234_5678), None);
}

#[test]
fn activate_walks_the_list_as_the_akl_sheet() {
    let mut set = LayoutSet::new(ORACLE);
    let us = load(&mut set, "00000409", KLF_NOTELLSHELL);
    let ch = load(&mut set, "00000807", KLF_NOTELLSHELL);
    // list: 08070809 04072000 04070809 04090409 08070807
    let a = set.activate(T, us, 0).unwrap();
    assert_eq!((a.previous, a.current, a.changed()), (ORACLE, us, true));
    assert!(!set.activate(T, us, 0).unwrap().changed());
    // Not loaded: nothing.
    assert_eq!(set.activate(T, 0x1234_5678, 0), None);
    assert_eq!(set.thread_hkl(T), us);
    // HKL_NEXT from US goes to the Swiss German one, then wraps to the first.
    assert_eq!(set.activate(T, HKL_NEXT, 0).unwrap().current, ch);
    assert_eq!(set.activate(T, HKL_NEXT, 0).unwrap().current, 0x0807_0809);
    assert_eq!(set.activate(T, HKL_NEXT, 0).unwrap().current, ORACLE);
    assert_eq!(set.activate(T, HKL_PREV, 0).unwrap().current, 0x0807_0809);
    assert_eq!(set.activate(T, HKL_PREV, 0).unwrap().current, ch);
    // KLF_REORDER moves it to the front.
    set.activate(T, us, KLF_REORDER).unwrap();
    assert_eq!(set.list(), &[us, 0x0807_0809, ORACLE, 0x0407_0809, ch]);
}

#[test]
fn layouts_are_per_thread_and_setforprocess_sets_the_rest() {
    let mut set = LayoutSet::new(ORACLE);
    let us = load(&mut set, "00000409", KLF_NOTELLSHELL);
    set.activate(1, us, 0).unwrap();
    assert_eq!(set.thread_hkl(1), us);
    assert_eq!(set.thread_hkl(2), ORACLE);
    let a = set.activate(1, ORACLE, KLF_SETFORPROCESS).unwrap();
    assert_eq!(a.previous, us);
    assert_eq!(set.thread_hkl(3), ORACLE);
    set.activate(2, us, KLF_SETFORPROCESS).unwrap();
    assert_eq!((set.thread_hkl(1), set.thread_hkl(3)), (us, us));
}

#[test]
fn unloading_the_active_layout_falls_back_to_the_first_of_the_list() {
    let mut set = LayoutSet::new(ORACLE);
    let us = load(&mut set, "00000409", KLF_NOTELLSHELL);
    set.activate(T, us, 0).unwrap();
    assert_eq!(set.unload(T, us), Some(Some(0x0807_0809)));
    assert_eq!(set.thread_hkl(T), 0x0807_0809);
    assert_eq!(set.list(), &[0x0807_0809, ORACLE, 0x0407_0809]);
    assert_eq!(set.unload(T, us), None);
}

#[test]
fn the_desktops_layout_replaces_the_middle_entry() {
    let mut set = LayoutSet::new(ORACLE);
    set.set_desktop(0x0807_0807);
    assert_eq!(set.list(), &[0x0807_0809, 0x0807_0807, 0x0407_0809]);
    assert_eq!(set.thread_hkl(T), 0x0807_0807);
    // A layout the thread chose stays.
    set.set_desktop(0x0409_0409);
    set.activate(T, 0x0407_0809, 0).unwrap();
    set.set_desktop(0x0807_0807);
    assert_eq!(set.thread_hkl(T), 0x0407_0809);
    assert_eq!(set.thread_hkl(T + 1), 0x0807_0807);
}
