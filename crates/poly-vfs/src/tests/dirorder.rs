//! The order tmpfs lists a directory in, its `d_off` values and what
//! reading from a position does, through `OpenFile::read_dir` and `seek`
//! (the spec sheet `specs/linux/x86_64/getdents64.yaml`,
//! `directory_order.tmpfs`): `.` and `..` first, then the entries newest
//! first; per-directory cookies from 3, never reused; rename over keeps
//! the replaced name's cookie; the end is `INT32_MAX` and stays the end
//! until lseek.

use super::Env;
use crate::*;
use alloc::string::String;
use alloc::vec::Vec;

const END: u64 = i32::MAX as u64;

/// A record as the tests see it: name and `d_off`.
type Rec = (String, u64);

/// One listing call that takes at most `max` records (a buffer with room
/// for `max`).
fn call(d: &OpenFile, max: usize) -> Vec<Rec> {
    let mut out = Vec::new();
    d.read_dir(&mut |e| {
        if out.len() == max {
            return false;
        }
        out.push((String::from_utf8(e.name.to_vec()).unwrap(), e.next));
        true
    })
    .unwrap();
    out
}

/// Calls of `max` records from the position until one lists nothing.
fn rest_by(d: &OpenFile, max: usize) -> Vec<Rec> {
    let mut all = Vec::new();
    loop {
        let got = call(d, max);
        if got.is_empty() {
            return all;
        }
        all.extend(got);
    }
}

fn rest(d: &OpenFile) -> Vec<Rec> {
    rest_by(d, usize::MAX)
}

/// The whole listing, from position 0.
fn listing(d: &OpenFile) -> Vec<Rec> {
    assert_eq!(d.seek(0, Whence::Set), Ok(0));
    rest(d)
}

/// The names, space-separated.
fn names(recs: &[Rec]) -> String {
    recs.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(" ")
}

fn offs(recs: &[Rec]) -> Vec<u64> {
    recs.iter().map(|r| r.1).collect()
}

fn pos(d: &OpenFile) -> u64 {
    d.seek(0, Whence::Cur).unwrap()
}

fn set(d: &OpenFile, at: u64) {
    assert_eq!(d.seek(at as i64, Whence::Set), Ok(at));
}

/// Creates each space-separated name in `dir`, in order.
fn mk(env: &Env, dir: &str, list: &str) {
    for n in list.split(' ') {
        env.write(&alloc::format!("{dir}/{n}"), b"");
    }
}

fn unlink(env: &Env, path: &str) {
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, path.as_bytes()).unwrap();
}

fn rename_flags(env: &Env, old: &str, new: &str, flags: RenameFlags) {
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, old.as_bytes(), At::Cwd, new.as_bytes(), flags).unwrap();
}

fn rename(env: &Env, old: &str, new: &str) {
    rename_flags(env, old, new, RenameFlags::default());
}

fn link(env: &Env, old: &str, new: &str) {
    env.vfs.link(&env.ctx, &env.root, At::Cwd, old.as_bytes(), At::Cwd, new.as_bytes(), false).unwrap();
}

fn open_dir(env: &Env, path: &str) -> alloc::sync::Arc<OpenFile> {
    env.open(path, OpenFlags::RDONLY | OpenFlags::DIRECTORY).unwrap()
}

/// A directory `/d` with `list` created in it, open.
fn dir_with(env: &Env, list: &str) -> alloc::sync::Arc<OpenFile> {
    env.mkdir("/d");
    mk(env, "/d", list);
    open_dir(env, "/d")
}

#[test]
fn newest_first_with_cookies_from_three() {
    let env = Env::new();
    let d = dir_with(&env, "mm cc xx aa zz bb yy dd");
    let l = listing(&d);
    assert_eq!(names(&l), ". .. dd yy bb zz aa xx cc mm");
    assert_eq!(offs(&l), [1, 10, 9, 8, 7, 6, 5, 4, 3, END]);
    assert_eq!(pos(&d), END);
    // Every kind of entry is a new one at the front: directories,
    // symbolic links, hard links, device nodes.
    env.mkdir("/d/sub");
    env.symlink("mm", "/d/sl");
    link(&env, "/d/mm", "/d/hl");
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, b"/d/fifo", FileType::Fifo, 0o644, DeviceId::default()).unwrap();
    let l = listing(&d);
    assert_eq!(names(&l), ". .. fifo hl sl sub dd yy bb zz aa xx cc mm");
    assert_eq!(offs(&l)[..6], [1, 14, 13, 12, 11, 10]);
    // A failed lookup of a name changes nothing.
    assert_eq!(env.stat("/d/nothing"), Err(Errno::ENOENT));
    assert_eq!(listing(&d), l);
}

