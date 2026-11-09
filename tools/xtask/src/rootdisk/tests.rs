//! Tests.

use super::*;

#[test]
fn trees_make_parents_and_keep_the_first_directory_mode() {
    let mut t = RootTree::default();
    t.dir("tmp", 0o1777);
    t.file("usr/lib/x/libc.so.6", 0o755, b"elf");
    t.dir("tmp", 0o755);
    t.symlink("lib", "usr/lib");
    assert!(matches!(t.entries.get("tmp"), Some(Entry::Dir(0o1777))));
    assert!(matches!(t.entries.get("usr"), Some(Entry::Dir(0o755))));
    assert!(matches!(t.entries.get("usr/lib/x"), Some(Entry::Dir(0o755))));
    assert!(matches!(t.entries.get("lib"), Some(Entry::Symlink(s)) if s == "usr/lib"));
    // BTreeMap order puts a directory before what is in it.
    let keys: Vec<&String> = t.entries.keys().collect();
    let pos = |k: &str| keys.iter().position(|x| *x == k).unwrap();
    assert!(pos("usr") < pos("usr/lib") && pos("usr/lib/x") < pos("usr/lib/x/libc.so.6"));
}

#[test]
fn header_closure_follows_includes_and_include_next() {
    let dir = std::env::temp_dir().join(format!("poly-hdr-{}", std::process::id()));
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(a.join("sys")).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("top.h"), "#include <sys/one.h>\n# include \"local.h\"\n#include_next <limits.h>\n").unwrap();
    std::fs::write(a.join("sys/one.h"), "#ifdef X\n#  include <missing.h>\n#endif\n").unwrap();
    std::fs::write(a.join("local.h"), "/* nothing */\n").unwrap();
    std::fs::write(a.join("limits.h"), "#include_next <limits.h>\n").unwrap();
    std::fs::write(b.join("limits.h"), "\n").unwrap();
    std::fs::write(b.join("unused.h"), "\n").unwrap();
    let got = header_closure(&["top.h"], &[a.clone(), b.clone()]);
    let _ = std::fs::remove_dir_all(&dir);
    let want: BTreeSet<PathBuf> =
        [a.join("top.h"), a.join("sys/one.h"), a.join("local.h"), a.join("limits.h"), b.join("limits.h")].into();
    assert_eq!(got.into_iter().collect::<BTreeSet<_>>(), want);
}

#[test]
fn linker_script_paths() {
    let dir = std::env::temp_dir().join(format!("poly-ldscript-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("libc.so");
    std::fs::write(
        &p,
        "/* GNU ld script */\nGROUP ( /lib/x/libc.so.6 /usr/lib/x/libc_nonshared.a  AS_NEEDED ( /lib64/ld.so.2 ) )\n",
    )
    .unwrap();
    let got = script_paths(&p);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got, ["/lib/x/libc.so.6", "/usr/lib/x/libc_nonshared.a", "/lib64/ld.so.2"].map(PathBuf::from));
}

#[test]
fn image_sizes_leave_room() {
    let mut t = RootTree::default();
    t.file("a", 0o644, &vec![0; 10 << 20]);
    let (size, inodes) = image_size(&t);
    assert!(size >= (170 << 20) && size % (4 << 20) == 0);
    assert!(inodes >= 16384 && inodes % 1024 == 0);
}
