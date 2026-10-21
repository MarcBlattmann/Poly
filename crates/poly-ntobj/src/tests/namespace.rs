use super::*;
use alloc::vec;
use std::string::String;

const CI: u32 = OBJ_CASE_INSENSITIVE;

#[test]
fn well_known_directories_and_links_exist() {
    let (mut om, _) = setup();
    assert_eq!(om.object_type(om.root()), Some(ObjectType::Directory));
    assert_eq!(name_of(&om, om.root()).as_deref(), Some("\\"));
    for path in crate::manager::WELL_KNOWN_DIRECTORIES {
        let id = lookup(&mut om, path).unwrap();
        assert_eq!(om.object_type(id), Some(ObjectType::Directory), "{path}");
        assert_eq!(name_of(&om, id).as_deref(), Some(path));
        assert_eq!(om.is_permanent(id), Some(true));
    }
    for (link, target) in crate::manager::WELL_KNOWN_LINKS {
        // Lookups follow links; the walk lands on the target.
        assert_eq!(lookup(&mut om, link), lookup(&mut om, target), "{link}");
    }
    let root: Vec<String> =
        om.directory_entries(om.root()).unwrap().into_iter().map(|(n, _)| String::from_utf16(&n).unwrap()).collect();
    for name in ["??", "BaseNamedObjects", "Device", "DosDevices", "GLOBAL??", "KernelObjects", "KnownDlls", "Sessions"]
    {
        assert!(root.iter().any(|n| n == name), "{name} in {root:?}");
    }
}

#[test]
fn create_then_open_by_name() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    assert_eq!(ev.status, STATUS_SUCCESS);
    assert_eq!(name_of(&om, ev.object).as_deref(), Some("\\BaseNamedObjects\\Ev"));
    let h = open(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    assert_ne!(h, ev.handle);
    assert_eq!(om.object_by_handle(&c, h, 0, Some(ObjectType::Event)), Ok(ev.object));
    assert_eq!(om.handle_count(ev.object), Some(2));
    // Another process opens the same object.
    let c2 = caller(P2, T2);
    let h2 = open(&mut om, &c2, "\\BaseNamedObjects\\Ev", 0).unwrap();
    assert_eq!(om.object_by_handle(&c2, h2, 0, None), Ok(ev.object));
}

#[test]
fn case_insensitive_lookup_needs_the_flag() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\MyÉvent", CI).unwrap();
    let h = open(&mut om, &c, "\\basenamedobjects\\MYéVENT", CI).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(ev.object));
    // Without the flag the comparison is exact (OBJECT_ATTRIBUTES on Learn:
    // "default system settings"; to be confirmed).
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\myévent", 0), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(open(&mut om, &c, "\\basenamedobjects\\MyÉvent", 0), Err(STATUS_OBJECT_PATH_NOT_FOUND));
    assert!(open(&mut om, &c, "\\BaseNamedObjects\\MyÉvent", 0).is_ok());
}

#[test]
fn case_sensitive_creates_can_share_a_folded_name() {
    let (mut om, c) = setup();
    let upper = event(&mut om, &c, "\\BaseNamedObjects\\Foo", 0).unwrap();
    let lower = event(&mut om, &c, "\\BaseNamedObjects\\foo", 0).unwrap();
    assert_ne!(upper.object, lower.object);
    // A case-insensitive create collides with either.
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\FOO", CI), Err(STATUS_OBJECT_NAME_COLLISION));
    // Exact opens find each; a case-insensitive open finds the older.
    let h = open(&mut om, &c, "\\BaseNamedObjects\\foo", 0).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(lower.object));
    let h = open(&mut om, &c, "\\BaseNamedObjects\\FOO", CI).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(upper.object));
}

