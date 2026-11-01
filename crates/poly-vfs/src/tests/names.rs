//! Naming open files (`Vfs::file_path`, for `/proc/<pid>/fd` and
//! `/proc/<pid>/maps`), reading an offset without waiting, and `statfs`.

use super::*;

fn path_of(env: &Env, f: &OpenFile) -> Result<Vec<u8>> {
    env.vfs.file_path(&env.ctx, f)
}

#[test]
fn files_are_named_by_the_directory_and_name_they_were_opened_by() {
    let env = Env::new();
    env.mkdir("/a");
    env.mkdir("/a/b");
    env.write("/a/b/file", b"x");
    let f = env.open("/a/b/file", OpenFlags::RDONLY).unwrap();
    assert_eq!(path_of(&env, &f).unwrap(), b"/a/b/file");
    // Through `..`, `.` and a symbolic link: the name the file has.
    env.symlink("b/file", "/a/link");
    let g = env.open("/a/./b/../link", OpenFlags::RDONLY).unwrap();
    assert_eq!(path_of(&env, &g).unwrap(), b"/a/b/file");
    // Relative to a working directory.
    let mut ctx = env.ctx.clone();
    env.vfs.chdir(&mut ctx, &env.root, At::Cwd, b"/a/b").unwrap();
    let h = env.vfs.open(&ctx, &env.root, At::Cwd, b"file", OpenFlags::RDONLY, 0).unwrap();
    assert_eq!(env.vfs.file_path(&ctx, &h).unwrap(), b"/a/b/file");
    // A file in the root.
    env.write("/top", b"");
    assert_eq!(path_of(&env, &env.open("/top", OpenFlags::RDONLY).unwrap()).unwrap(), b"/top");
}

#[test]
fn created_files_and_directories_have_names_too() {
    let env = Env::new();
    env.mkdir("/d");
    let f = env.open("/d/new", OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::EXCL).unwrap();
    assert_eq!(path_of(&env, &f).unwrap(), b"/d/new");
    let d = env.open("/d/", OpenFlags::RDONLY | OpenFlags::DIRECTORY).unwrap();
    assert_eq!(path_of(&env, &d).unwrap(), b"/d");
    let root = env.open("/", OpenFlags::RDONLY).unwrap();
    assert_eq!(path_of(&env, &root).unwrap(), b"/");
    let p = env.open("/d/new", OpenFlags::PATH).unwrap();
    assert_eq!(path_of(&env, &p).unwrap(), b"/d/new");
}

#[test]
fn unlinked_files_are_marked_deleted_and_renamed_ones_found() {
    let env = Env::new();
    env.write("/gone", b"x");
    env.write("/moved", b"y");
    let gone = env.open("/gone", OpenFlags::RDONLY).unwrap();
    let moved = env.open("/moved", OpenFlags::RDONLY).unwrap();
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/gone").unwrap();
    assert_eq!(path_of(&env, &gone).unwrap(), b"/gone (deleted)");
    // Another file under the old name does not make it come back.
    env.write("/gone", b"new");
    assert_eq!(path_of(&env, &gone).unwrap(), b"/gone (deleted)");
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, b"/moved", At::Cwd, b"/elsewhere", RenameFlags::default()).unwrap();
    assert_eq!(path_of(&env, &moved).unwrap(), b"/elsewhere", "renamed in its directory");
    // Moved to another directory: the old path, still not deleted.
    env.mkdir("/sub");
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, b"/elsewhere", At::Cwd, b"/sub/x", RenameFlags::default()).unwrap();
    assert_eq!(path_of(&env, &moved).unwrap(), b"/moved");
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/sub/x").unwrap();
    assert_eq!(path_of(&env, &moved).unwrap(), b"/moved (deleted)");
}

#[test]
fn files_on_other_mounts_get_their_full_path() {
    let env = Env::new();
    env.mkdir("/mnt");
    env.vfs.mount(&env.ctx, &env.root, At::Cwd, b"/mnt", tmpfs(), MountFlags::default()).unwrap();
    env.mkdir("/mnt/sub");
    env.write("/mnt/sub/f", b"z");
    let f = env.open("/mnt/sub/f", OpenFlags::RDONLY).unwrap();
    assert_eq!(path_of(&env, &f).unwrap(), b"/mnt/sub/f");
    let at_root = env.open("/mnt", OpenFlags::RDONLY).unwrap();
    assert_eq!(path_of(&env, &at_root).unwrap(), b"/mnt");
}

#[test]
fn files_opened_for_exec_have_names() {
    let env = Env::new();
    env.mkdir("/bin");
    let f =
        env.vfs.open(&env.ctx, &env.root, At::Cwd, b"/bin/prog", OpenFlags::WRONLY | OpenFlags::CREAT, 0o755).unwrap();
    drop(f);
    let exe = env.vfs.open_exec(&env.ctx, &env.root, At::Cwd, b"/bin/../bin/prog").unwrap();
    assert_eq!(path_of(&env, &exe).unwrap(), b"/bin/prog");
}