#[test]
fn every_directory_counts_for_itself() {
    let env = Env::new();
    let d = dir_with(&env, "a b c d e");
    env.mkdir("/d/sub");
    let s = open_dir(&env, "/d/sub");
    // Empty: "." then "..", whose d_off is the end.
    let l = listing(&s);
    assert_eq!(names(&l), ". ..");
    assert_eq!(offs(&l), [1, END]);
    mk(&env, "/d/sub", "p q r");
    let l = listing(&s);
    assert_eq!(names(&l), ". .. r q p");
    assert_eq!(offs(&l), [1, 5, 4, 3, END]);
    assert_eq!(offs(&listing(&d))[1], 8, "sub took the parent's cookie 8");
}

#[test]
fn cookies_are_not_reused() {
    let env = Env::new();
    env.mkdir("/d");
    for i in 0..40 {
        env.write(&alloc::format!("/d/f{i:02}"), b"");
    }
    for i in (0..40).step_by(2) {
        unlink(&env, &alloc::format!("/d/f{i:02}"));
    }
    mk(&env, "/d", "g0 g1 g2 g3 g4");
    let d = open_dir(&env, "/d");
    let l = listing(&d);
    let mut want = String::from(". .. g4 g3 g2 g1 g0");
    for i in (1..40).rev().step_by(2) {
        want.push_str(&alloc::format!(" f{i:02}"));
    }
    assert_eq!(names(&l), want);
    // g0..g4 got 43..47; g0 is followed by f39 (cookie 42); f01 (cookie 4)
    // is the last.
    assert_eq!(offs(&l)[..7], [1, 47, 46, 45, 44, 43, 42]);
    assert_eq!(l[l.len() - 2], (String::from("f03"), 4));
    assert_eq!(l[l.len() - 1], (String::from("f01"), END));
}

#[test]
fn a_name_created_again_is_a_new_entry() {
    let env = Env::new();
    let d = dir_with(&env, "a b");
    let still_open = env.open("/d/a", OpenFlags::RDONLY).unwrap();
    unlink(&env, "/d/a");
    mk(&env, "/d", "a");
    let l = listing(&d);
    assert_eq!(names(&l), ". .. a b");
    assert_eq!(offs(&l), [1, 5, 4, END]);
    drop(still_open);
}

#[test]
fn rename_to_a_new_name_goes_to_the_front_with_a_new_cookie() {
    let env = Env::new();
    let d = dir_with(&env, "a b c d e");
    env.mkdir("/o");
    rename(&env, "/d/b", "/d/b2");
    let l = listing(&d);
    assert_eq!(names(&l), ". .. b2 e d c a");
    assert_eq!(offs(&l), [1, 8, 7, 6, 5, 3, END]);
    // From another directory: the same (z made there first).
    mk(&env, "/o", "z");
    rename(&env, "/o/z", "/d/z");
    let l = listing(&d);
    assert_eq!(names(&l), ". .. z b2 e d c a");
    assert_eq!(offs(&l)[..3], [1, 9, 8]);
    // A directory moved in.
    env.mkdir("/o/inner");
    rename(&env, "/o/inner", "/d/inner");
    assert_eq!(names(&listing(&d)), ". .. inner z b2 e d c a");
    // Away and back: a new cookie each time.
    rename(&env, "/d/a", "/d/a_tmp");
    rename(&env, "/d/a_tmp", "/d/a");
    let l = listing(&d);
    assert_eq!(names(&l), ". .. a inner z b2 e d c");
    assert_eq!(offs(&l)[..3], [1, 12, 10]);
    // Moved out: gone; the others keep their places and cookies.
    rename(&env, "/d/e", "/o/e");
    let l = listing(&d);
    assert_eq!(names(&l), ". .. a inner z b2 d c");
    assert_eq!(offs(&l), [1, 12, 10, 9, 8, 6, 5, END]);
}