#[test]
fn create_collides_or_opens_with_openif() {
    let (mut om, c) = setup();
    let name = w("\\BaseNamedObjects\\Ev");
    let attrs = ObjectAttributes::named(&name, 0);
    let first = om.nt_create_event(&c, &attrs, EVENT_ALL_ACCESS, EventType::Notification, false).unwrap();
    assert_eq!(
        om.nt_create_event(&c, &attrs, EVENT_ALL_ACCESS, EventType::Notification, true),
        Err(STATUS_OBJECT_NAME_COLLISION)
    );
    let openif = ObjectAttributes::named(&name, OBJ_OPENIF);
    let second = om.nt_create_event(&c, &openif, EVENT_ALL_ACCESS, EventType::Synchronization, true).unwrap();
    assert_eq!(second.status, STATUS_OBJECT_NAME_EXISTS);
    assert!(second.status.is_success());
    assert_eq!(second.object, first.object);
    assert_ne!(second.handle, first.handle);
    // The existing object's state is untouched by the new parameters.
    assert_eq!(om.event_info(first.object), Ok(EventInfo { kind: EventType::Notification, signaled: false }));
    // Another type under the same name.
    assert_eq!(om.nt_create_mutant(&c, &openif, MUTANT_ALL_ACCESS, false), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(om.nt_create_mutant(&c, &attrs, MUTANT_ALL_ACCESS, false), Err(STATUS_OBJECT_NAME_COLLISION));
    // Also for directories that already exist.
    let bno = w("\\BaseNamedObjects");
    assert_eq!(
        om.nt_create_directory_object(&c, &ObjectAttributes::named(&bno, 0), DIRECTORY_ALL_ACCESS),
        Err(STATUS_OBJECT_NAME_COLLISION)
    );
    let d = om.nt_create_directory_object(&c, &ObjectAttributes::named(&bno, OBJ_OPENIF), DIRECTORY_QUERY).unwrap();
    assert_eq!(d.status, STATUS_OBJECT_NAME_EXISTS);
}

#[test]
fn missing_names_and_paths() {
    let (mut om, c) = setup();
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\Nope", 0), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(open(&mut om, &c, "\\Nope\\Ev", 0), Err(STATUS_OBJECT_PATH_NOT_FOUND));
    assert_eq!(open(&mut om, &c, "\\Nope", 0), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(event(&mut om, &c, "\\Nope\\Ev", 0), Err(STATUS_OBJECT_PATH_NOT_FOUND));
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\A\\B", 0), Err(STATUS_OBJECT_PATH_NOT_FOUND));
}

#[test]
fn name_syntax_errors() {
    let (mut om, c) = setup();
    assert_eq!(event(&mut om, &c, "BaseNamedObjects\\Ev", 0), Err(STATUS_OBJECT_PATH_SYNTAX_BAD));
    assert_eq!(open(&mut om, &c, "Ev", 0), Err(STATUS_OBJECT_PATH_SYNTAX_BAD));
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\\\Ev", 0), Err(STATUS_OBJECT_NAME_INVALID));
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\", 0), Err(STATUS_OBJECT_NAME_INVALID));
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\", 0), Err(STATUS_OBJECT_NAME_INVALID));
    assert_eq!(open(&mut om, &c, "\\\\", 0), Err(STATUS_OBJECT_NAME_INVALID));
    // No name at all when opening.
    assert_eq!(
        om.nt_open_event(&c, &ObjectAttributes::unnamed(0), EVENT_ALL_ACCESS),
        Err(STATUS_OBJECT_PATH_SYNTAX_BAD)
    );
    // "\" alone is the root directory.
    let h = open(&mut om, &c, "\\", 0).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(om.root()));
    // Unknown attribute bits.
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0x8), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects", 0x2000), Err(STATUS_INVALID_PARAMETER));
}

#[test]
fn objects_on_the_way_must_be_directories() {
    let (mut om, c) = setup();
    event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\Ev\\Sub", 0), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\Ev\\Sub", 0), Err(STATUS_OBJECT_TYPE_MISMATCH));
}

