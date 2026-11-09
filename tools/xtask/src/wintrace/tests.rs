//! Tests.

use super::*;

fn imp(dll: &str, sym: Sym) -> Import {
    Import { dll: dll.into(), sym, delay: false }
}
fn name(n: &str) -> Sym {
    Sym::Name(n.into())
}

#[test]
fn plans_one_proxy_per_dll_with_variables_forwarded() {
    let imports = [
        imp("KERNEL32.dll", name("CreateFileW")),
        imp("msvcrt.dll", name("_iob")),
        imp("msvcrt.dll", name("memcpy")),
        imp("KERNEL32.dll", name("ExitProcess")),
        imp("OLEAUT32.dll", Sym::Ordinal(2)),
        imp("msvcrt.dll", name("memcpy")),
        imp("msvcrt.dll", name("_CxxThrowException")),
        imp("msvcrt.dll", name("my_var")),
        imp("kernel32.dll", name("FooBar")),
    ];
    let p = plan(&imports, &["my_var".to_string()]).unwrap();
    assert_eq!(p.len(), 3);
    assert_eq!(
        (p[0].real.as_str(), p[0].file.as_str(), p[0].crate_name.as_str()),
        ("kernel32.dll", "pt_kernel32.dll", "pt_kernel32")
    );
    assert_eq!(p[0].funcs.len(), 3);
    assert_eq!(p[0].funcs[0].sig, "h=wiipiih");
    assert_eq!(p[0].funcs[1].flags, F_NORETURN);
    assert_eq!(p[0].funcs[2].sig, "", "an unknown function has no signature");
    assert_eq!(p[1].data, ["_iob", "my_var"]);
    assert_eq!(p[1].funcs.len(), 2, "duplicates are exported once");
    assert_eq!(p[1].funcs[1].flags, F_TAIL);
    // oleaut32's ordinal 2 is SysAllocString.
    assert_eq!(p[2].funcs[0].sig, "p=w");
    assert!(plan(&[imp("pt_kernel32.dll", name("X"))], &[]).is_err());
    assert!(plan(&[imp("x.drv", name("X"))], &[]).is_err());
    assert!(plan(&[imp("a.dll", name("bad\"name"))], &[]).is_err());
}

#[test]
fn generates_thunks_tables_and_exports() {
    let imports = [
        imp("msvcrt.dll", name("??1type_info@@UEAA@XZ")),
        imp("msvcrt.dll", name("_fmode")),
        imp("msvcrt.dll", name("exit")),
    ];
    let p = &plan(&imports, &[]).unwrap()[0];
    let lib = lib_rs(p, Path::new("/r/tools/wintrace/proxy/common.rs"));
    assert!(lib.contains("#[path = \"/r/tools/wintrace/proxy/common.rs\"]"));
    assert!(lib.contains("#[link(name = \"msvcrt\", kind = \"raw-dylib\")]"));
    assert!(lib.contains("#[link_name = \"??1type_info@@UEAA@XZ\"]\n    fn real_0();"));
    assert!(lib.contains("pt_f0:\n    .quad \"__imp_??1type_info@@UEAA@XZ\""));
    assert!(lib.contains(
        "pt_f1:\n    .quad \"__imp_exit\"\n    .quad pt_dll\n    .quad pt_n1\n    .quad pt_g1\n    .long 1\n"
    ));
    assert!(lib.contains("pt_g1:\n    .asciz \"v=i\""));
    assert!(lib.contains("pt_t1:\n    lea r10, [rip + pt_f1]\n    jmp pt_common"));
    // The raw string survives names with `#`.
    assert!(lib.contains("r##\"") && lib.ends_with("\"##\n);\n"));
    assert_eq!(
        link_args(p),
        [
            "/ENTRY:pt_proxy_main",
            "/EXPORT:??1type_info@@UEAA@XZ=pt_t0",
            "/EXPORT:exit=pt_t1",
            "/EXPORT:_fmode=msvcrt._fmode"
        ]
    );
    let o = &plan(&[imp("OLEAUT32.dll", Sym::Ordinal(9))], &[]).unwrap()[0];
    assert_eq!(link_args(o)[1], "/EXPORT:pt_ordinal_9=pt_t0,@9,NONAME");
    let lib = lib_rs(o, Path::new("c.rs"));
    assert!(lib.contains("#[link_ordinal(9)]\n    fn real_0();"));
    assert!(lib.contains(".quad \"__imp_real_0\""));
    assert!(lib.contains("pt_n0:\n    .asciz \"#9\""));
    let b = build_rs(o);
    assert!(b.contains("println!(\"cargo:rustc-link-arg=/EXPORT:pt_ordinal_9=pt_t0,@9,NONAME\");"));
}

