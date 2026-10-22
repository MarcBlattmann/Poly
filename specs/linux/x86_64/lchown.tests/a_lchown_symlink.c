/* lchown(2) does not follow a symbolic link in the last component: it acts
 * on the link itself, which has its own owner, group and ctime. The
 * target is not looked at, so a dangling link, a link to itself and a link
 * to a file of another user ("/") all work. A link keeps mode 0777. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define D "/tmp/a_lchown_symlink"

static i64 lchown(const char *p, i64 uid, i64 gid) { return syscall3(SYS_lchown, p, uid, gid); }

static void nap(void) {
    struct a_timespec t = {0, 30000000};
    syscall2(SYS_nanosleep, &t, 0);
}

static int later(const struct a_stat *a, const struct a_stat *b) {
    return b->st_ctime_sec > a->st_ctime_sec || (b->st_ctime_sec == a->st_ctime_sec && b->st_ctime_nsec > a->st_ctime_nsec);
}
static int same_ctime(const struct a_stat *a, const struct a_stat *b) {
    return a->st_ctime_sec == b->st_ctime_sec && a->st_ctime_nsec == b->st_ctime_nsec;
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    struct a_stat l0, l1, f0, f1;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "nowhere", AT_FDCWD, D "/dangling"), 0);
    expect("setup_self_loop", syscall3(SYS_symlinkat, "loop", AT_FDCWD, D "/loop"), 0);
    expect("setup_root_link", syscall3(SYS_symlinkat, "/", AT_FDCWD, D "/root"), 0);

    a_lstat_path(D "/l", &l0);
    a_stat_path(D "/f", &f0);
    nap();
    expect("link_self_ids", lchown(D "/l", uid, gid), 0);
    a_lstat_path(D "/l", &l1);
    a_stat_path(D "/f", &f1);
    check("link_is_link", (l1.st_mode & S_IFMT) == S_IFLNK, l1.st_mode);
    check("link_ids", l1.st_uid == (u32)uid && l1.st_gid == (u32)gid, l1.st_uid);
    check("link_ctime_changed", later(&l0, &l1), 0);
    check("target_ctime_unchanged", same_ctime(&f0, &f1), 0);
    expect("link_mode_0777", l1.st_mode & 07777, 0777);

    nap();
    expect("link_minus1", lchown(D "/l", -1, -1), 0);
    a_lstat_path(D "/l", &l0);
    a_stat_path(D "/f", &f0);
    check("link_minus1_ctime_changed", later(&l1, &l0), 0);
    check("link_minus1_target_untouched", same_ctime(&f0, &f1), 0);

    expect("dangling_link", lchown(D "/dangling", uid, gid), 0);
    expect("dangling_link_minus1", lchown(D "/dangling", -1, -1), 0);
    expect("self_loop_link", lchown(D "/loop", uid, gid), 0);
    /* The link to "/" is the caller's; "/" is not looked at. */
    expect("link_to_root_dir", lchown(D "/root", uid, gid), 0);
    a_lstat_path(D "/root", &l0);
    check("link_to_root_dir_ids", (l0.st_mode & S_IFMT) == S_IFLNK && l0.st_uid == (u32)uid, l0.st_uid);

    /* A link in a middle component is followed as usual. */
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_sub_file", a_make_file(D "/sub/g", "", 0644), 0);
    expect("setup_dirlink", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/dl"), 0);
    expect("through_dir_link", lchown(D "/dl/g", uid, gid), 0);
    expect("through_root_link", lchown(D "/root/tmp/a_lchown_symlink/f", -1, -1), 0);

    expect("cleanup_sub_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_dl", a_unlink(D "/dl"), 0);
    expect("cleanup_root", a_unlink(D "/root"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