#[test]
fn open_checks_the_type() {
    let (mut om, c) = setup();
    let name = w("\\BaseNamedObjects\\M");
    let attrs = ObjectAttributes::named(&name, 0);
    om.nt_create_mutant(&c, &attrs, MUTANT_ALL_ACCESS, false).unwrap();
    assert_eq!(om.nt_open_event(&c, &attrs, EVENT_ALL_ACCESS), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert_eq!(om.nt_open_semaphore(&c, &attrs, SEMAPHORE_ALL_ACCESS), Err(STATUS_OBJECT_TYPE_MISMATCH));
    assert!(om.nt_open_mutant(&c, &attrs, MUTANT_ALL_ACCESS).is_ok());
    let dir = w("\\BaseNamedObjects");
    assert!(om.nt_open_directory_object(&c, &ObjectAttributes::named(&dir, 0), DIRECTORY_QUERY).is_ok());
    assert_eq!(
        om.nt_open_timer(&c, &ObjectAttributes::named(&dir, 0), TIMER_ALL_ACCESS),
        Err(STATUS_OBJECT_TYPE_MISMATCH)
    );
}

#[test]
fn names_relative_to_a_root_directory() {
    let (mut om, c) = setup();
    let bno = w("\\BaseNamedObjects");
    let dir = om.nt_open_directory_object(&c, &ObjectAttributes::named(&bno, 0), DIRECTORY_ALL_ACCESS).unwrap();
    let rel = w("Rel");
    let ev = om
        .nt_create_event(
            &c,
            &ObjectAttributes::relative(dir, &rel, 0),
            EVENT_ALL_ACCESS,
            EventType::Notification,
            false,
        )
        .unwrap();
    assert_eq!(name_of(&om, ev.object).as_deref(), Some("\\BaseNamedObjects\\Rel"));
    let h = om.nt_open_event(&c, &ObjectAttributes::relative(dir, &rel, 0), EVENT_QUERY_STATE).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(ev.object));
    // An empty name opens the root directory itself.
    let h = om.open_object(&c, &ObjectAttributes::relative(dir, &[], 0), 0, None, &mut NoParse).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, Some(ObjectType::Directory)), lookup(&mut om, "\\BaseNamedObjects"));
    // ...but cannot name a new object.
    assert_eq!(
        om.nt_create_event(&c, &ObjectAttributes::relative(dir, &[], 0), 0, EventType::Notification, false),
        Err(STATUS_OBJECT_NAME_INVALID)
    );
    // A relative name may not start with "\".
    let abs = w("\\Rel");
    assert_eq!(om.nt_open_event(&c, &ObjectAttributes::relative(dir, &abs, 0), 0), Err(STATUS_OBJECT_PATH_SYNTAX_BAD));
    // The root must be a directory, and a valid handle.
    assert_eq!(
        om.nt_open_event(&c, &ObjectAttributes::relative(ev.handle, &rel, 0), 0),
        Err(STATUS_OBJECT_TYPE_MISMATCH)
    );
    assert_eq!(
        om.nt_open_event(&c, &ObjectAttributes::relative(Handle(0x400), &rel, 0), 0),
        Err(STATUS_INVALID_HANDLE)
    );
}

#[test]
fn symbolic_links_on_the_way_are_followed() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    let link = w("\\??\\Objs");
    let target = w("\\BaseNamedObjects");
    let l = om
        .nt_create_symbolic_link_object(&c, &ObjectAttributes::named(&link, 0), SYMBOLIC_LINK_ALL_ACCESS, &target)
        .unwrap();
    for path in
        ["\\??\\Objs\\Ev", "\\DosDevices\\Objs\\Ev", "\\BaseNamedObjects\\Global\\Ev", "\\BaseNamedObjects\\Local\\Ev"]
    {
        let h = open(&mut om, &c, path, 0).unwrap();
        assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(ev.object), "{path}");
    }
    // Creating through a link puts the object in the target directory.
    let made = event(&mut om, &c, "\\??\\Objs\\Made", 0).unwrap();
    assert_eq!(name_of(&om, made.object).as_deref(), Some("\\BaseNamedObjects\\Made"));
    assert_eq!(om.nt_query_symbolic_link_object(&c, l.handle).map(|t| t == target), Ok(true));
}

