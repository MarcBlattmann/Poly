use super::*;
use crate::defaults::*;
use crate::hive;
use crate::tree::Resolved;
use std::collections::BTreeMap;
use std::string::ToString;

fn no_files(_: &str) -> Option<Vec<u8>> {
    None
}

fn sz_value(r: &Registry, key: KeyId, name: &str) -> String {
    let v = r.value(key, &wide(name)).unwrap();
    assert!(v.kind == REG_SZ || v.kind == REG_EXPAND_SZ, "{name} is a string");
    let units: Vec<u16> = v.data.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert_eq!(units.last(), Some(&0), "{name} ends with a NUL");
    String::from_utf16(&units[..units.len() - 1]).unwrap()
}

#[test]
fn first_boot_makes_every_default() {
    let mut r = Registry::new();
    let outcomes = install(&mut r, T0, &mut no_files).unwrap();
    assert_eq!(outcomes, [Outcome::Created; 4]);
    assert_eq!(r.dirty_hives().len(), 4);
    let m = r.machine();
    let cv = r.find(m, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion").unwrap();
    assert_eq!(sz_value(&r, cv, "ProductName"), "Poly");
    assert_eq!(sz_value(&r, cv, "CurrentBuild"), BUILD);
    assert_eq!(r.value(cv, &wide("CurrentMajorVersionNumber")).unwrap().data, 10u32.to_le_bytes());
    assert_eq!(r.value(cv, &wide("InstallDate")).unwrap().data, 1_000_000_000u32.to_le_bytes());
    assert!(r.find(m, "SOFTWARE\\Classes").is_some());
    let env = r.find(m, "SYSTEM\\ControlSet001\\Control\\Session Manager\\Environment").unwrap();
    assert_eq!(sz_value(&r, env, "OS"), "Windows_NT");
    assert_eq!(r.value(env, &wide("Path")).unwrap().kind, REG_EXPAND_SZ);
    let u = r.user();
    assert!(r.find(u, ".DEFAULT\\Environment").is_some());
    let me = r.find(u, POLY_USER_SID).unwrap();
    let ve = r.child(me, &wide("Volatile Environment")).unwrap();
    assert!(r.key(ve).unwrap().is_volatile());
    assert_eq!(sz_value(&r, ve, "USERNAME"), POLY_USER_NAME);
    assert!(r.find(m, "HARDWARE\\DESCRIPTION\\System").is_some());
    assert!(r.key(r.find(m, "HARDWARE").unwrap()).unwrap().is_volatile());
}

#[test]
fn current_control_set_is_a_link() {
    let mut r = Registry::new();
    install(&mut r, T0, &mut no_files).unwrap();
    let m = r.machine();
    let system = r.find(m, "SYSTEM").unwrap();
    let ccs = r.child(system, &wide("currentcontrolset")).unwrap();
    assert!(r.key(ccs).unwrap().is_link() && r.key(ccs).unwrap().is_volatile());
    let Ok(Resolved::Reparse(p)) = r.resolve(system, &wide("CurrentControlSet\\Control\\ComputerName"), false) else {
        panic!("not a reparse")
    };
    assert_eq!(String::from_utf16(&p).unwrap(), "\\REGISTRY\\MACHINE\\SYSTEM\\ControlSet001\\Control\\ComputerName");
}

#[test]
fn saved_hives_load_on_the_next_boot() {
    let mut r = Registry::new();
    install(&mut r, T0, &mut no_files).unwrap();
    let m = r.machine();
    let sw = r.find(m, "SOFTWARE").unwrap();
    let mine = create(&mut r, sw, "PolyApp");
    r.set_value(mine, &wide("Setting"), REG_DWORD, &3u32.to_le_bytes(), T0).unwrap();
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for h in r.dirty_hives() {
        let name = r.hive(h).file.clone();
        files.insert(name, r.save(h));
    }
    assert_eq!(files.len(), 4);

    let mut r2 = Registry::new();
    let outcomes = install(&mut r2, T0 + 50, &mut |f: &str| files.get(f).cloned()).unwrap();
    assert_eq!(outcomes, [Outcome::Loaded; 4]);
    assert!(r2.dirty_hives().is_empty(), "loading and the volatile keys leave hives clean");
    let m2 = r2.machine();
    let mine2 = r2.find(m2, "SOFTWARE\\PolyApp").unwrap();
    assert_eq!(r2.value(mine2, &wide("setting")).unwrap().data, 3u32.to_le_bytes());
    // The volatile keys came back, made afresh.
    assert!(r2.find(m2, "SYSTEM\\CurrentControlSet").is_some());
    assert!(r2.find(r2.user(), &(POLY_USER_SID.to_string() + "\\Volatile Environment")).is_some());
}

#[test]
fn a_corrupt_file_falls_back_to_the_defaults() {
    let mut r = Registry::new();
    let outcomes = install(&mut r, T0, &mut |f: &str| (f == "SOFTWARE").then(|| alloc::vec![1, 2, 3])).unwrap();
    assert_eq!(outcomes[1], Outcome::Corrupt(STATUS_REGISTRY_CORRUPT));
    let m = r.machine();
    assert!(r.find(m, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion").is_some());
    let sw = r.hive_by_file("SOFTWARE").unwrap();
    assert!(r.hive(sw).dirty, "the next save replaces the bad file");
    assert!(hive::decode(&r.save(sw)).is_ok());
}

#[test]
fn every_default_string_is_terminated() {
    for spec in &HIVES {
        for key in spec.keys {
            for (name, data) in key.values {
                let b = data.bytes();
                match data.kind() {
                    REG_SZ | REG_EXPAND_SZ => assert_eq!(&b[b.len() - 2..], &[0, 0], "{name}"),
                    REG_DWORD => assert_eq!(b.len(), 4),
                    _ => {}
                }
            }
        }
    }
    assert_eq!(Data::MultiSz(&["a", "b"]).bytes(), [b'a', 0, 0, 0, b'b', 0, 0, 0, 0, 0]);
}