#[test]
fn signatures_are_well_formed_and_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for (n, s) in SIGNATURES {
        assert!(seen.insert(*n), "{n} twice");
        let (ret, args) = s.split_once('=').unwrap_or_else(|| panic!("{n}: {s}"));
        assert!(ret.len() == 1 && "iqscphwav".contains(ret), "{n}: {s}");
        assert!(args.len() <= 20 && args.chars().all(|c| "iqscphwa".contains(c)), "{n}: {s}");
    }
    assert_eq!((stack_slots("h=wiipiih"), stack_slots("i="), stack_slots("")), (3, 0, 16));
    for n in NORETURN.iter().chain(TAIL).chain(FLUSH) {
        assert!(!DATA.contains(n), "{n}");
    }
}

/// A minimal PE32+ image: headers, `.rdata` with an import directory
/// for `dlls` (each importing `F` by name), optionally a signature.
fn image(dlls: &[&str], signed: bool) -> Vec<u8> {
    let mut b = vec![0u8; 0x400];
    b[0..2].copy_from_slice(b"MZ");
    b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    let fh = 0x84;
    b[fh..fh + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    b[fh + 2..fh + 4].copy_from_slice(&1u16.to_le_bytes());
    b[fh + 16..fh + 18].copy_from_slice(&240u16.to_le_bytes());
    b[fh + 18..fh + 20].copy_from_slice(&0x22u16.to_le_bytes());
    let opt = fh + 20;
    b[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
    b[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
    b[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    b[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
    b[opt + 40..opt + 42].copy_from_slice(&6u16.to_le_bytes());
    b[opt + 48..opt + 50].copy_from_slice(&6u16.to_le_bytes());
    b[opt + 56..opt + 60].copy_from_slice(&0x2000u32.to_le_bytes());
    b[opt + 60..opt + 64].copy_from_slice(&0x400u32.to_le_bytes());
    b[opt + 68..opt + 70].copy_from_slice(&3u16.to_le_bytes());
    b[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
    // .rdata at RVA 0x1000, file 0x400, 0x200 bytes.
    let mut sec = vec![0u8; 0x200];
    let n = dlls.len();
    let desc = 0usize;
    let mut at = 20 * (n + 1);
    for (i, dll) in dlls.iter().enumerate() {
        let ilt = at;
        sec[ilt..ilt + 8].copy_from_slice(&(0x1000 + ilt as u64 + 32).to_le_bytes());
        let iat = ilt + 16;
        sec[iat..iat + 8].copy_from_slice(&(0x1000 + ilt as u64 + 32).to_le_bytes());
        let hint = ilt + 32;
        sec[hint + 2] = b'F';
        let name = hint + 8;
        sec[name..name + dll.len()].copy_from_slice(dll.as_bytes());
        let d = desc + 20 * i;
        sec[d..d + 4].copy_from_slice(&(0x1000 + ilt as u32).to_le_bytes());
        sec[d + 12..d + 16].copy_from_slice(&(0x1000 + name as u32).to_le_bytes());
        sec[d + 16..d + 20].copy_from_slice(&(0x1000 + iat as u32).to_le_bytes());
        at = name + 24;
    }
    let dir = |b: &mut Vec<u8>, i: usize, rva: u32, size: u32| {
        b[opt + 112 + 8 * i..opt + 116 + 8 * i].copy_from_slice(&rva.to_le_bytes());
        b[opt + 116 + 8 * i..opt + 120 + 8 * i].copy_from_slice(&size.to_le_bytes());
    };
    dir(&mut b, 1, 0x1000, 20 * (n as u32 + 1));
    let t = opt + 240;
    b[t..t + 8].copy_from_slice(b".rdata\0\0");
    b[t + 8..t + 12].copy_from_slice(&0x200u32.to_le_bytes());
    b[t + 12..t + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    b[t + 16..t + 20].copy_from_slice(&0x200u32.to_le_bytes());
    b[t + 20..t + 24].copy_from_slice(&0x400u32.to_le_bytes());
    b[t + 36..t + 40].copy_from_slice(&0x4000_0040u32.to_le_bytes());
    b.extend_from_slice(&sec);
    if signed {
        let off = b.len() as u32;
        b.extend_from_slice(&[0xAA; 16]);
        dir(&mut b, 4, off, 16);
    }
    b
}

#[test]
fn renames_only_the_dll_names() {
    let orig = image(&["KERNEL32.dll", "msvcrt.dll", "keep.dll"], true);
    let before = crate::winimports::imports_of(&orig).unwrap();
    assert_eq!(before.len(), 3);
    let renamed =
        rename_imports(&orig, |d| (d != "keep.dll").then(|| format!("pt_{}", d.to_ascii_lowercase()))).unwrap();
    let after = crate::winimports::imports_of(&renamed).unwrap();
    let dlls: Vec<&str> = after.iter().map(|i| i.dll.as_str()).collect();
    assert_eq!(dlls, ["pt_kernel32.dll", "pt_msvcrt.dll", "keep.dll"]);
    assert!(after.iter().all(|i| i.sym == Sym::Name("F".into())));
    let pe = poly_pe::Pe::parse(&renamed).unwrap();
    let secs: Vec<_> = pe.sections().iter().collect();
    assert_eq!(secs.len(), 2);
    assert_eq!(secs[1].short_name(), b".ptname");
    assert_eq!((secs[1].virtual_address, secs[1].pointer_to_raw_data), (0x2000, 0x600));
    assert_eq!(pe.info().optional.size_of_image, 0x3000);
    // The signature is gone, the section's data is where it was, and
    // the original section is untouched but for the name fields.
    assert_eq!(renamed.len(), 0x800);
    assert_eq!(&renamed[0x400 + 0x80..0x400 + 0x200], &orig[0x400 + 0x80..0x400 + 0x200]);
    let ck = 0x84 + 20 + 64;
    let sum = u32::from_le_bytes(renamed[ck..ck + 4].try_into().unwrap());
    assert_eq!(sum, pe_checksum(&renamed, ck));
    assert_ne!(sum, 0);
    // Nothing to rename, or no room for a header.
    assert!(rename_imports(&orig, |_| None).is_err());
    let mut full = orig.clone();
    full[0x84 + 20 + 240 + 40] = 1;
    assert!(rename_imports(&full, |d| Some(d.into())).is_err());
}

#[test]
fn checksum_matches_a_hand_computed_one() {
    // Words 0x0201, 0x0403 (the field at 4..8 skipped), 0x0009; length 9.
    let b = [1, 2, 3, 4, 0xFF, 0xFF, 0xFF, 0xFF, 9];
    assert_eq!(pe_checksum(&b, 4), 0x0201 + 0x0403 + 0x0009 + 9);
    // The carry folds back in.
    let b = [0xFF, 0xFF, 0x02, 0x00];
    assert_eq!(pe_checksum(&b, 100), 0x0001 + 1 + 4);
}

const A: &str = r#"{"format":"polytrace/1"}
{"seq":1,"t":0,"dll":"kernel32.dll","fn":"GetFileAttributesW","args":[{"w":"C:\\w\\x.txt"}],"ret":4294967295,"err":2}
{"seq":0,"t":0,"dll":"kernel32.dll","fn":"GetTickCount","args":[],"ret":123}
{"seq":2,"t":0,"dll":"msvcrt.dll","fn":"malloc","args":[16],"ret":"0x1000"}
{"seq":3,"t":1,"dll":"kernel32.dll","fn":"CloseHandle","args":["0x8"],"ret":1}
{"seq":4,"t":0,"dll":"kernel32.dll","fn":"ExitProcess","args":[0],"ret":null}
{"seq":5,"t":0,"dll":"kernel32.dll","fn":"Cut"#;

const B: &str = r#"{"seq":10,"t":0,"dll":"kernel32.dll","fn":"GetTickCount","args":[],"ret":999}
{"seq":11,"t":0,"dll":"kernel32.dll","fn":"GetFileAttributesW","args":[{"w":"Z:\\7zip\\x.txt"}],"ret":32}
{"seq":12,"t":0,"dll":"msvcrt.dll","fn":"malloc","args":[16],"ret":"0x7f00"}
{"seq":13,"t":0,"dll":"msvcrt.dll","fn":"free","args":["0x7f00"],"ret":null}
{"seq":14,"t":0,"dll":"kernel32.dll","fn":"ExitProcess","args":[0],"ret":null}
{"seq":15,"t":1,"dll":"kernel32.dll","fn":"CloseHandle","args":["0x30"],"ret":1}
"#;

#[test]
fn parses_traces() {
    let a = parse_trace(A).unwrap();
    assert_eq!(a.len(), 5, "header skipped, cut-off last line ignored");
    assert_eq!(a[0].func, "GetFileAttributesW");
    assert_eq!(a[0].err, Some(2));
    assert_eq!(a[2].ret, Value::String("0x1000".into()));
    assert_eq!(a[4].ret, Value::Null);
    assert!(parse_trace("{bad\n{\"seq\":1}\n").is_err());
    assert_eq!(
        a[0].to_string(),
        "kernel32.dll!GetFileAttributesW({\"w\":\"C:\\\\w\\\\x.txt\"}) = 4294967295 [last error 2]  (seq 1)"
    );
}

#[test]
fn normalises_machine_facts_and_byte_arguments() {
    let t = "{\"seq\":0,\"t\":0,\"dll\":\"kernel32.dll\",\"fn\":\"IsProcessorFeaturePresent\",\"args\":[17],\"ret\":1}\n\
             {\"seq\":1,\"t\":0,\"dll\":\"kernel32.dll\",\"fn\":\"GetVersion\",\"args\":[],\"ret\":1717043210}\n\
             {\"seq\":2,\"t\":0,\"dll\":\"msvcrt.dll\",\"fn\":\"memset\",\"args\":[\"0x10\",4294775809,1],\"ret\":\"0x10\"}\n\
             {\"seq\":3,\"t\":0,\"dll\":\"kernel32.dll\",\"fn\":\"GetOEMCP\",\"args\":[],\"ret\":850}\n";
    let n = normalise(&parse_trace(t).unwrap());
    let funcs: Vec<&str> = n.iter().map(|c| c.func.as_str()).collect();
    assert_eq!(funcs, ["GetVersion", "memset", "GetOEMCP"]);
    assert_eq!(n[0].ret, Value::from("version 10.0"));
    assert_eq!(n[1].args[1], Value::from(1));
    // Another build, another register's junk, another code page: the same.
    let other = t.replace("1717043210", "1248133130").replace("4294775809", "1").replace("850", "437");
    let d = diff_traces(&n, &normalise(&parse_trace(&other).unwrap()), &[], &[]);
    assert!(d.divergences.is_empty(), "{d:?}");
    let c = canonical(&parse_trace(t).unwrap(), &[]);
    assert_eq!((c[2].ret.clone(), c[1].args[0].clone()), (Value::from("*"), Value::from("ptr")));
}

#[test]
fn diffs_align_by_thread_and_ignore_pointers() {
    let (a, b) = (parse_trace(A).unwrap(), parse_trace(B).unwrap());
    let map_a = [("C:\\w".to_string(), "<dir>".to_string())];
    let map_b = [("Z:\\7zip".to_string(), "<dir>".to_string())];
    let d = diff_traces(&a, &b, &map_a, &map_b);
    assert_eq!(d.per_dll["kernel32.dll"], (4, 4));
    assert_eq!(d.per_dll["msvcrt.dll"], (1, 2));
    // Thread 0: GetTickCount (volatile) and malloc match; the
    // attributes differ; free is only in B. Thread 1: the handles are
    // both "some value".
    assert_eq!(d.threads[&0], (4, 5, 3));
    assert_eq!(d.threads[&1], (1, 1, 1));
    assert_eq!(d.divergences.len(), 2);
    match &d.divergences[0] {
        Divergence::Differs { what, a, .. } => {
            assert_eq!(a.func, "GetFileAttributesW");
            assert_eq!(what, &["result 4294967295 vs 32", "last error 2 vs unchanged"]);
        }
        other => panic!("{other:?}"),
    }
    match &d.divergences[1] {
        Divergence::Extra { only_a, only_b, before, .. } => {
            assert!(only_a.is_empty());
            assert_eq!(only_b.len(), 1);
            assert_eq!(only_b[0].func, "free");
            assert_eq!(before.as_ref().unwrap().func, "malloc");
        }
        other => panic!("{other:?}"),
    }
    // Without the path mapping the string argument differs too.
    let d = diff_traces(&a, &b, &[], &[]);
    let Divergence::Differs { what, .. } = &d.divergences[0] else { panic!() };
    assert_eq!(what.len(), 3);
    let text = report(&d, "a", "b", 1);
    assert!(text.contains("A: a (5 calls)\nB: b (6 calls)"), "{text}");
    assert!(text.contains("1. thread 0: kernel32.dll!GetFileAttributesW: result"), "{text}");
    assert!(text.contains("... 1 more divergences"), "{text}");
    // Identical traces: nothing to report.
    assert!(diff_traces(&a, &a, &[], &[]).divergences.is_empty());
}

#[test]
fn diff_resynchronises_after_extra_calls() {
    let call = |seq, f: &str| Call {
        seq,
        t: 0,
        dll: "k.dll".into(),
        func: f.into(),
        args: vec![],
        ret: Value::Null,
        err: None,
    };
    let a: Vec<Call> = ["A", "B", "C", "D"].iter().enumerate().map(|(i, f)| call(i as u64, f)).collect();
    let b: Vec<Call> = ["A", "X", "Y", "C", "D", "E"].iter().enumerate().map(|(i, f)| call(i as u64, f)).collect();
    let d = diff_traces(&a, &b, &[], &[]);
    assert_eq!(d.threads[&0], (4, 6, 3));
    let kinds: Vec<(usize, usize)> = d
        .divergences
        .iter()
        .map(|x| match x {
            Divergence::Extra { only_a, only_b, .. } => (only_a.len(), only_b.len()),
            Divergence::Differs { .. } => (9, 9),
        })
        .collect();
    assert_eq!(kinds, [(1, 2), (0, 1)]);
}