#[test]
fn symbolic_link_as_the_last_component() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();
    let link = w("\\BaseNamedObjects\\EvLink");
    let target = w("\\BaseNamedObjects\\Ev");
    let l = om
        .nt_create_symbolic_link_object(&c, &ObjectAttributes::named(&link, 0), SYMBOLIC_LINK_QUERY, &target)
        .unwrap();
    // Opening an event follows the link.
    let h = om.nt_open_event(&c, &ObjectAttributes::named(&link, 0), EVENT_QUERY_STATE).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(ev.object));
    // Opening a link opens the link.
    let h = om.nt_open_symbolic_link_object(&c, &ObjectAttributes::named(&link, 0), SYMBOLIC_LINK_QUERY).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(l.object));
    // So does OBJ_OPENLINK with no expected type.
    let h = open(&mut om, &c, "\\BaseNamedObjects\\EvLink", OBJ_OPENLINK).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(l.object));
    // Creating over the link's name collides with the link, even with OPENIF.
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\EvLink", 0), Err(STATUS_OBJECT_NAME_COLLISION));
    assert_eq!(event(&mut om, &c, "\\BaseNamedObjects\\EvLink", OBJ_OPENIF), Err(STATUS_OBJECT_TYPE_MISMATCH));
    // Querying needs SYMBOLIC_LINK_QUERY.
    let h = om.nt_open_symbolic_link_object(&c, &ObjectAttributes::named(&link, 0), DELETE).unwrap();
    assert_eq!(om.nt_query_symbolic_link_object(&c, h), Err(STATUS_ACCESS_DENIED));
}

#[test]
fn dangling_and_relative_link_targets() {
    let (mut om, c) = setup();
    let make = |om: &mut ObjectManager, name: &str, target: &str| {
        let (n, t) = (w(name), w(target));
        om.nt_create_symbolic_link_object(&c, &ObjectAttributes::named(&n, 0), SYMBOLIC_LINK_ALL_ACCESS, &t).unwrap()
    };
    make(&mut om, "\\??\\Dangling", "\\BaseNamedObjects\\Nothing");
    make(&mut om, "\\??\\Relative", "BaseNamedObjects");
    assert_eq!(open(&mut om, &c, "\\??\\Dangling", 0), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(open(&mut om, &c, "\\??\\Dangling\\X", 0), Err(STATUS_OBJECT_PATH_NOT_FOUND));
    assert_eq!(open(&mut om, &c, "\\??\\Relative\\X", 0), Err(STATUS_OBJECT_PATH_SYNTAX_BAD));
}

#[test]
fn reparse_limit_and_dont_reparse() {
    let (mut om, c) = setup();
    let link = |om: &mut ObjectManager, name: &str, target: &str| {
        let (n, t) = (w(name), w(target));
        om.nt_create_symbolic_link_object(&c, &ObjectAttributes::named(&n, 0), SYMBOLIC_LINK_ALL_ACCESS, &t).unwrap();
    };
    link(&mut om, "\\A", "\\B");
    link(&mut om, "\\B", "\\A");
    assert_eq!(open(&mut om, &c, "\\A", 0), Err(STATUS_REPARSE_POINT_NOT_RESOLVED));
    assert_eq!(open(&mut om, &c, "\\A\\X", 0), Err(STATUS_REPARSE_POINT_NOT_RESOLVED));
    // A chain of exactly MAX_REPARSE links resolves; one more does not.
    let n = crate::namespace::MAX_REPARSE as usize;
    for i in 0..n - 1 {
        link(&mut om, &std::format!("\\L{i}"), &std::format!("\\L{}", i + 1));
    }
    link(&mut om, &std::format!("\\L{}", n - 1), "\\BaseNamedObjects");
    link(&mut om, "\\Lpre", "\\L0");
    let h = open(&mut om, &c, "\\L0", 0).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), lookup(&mut om, "\\BaseNamedObjects"));
    assert_eq!(open(&mut om, &c, "\\Lpre", 0), Err(STATUS_REPARSE_POINT_NOT_RESOLVED));
    // OBJ_DONT_REPARSE refuses any link.
    assert_eq!(open(&mut om, &c, "\\DosDevices", OBJ_DONT_REPARSE), Err(STATUS_REPARSE_POINT_ENCOUNTERED));
    assert!(open(&mut om, &c, "\\DosDevices", OBJ_DONT_REPARSE | OBJ_OPENLINK).is_ok());
}