#[test]
fn rename_over_puts_the_name_in_front_with_its_cookie() {
    let env = Env::new();
    let d = dir_with(&env, "v w x y z");
    let w = env.ino("/d/w").unwrap();
    rename(&env, "/d/w", "/d/y");
    let l = listing(&d);
    assert_eq!(names(&l), ". .. y z x v");
    // y kept 6, below z's 7: the list order is not the cookie order.
    assert_eq!(offs(&l), [1, 6, 7, 5, 3, END]);
    assert_eq!(env.ino("/d/y"), Ok(w));
    // From another directory over x (cookie 5).
    env.mkdir("/o");
    mk(&env, "/o", "q");
    rename(&env, "/o/q", "/d/x");
    let l = listing(&d);
    assert_eq!(names(&l), ". .. x y z v");
    assert_eq!(offs(&l), [1, 5, 6, 7, 3, END]);
    // A directory over an empty directory.
    env.mkdir("/d/s1");
    env.mkdir("/d/s2");
    rename(&env, "/d/s2", "/d/s1");
    let l = listing(&d);
    assert_eq!(names(&l), ". .. s1 x y z v");
    assert_eq!(offs(&l)[1], 8, "s1 kept its cookie");
}

#[test]
fn rename_exchange_moves_both_names_to_the_front() {
    let env = Env::new();
    let d = dir_with(&env, "a b c");
    let (ia, ic) = (env.ino("/d/a").unwrap(), env.ino("/d/c").unwrap());
    let exchange = RenameFlags { exchange: true, ..RenameFlags::default() };
    rename_flags(&env, "/d/a", "/d/c", exchange);
    let l = listing(&d);
    // The second name first, then the first, then the rest; each name
    // keeps its cookie (a 3, b 4, c 5).
    assert_eq!(names(&l), ". .. c a b");
    assert_eq!(offs(&l), [1, 5, 3, 4, END]);
    assert_eq!((env.ino("/d/a"), env.ino("/d/c")), (Ok(ic), Ok(ia)));
    // Across directories: each name keeps its cookie and goes to the
    // front of its own directory.
    env.mkdir("/o");
    mk(&env, "/o", "p q");
    rename_flags(&env, "/d/b", "/o/p", exchange);
    let l = listing(&d);
    assert_eq!(names(&l), ". .. b c a");
    assert_eq!(offs(&l), [1, 4, 5, 3, END]);
    let o = open_dir(&env, "/o");
    let l = listing(&o);
    assert_eq!(names(&l), ". .. p q");
    assert_eq!(offs(&l), [1, 3, 4, END]);
}

#[test]
fn renames_that_do_nothing_change_nothing() {
    let env = Env::new();
    let d = dir_with(&env, "a b c");
    link(&env, "/d/c", "/d/c_link");
    let before = listing(&d);
    rename(&env, "/d/b", "/d/b");
    rename(&env, "/d/c", "/d/c_link");
    rename(&env, "/d/c_link", "/d/c");
    let exchange = RenameFlags { exchange: true, ..RenameFlags::default() };
    rename_flags(&env, "/d/c", "/d/c_link", exchange);
    assert_eq!(listing(&d), before);
    assert_eq!(names(&before), ". .. c_link c b a");
}

#[test]
fn reading_from_a_position() {
    let env = Env::new();
    let d = dir_with(&env, "a b c d e f");
    // a..f have 3..8.
    let from = |p: u64| {
        set(&d, p);
        names(&rest(&d))
    };
    assert_eq!(from(0), ". .. f e d c b a");
    assert_eq!(from(1), ".. f e d c b a");
    assert_eq!(from(2), "f e d c b a");
    assert_eq!(from(5), "c b a");
    // No cookie 100: the largest below it (f's).
    assert_eq!(from(100), "f e d c b a");
    assert_eq!(from(END), "");
    // Above the end: from the largest cookie.
    assert_eq!(from(END + 1), "f e d c b a");
    assert_eq!(from(1 << 40), "f e d c b a");
    // Every cookie above the position: from the front.
    unlink(&env, "/d/a");
    assert_eq!(from(3), "f e d c b");
    // Seeking to each d_off reads the record after it, none after the last.
    let l = listing(&d);
    for (i, r) in l.iter().enumerate() {
        set(&d, r.1);
        assert_eq!(call(&d, 1).first(), l.get(i + 1), "after {}", r.0);
    }
}