#[test]
fn offset_now_does_not_wait_for_a_transfer() {
    let env = Env::new();
    env.write("/f", b"0123456789");
    let f = env.open("/f", OpenFlags::RDONLY).unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(f.read(&mut buf), Ok(4));
    assert_eq!(f.offset_now(), Some(4));
    // During a listing of a directory the description's own offset is
    // held: asking for it from the callback must not deadlock.
    let d = env.open("/", OpenFlags::RDONLY).unwrap();
    let mut seen = None;
    d.read_dir(&mut |_| {
        seen = Some(d.offset_now());
        false
    })
    .unwrap();
    assert_eq!(seen, Some(None));
    assert_eq!(d.offset_now(), Some(0));
}

#[test]
fn statfs_reports_the_file_system_and_the_mount_flags() {
    let env = Env::new();
    env.write("/f", &[1u8; 5000]);
    let root = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/f", true).unwrap();
    let st = root.statfs().unwrap();
    assert_eq!(st.magic, TMPFS_MAGIC);
    assert_eq!(st.block_size, 4096);
    assert_eq!(st.blocks, 2, "no limit: the pages in use");
    assert_eq!(st.blocks_free, 0);
    assert_eq!(st.flags, ST_RELATIME, "relatime, the default access time mode");
    assert_eq!(st.name_max, 255);
    assert!(st.files >= 2);

    env.mkdir("/ro");
    let limited = Arc::new(Tmpfs::with_limit(Arc::new(TestClock::default()), 10 * 4096));
    let flags = MountFlags { read_only: true, no_exec: true, no_dev: true, no_suid: true };
    env.vfs.mount(&env.ctx, &env.root, At::Cwd, b"/ro", limited, flags).unwrap();
    let ro = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/ro", true).unwrap();
    let st = ro.statfs().unwrap();
    assert_eq!((st.blocks, st.blocks_free, st.blocks_avail), (10, 10, 10));
    assert_eq!(st.flags, ST_RDONLY | ST_NOSUID | ST_NODEV | ST_NOEXEC | ST_RELATIME);
    assert_ne!(st.fsid, root.statfs().unwrap().fsid);
}

#[test]
fn tmpfs_limits_are_fixed_totals() {
    let env = Env::new();
    env.mkdir("/l");
    let fs = Arc::new(Tmpfs::with_limits(Arc::new(TestClock::default()), 4 * 4096, 3));
    env.vfs.mount(&env.ctx, &env.root, At::Cwd, b"/l", fs, MountFlags::default()).unwrap();
    let stat = || env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/l", true).unwrap().statfs().unwrap();
    let before = stat();
    assert_eq!((before.blocks, before.blocks_free, before.files, before.files_free), (4, 4, 3, 2));
    env.write("/l/a", &[1u8; 5000]);
    let during = stat();
    assert_eq!((during.blocks, during.blocks_free, during.files, during.files_free), (4, 2, 3, 1));
    env.write("/l/b", b"x");
    assert_eq!(env.open("/l/c", OpenFlags::WRONLY | OpenFlags::CREAT).map(|_| ()), Err(Errno::ENOSPC), "no inode left");
    // Without limits the inode counts are consistent too.
    let st = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/", true).unwrap().statfs().unwrap();
    assert!(st.files_free <= st.files);
}

#[test]
fn magic_numbers_by_name() {
    assert_eq!(magic_for("tmpfs"), 0x0102_1994);
    assert_eq!(magic_for("ext2"), 0xef53);
    assert_eq!(magic_for("proc"), 0x9fa0);
    assert_eq!(magic_for("sysfs"), 0x6265_6572);
    assert_eq!(magic_for("nope"), 0);
}

#[test]
fn the_namespace_generation_counts_every_change_of_a_name_and_nothing_else() {
    let env = Env::new();
    let g = || env.vfs.namespace_generation();
    let start = g();
    env.mkdir("/d");
    env.write("/d/f", b"one");
    assert_eq!(g(), start + 2);
    // Writing, reading and changing the mode of an existing file change no
    // name.
    env.write("/d/f", b"two");
    assert_eq!(env.read("/d/f").unwrap(), b"two");
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/d/f", 0o600, true).unwrap();
    assert_eq!(g(), start + 2);
    // A link, a symbolic link, a node, a rename, four unlinks, an rmdir.
    env.vfs.link(&env.ctx, &env.root, At::Cwd, b"/d/f", At::Cwd, b"/d/g", false).unwrap();
    env.symlink("f", "/d/s");
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, b"/d/fifo", FileType::Fifo, 0o644, DeviceId::default()).unwrap();
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, b"/d/g", At::Cwd, b"/d/h", RenameFlags::default()).unwrap();
    for name in [&b"/d/f"[..], b"/d/h", b"/d/s", b"/d/fifo"] {
        env.vfs.unlink(&env.ctx, &env.root, At::Cwd, name).unwrap();
    }
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/d").unwrap();
    assert_eq!(g(), start + 2 + 4 + 4 + 1);
    // What fails changes nothing.
    assert!(env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/nothing").is_err());
    assert!(env.vfs.mkdir(&env.ctx, &env.root, At::Cwd, b"/missing/d", 0o755).is_err());
    assert_eq!(g(), start + 11);
}