#[test]
fn query_directory_lists_names_and_types() {
    let (mut om, c) = setup();
    let dir = w("\\BaseNamedObjects\\Mine");
    let d = om.nt_create_directory_object(&c, &ObjectAttributes::named(&dir, 0), DIRECTORY_ALL_ACCESS).unwrap();
    event(&mut om, &c, "\\BaseNamedObjects\\Mine\\b", 0).unwrap();
    let m = w("\\BaseNamedObjects\\Mine\\A");
    om.nt_create_mutant(&c, &ObjectAttributes::named(&m, 0), MUTANT_ALL_ACCESS, false).unwrap();
    let entries = om.nt_query_directory_object(&c, d.handle).unwrap();
    assert_eq!(entries, vec![(w("A"), "Mutant"), (w("b"), "Event")]);
    let ro = om.nt_open_directory_object(&c, &ObjectAttributes::named(&dir, 0), DIRECTORY_TRAVERSE).unwrap();
    assert_eq!(om.nt_query_directory_object(&c, ro), Err(STATUS_ACCESS_DENIED));
}

/// A device that owns its names: records what it was asked and answers
/// with a file object, a reparse or a failure depending on the name.
#[derive(Debug, Default)]
struct Disk {
    calls: Vec<(Vec<u16>, Intent, Option<ObjectType>)>,
}

impl ParseHook for Disk {
    fn parse(&mut self, om: &mut ObjectManager, r: &ParseRequest<'_>) -> Parsed {
        assert_eq!(r.kind, ObjectType::Device);
        assert_eq!(r.cookie, 7);
        self.calls.push((r.remaining.to_vec(), r.intent, r.object_type));
        let rest = String::from_utf16(r.remaining).unwrap();
        match rest.as_str() {
            "\\fail" => Parsed::Fail(STATUS_ACCESS_DENIED),
            "\\elsewhere" => Parsed::Reparse(w("\\BaseNamedObjects\\Ev")),
            "\\loop" => Parsed::Reparse(w("\\Device\\Disk0\\loop")),
            _ => {
                let file =
                    NewObject::Opaque { kind: ObjectType::File, cookie: 100, signaled: Some(false), parse: false };
                Parsed::Object(om.new_object(file, ThreadRef(0)).unwrap())
            }
        }
    }
}