#[test]
fn reading_from_a_position_when_list_and_cookie_order_differ() {
    let env = Env::new();
    let d = dir_with(&env, "v w x y z");
    rename(&env, "/d/w", "/d/y");
    // ". .. y z x v" with y 6, z 7, x 5, v 3.
    set(&d, 2);
    assert_eq!(names(&rest(&d)), "y z x v");
    set(&d, 7);
    assert_eq!(names(&rest(&d)), "z x v");
    set(&d, END + 1);
    assert_eq!(names(&rest(&d)), "z x v", "above the end: the largest cookie, z");
    set(&d, 6);
    assert_eq!(names(&rest(&d)), "y z x v");
    set(&d, 4);
    assert_eq!(names(&rest(&d)), "v");
}

#[test]
fn entries_removed_between_calls() {
    let env = Env::new();
    // Not yet returned: skipped; the next one removed: the one after it.
    let d = dir_with(&env, "a b c d e f g h");
    assert_eq!(names(&call(&d, 4)), ". .. h g");
    assert_eq!(pos(&d), 8, "f's cookie");
    unlink(&env, "/d/f");
    unlink(&env, "/d/c");
    assert_eq!(names(&rest(&d)), "e d b a");
    // Already returned: nothing changes.
    set(&d, 0);
    assert_eq!(names(&call(&d, 4)), ". .. h g");
    unlink(&env, "/d/h");
    assert_eq!(names(&rest(&d)), "e d b a");
}

#[test]
fn a_removed_position_can_list_entries_again() {
    // The spec's examples where the list order is not the cookie order.
    let env = Env::new();
    let d = dir_with(&env, "v w x y z");
    rename(&env, "/d/w", "/d/y");
    assert_eq!(names(&call(&d, 3)), ". .. y");
    assert_eq!(pos(&d), 7, "z's cookie");
    unlink(&env, "/d/z");
    // The largest cookie not above 7 is y's (6).
    assert_eq!(names(&rest(&d)), "y x v");

    let env = Env::new();
    let d = dir_with(&env, "k l m n");
    rename(&env, "/d/l", "/d/k");
    assert_eq!(names(&listing(&d)), ". .. k n m");
    set(&d, 0);
    assert_eq!(names(&call(&d, 4)), ". .. k n");
    unlink(&env, "/d/m");
    // Only k's cookie (3) is at or below m's (5): from k, at the front.
    assert_eq!(names(&rest(&d)), "k n");
    assert_eq!(call(&d, 10), []);
}

#[test]
fn entries_added_or_renamed_between_calls() {
    let env = Env::new();
    let d = dir_with(&env, "a b c");
    // After "." only: ".." is followed by the front, so a new entry is
    // listed.
    assert_eq!(names(&call(&d, 1)), ".");
    mk(&env, "/d", "x");
    assert_eq!(names(&rest(&d)), ".. x c b a");
    // After "..": the front has been left.
    set(&d, 0);
    assert_eq!(names(&call(&d, 2)), ". ..");
    mk(&env, "/d", "y");
    assert_eq!(names(&rest(&d)), "x c b a");
    // Not yet returned and renamed: moved to the front, not in this pass.
    set(&d, 0);
    assert_eq!(names(&call(&d, 3)), ". .. y");
    rename(&env, "/d/b", "/d/b2");
    assert_eq!(names(&rest(&d)), "x c a");
    // Renamed over one not yet returned: neither is returned.
    set(&d, 0);
    assert_eq!(names(&call(&d, 3)), ". .. b2");
    rename(&env, "/d/b2", "/d/c");
    assert_eq!(names(&rest(&d)), "y x a");
    assert_eq!(names(&listing(&d)), ". .. c y x a");
}

#[test]
fn the_end_stays_the_end_until_lseek() {
    let env = Env::new();
    let d = dir_with(&env, "a");
    assert_eq!(names(&rest(&d)), ". .. a");
    assert_eq!(pos(&d), END);
    mk(&env, "/d", "b");
    assert_eq!(call(&d, 10), []);
    assert_eq!(call(&d, 10), []);
    assert_eq!(pos(&d), END);
    // Another descriptor, opened before b was made, lists it.
    let other = open_dir(&env, "/d");
    mk(&env, "/d", "c");
    assert_eq!(names(&rest(&other)), ". .. c b a");
    assert_eq!(names(&listing(&d)), ". .. c b a");

    // A position whose entries have all gone: the call that finds nothing
    // moves it to the end, where it stays.
    let env = Env::new();
    let d = dir_with(&env, "a");
    assert_eq!(names(&call(&d, 2)), ". ..");
    assert_eq!(pos(&d), 3);
    unlink(&env, "/d/a");
    assert_eq!(call(&d, 10), []);
    assert_eq!(pos(&d), END);
    mk(&env, "/d", "b");
    assert_eq!(call(&d, 10), []);
    set(&d, 3);
    assert_eq!(names(&rest(&d)), "b", "from the front: every cookie is above 3");
}

#[test]
fn a_record_that_does_not_fit_leaves_the_position() {
    let env = Env::new();
    let d = dir_with(&env, "a b c");
    assert_eq!(call(&d, 0), []);
    assert_eq!(pos(&d), 0, "not moved to the end: nothing fitted");
    assert_eq!(names(&call(&d, 2)), ". ..");
    let at = pos(&d);
    assert_eq!(at, 5, "c's cookie");
    assert_eq!(call(&d, 0), []);
    assert_eq!(pos(&d), at);
    assert_eq!(names(&rest(&d)), "c b a");
}

#[test]
fn lseek_on_a_directory() {
    let env = Env::new();
    let d = dir_with(&env, "a b c");
    assert_eq!(d.seek(5, Whence::Set), Ok(5));
    assert_eq!(d.seek(3, Whence::Cur), Ok(8));
    assert_eq!(d.seek(-100, Whence::Cur), Err(Errno::EINVAL));
    assert_eq!(d.seek(-1, Whence::Set), Err(Errno::EINVAL));
    assert_eq!(d.seek(0, Whence::End), Err(Errno::EINVAL));
    assert_eq!(d.seek(0, Whence::Data), Err(Errno::EINVAL));
    assert_eq!(d.seek(0, Whence::Hole), Err(Errno::EINVAL));
    assert_eq!(pos(&d), 8, "unchanged after the errors");
    assert_eq!(d.seek(1 << 40, Whence::Set), Ok(1 << 40));
    assert_eq!(d.seek(-(1 << 40) + 4, Whence::Cur), Ok(4));
    assert_eq!(names(&rest(&d)), "b a", "4 is b's cookie");
    // A remembered position gives the same rest while nothing changes.
    set(&d, 0);
    call(&d, 3);
    let mark = pos(&d);
    let first = rest(&d);
    set(&d, mark);
    assert_eq!(rest(&d), first);
    assert_eq!(names(&first), "b a");
}

#[test]
fn d_off_does_not_depend_on_the_call_size_or_on_batches() {
    // More entries than tmpfs copies out per round, and a list whose order
    // is not the cookie order across the rounds.
    let env = Env::new();
    env.mkdir("/d");
    for i in 0..150 {
        env.write(&alloc::format!("/d/f{i:03}"), b"");
    }
    rename(&env, "/d/f010", "/d/f100");
    rename(&env, "/d/f020", "/d/f070");
    let d = open_dir(&env, "/d");
    let all = listing(&d);
    // ". .. f070 f100 f149 ... f000" without f010 and f020, each once.
    let mut want = alloc::vec![String::from("."), String::from(".."), String::from("f070"), String::from("f100")];
    for i in (0..150).rev() {
        if ![10, 20, 70, 100].contains(&i) {
            want.push(alloc::format!("f{i:03}"));
        }
    }
    assert_eq!(all.iter().map(|r| r.0.clone()).collect::<Vec<_>>(), want);
    assert_eq!(offs(&all)[..5], [1, 73, 103, 152, 151]);
    for per in [1, 2, 3, 7, 64] {
        set(&d, 0);
        assert_eq!(rest_by(&d, per), all, "{per} records a call");
    }
}