#[test]
fn parse_hook_owns_the_names_below_a_device() {
    let (mut om, c) = setup();
    let k = Caller::kernel(P1, T1);
    let dev = w("\\Device\\Disk0");
    let device = NewObject::Opaque { kind: ObjectType::Device, cookie: 7, signaled: None, parse: true };
    om.create_object(&k, &ObjectAttributes::named(&dev, OBJ_PERMANENT), 0, device, &mut NoParse).unwrap();
    let link = w("\\??\\C:");
    om.nt_create_symbolic_link_object(&c, &ObjectAttributes::named(&link, 0), SYMBOLIC_LINK_ALL_ACCESS, &dev).unwrap();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ev", 0).unwrap();

    let mut disk = Disk::default();
    let path = w("\\??\\C:\\dir\\file.txt");
    let h = om.open_object(&c, &ObjectAttributes::named(&path, CI), FILE_GENERIC_READ, None, &mut disk).unwrap();
    let file = om.object_by_handle(&c, h, FILE_GENERIC_READ, Some(ObjectType::File)).unwrap();
    assert_eq!(disk.calls, vec![(w("\\dir\\file.txt"), Intent::Open, None)]);
    // The file holds the handle's reference only.
    assert_eq!((om.handle_count(file), om.pointer_count(file)), (Some(1), Some(1)));
    assert_eq!(om.object_name(file), None);

    // The device itself, with nothing after it.
    let h = om.open_object(&c, &ObjectAttributes::named(&dev, 0), 0, None, &mut disk).unwrap();
    assert_eq!(om.nt_query_object_type(&c, h), Ok("File"));
    assert_eq!(disk.calls.last(), Some(&(vec![], Intent::Open, None)));

    // Reparse, failure, endless reparse.
    let path = w("\\Device\\Disk0\\elsewhere");
    let h = om.open_object(&c, &ObjectAttributes::named(&path, 0), 0, None, &mut disk).unwrap();
    assert_eq!(om.object_by_handle(&c, h, 0, None), Ok(ev.object));
    let path = w("\\Device\\Disk0\\fail");
    assert_eq!(om.open_object(&c, &ObjectAttributes::named(&path, 0), 0, None, &mut disk), Err(STATUS_ACCESS_DENIED));
    let path = w("\\Device\\Disk0\\loop");
    assert_eq!(
        om.open_object(&c, &ObjectAttributes::named(&path, 0), 0, None, &mut disk),
        Err(STATUS_REPARSE_POINT_NOT_RESOLVED)
    );
    // Opening a file as an event: the hook's object has the wrong type.
    let path = w("\\Device\\Disk0\\x");
    assert_eq!(
        om.open_object(&c, &ObjectAttributes::named(&path, 0), 0, Some(ObjectType::Event), &mut disk),
        Err(STATUS_OBJECT_TYPE_MISMATCH)
    );
    assert_eq!(disk.calls.last(), Some(&(w("\\x"), Intent::Open, Some(ObjectType::Event))));
    // A caller without parse procedures cannot get past the device.
    assert_eq!(
        om.open_object(&c, &ObjectAttributes::named(&path, 0), 0, None, &mut NoParse),
        Err(STATUS_OBJECT_PATH_NOT_FOUND)
    );

    // Creating below the device asks the hook with the create intent.
    let file = NewObject::Opaque { kind: ObjectType::File, cookie: 1, signaled: Some(false), parse: false };
    let path = w("\\Device\\Disk0\\new.txt");
    let made = om.create_object(&c, &ObjectAttributes::named(&path, 0), FILE_ALL_ACCESS, file, &mut disk).unwrap();
    assert_eq!(made.status, STATUS_SUCCESS);
    assert_eq!(disk.calls.last(), Some(&(w("\\new.txt"), Intent::Create, Some(ObjectType::File))));
    // An event cannot be made there.
    let ev_new = NewObject::Event { kind: EventType::Notification, signaled: false };
    assert_eq!(
        om.create_object(&c, &ObjectAttributes::named(&path, 0), 0, ev_new, &mut disk),
        Err(STATUS_OBJECT_TYPE_MISMATCH)
    );
}

#[test]
fn parse_hook_on_the_root_directory_handle() {
    let (mut om, c) = setup();
    let k = Caller::kernel(P1, T1);
    let dev = w("\\Device\\Disk0");
    let device = NewObject::Opaque { kind: ObjectType::Device, cookie: 7, signaled: None, parse: true };
    let d = om.create_object(&k, &ObjectAttributes::named(&dev, 0), 0, device, &mut NoParse).unwrap();
    // The kernel's handle is in P1's table (no OBJ_KERNEL_HANDLE).
    let mut disk = Disk::default();
    let rel = w("sub\\file");
    om.open_object(&c, &ObjectAttributes::relative(d.handle, &rel, 0), 0, None, &mut disk).unwrap();
    assert_eq!(disk.calls, vec![(w("sub\\file"), Intent::Open, None)]);
}

#[test]
fn unicode_names_round_trip() {
    let (mut om, c) = setup();
    let ev = event(&mut om, &c, "\\BaseNamedObjects\\Ж-中-\u{10428}", CI).unwrap();
    assert_eq!(name_of(&om, ev.object).as_deref(), Some("\\BaseNamedObjects\\Ж-中-\u{10428}"));
    assert!(open(&mut om, &c, "\\BaseNamedObjects\\ж-中-\u{10428}", CI).is_ok());
    assert_eq!(open(&mut om, &c, "\\BaseNamedObjects\\ж-中-\u{10400}", CI), Err(STATUS_OBJECT_NAME_NOT_FOUND));
    let h = ev.handle;
    assert_eq!(om.nt_query_object_name(&c, h), Ok(w("\\BaseNamedObjects\\Ж-中-\u{10428}")));
}
