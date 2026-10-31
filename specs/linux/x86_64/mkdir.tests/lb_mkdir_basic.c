/* mkdir(2): a new empty directory with mode (mode & 01777 & ~umask): the
 * set-user-ID and set-group-ID bits of mode are ignored, the sticky bit is
 * kept. Owner and group are the caller's. It has 2 links and adds one to
 * its parent; the parent's mtime and ctime change. Trailing slashes are
 * fine; relative paths start at the working directory. */
#include "a_sys.h"

#define D "/tmp/lb_mkdir_basic"

static i64 mkdir_(const char *p, i64 mode) { return syscall2(SYS_mkdir, p, mode); }
static i64 umask_(i64 m) { return syscall1(SYS_umask, m); }
static u32 mode_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st)) return 0xffffffff;
    return st.st_mode;
}
static i64 nlink_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st)) return -1;
    return (i64)st.st_nlink;
}

int test_main(void) {
    struct a_stat st, pst, pst2;
    expect("mkdir", mkdir_(D, 0755), 0);
    expect("mode_0755", mode_of(D), S_IFDIR | 0755);
    expect("nlink_2", nlink_of(D), 2);
    expect("stat", a_stat_path(D, &st), 0);
    expect("uid_is_euid", st.st_uid, syscall0(SYS_geteuid));
    expect("gid_is_egid", st.st_gid, syscall0(SYS_getegid));
    expect("size_nonneg", st.st_size >= 0, 1);
    check("times_equal", st.st_atime_sec == st.st_mtime_sec && st.st_mtime_sec == st.st_ctime_sec &&
                             st.st_mtime_nsec == st.st_ctime_nsec, 0);

    /* umask 022 applies */
    expect("mkdir_0777", mkdir_(D "/a", 0777), 0);
    expect("mode_a", mode_of(D "/a"), S_IFDIR | 0755);
    /* set-id bits ignored, sticky kept */
    expect("mkdir_07777", mkdir_(D "/b", 07777), 0);
    expect("mode_b", mode_of(D "/b"), S_IFDIR | 01755);
    expect("mkdir_06700", mkdir_(D "/c", 06700), 0);
    expect("mode_c", mode_of(D "/c"), S_IFDIR | 0700);
    expect("mkdir_0", mkdir_(D "/z", 0), 0);
    expect("mode_z", mode_of(D "/z"), S_IFDIR);
    /* type bits in mode are ignored */
    expect("mkdir_type_bits", mkdir_(D "/t", S_IFREG | 0750), 0);
    expect("mode_t", mode_of(D "/t"), S_IFDIR | 0750);
    /* bits above 07777 and the upper half are ignored */
    expect("mkdir_high", mkdir_(D "/h", (i64)0x500010000L | 0711), 0);
    expect("mode_h", mode_of(D "/h"), S_IFDIR | 0711);
    /* umask 0 and umask 0777 */
    expect("umask_0", umask_(0), 022);
    expect("mkdir_umask0", mkdir_(D "/u0", 01777), 0);
    expect("mode_u0", mode_of(D "/u0"), S_IFDIR | 01777);
    expect("umask_0777", umask_(0777), 0);
    expect("mkdir_umask777", mkdir_(D "/u7", 0777), 0);
    expect("mode_u7", mode_of(D "/u7"), S_IFDIR);
    expect("umask_back", umask_(022), 0777);

    /* link counts: subdirectories count, files do not */
    expect("parent_nlink", nlink_of(D), 2 + 8);
    expect("make_file", a_make_file(D "/file", "x", 0644), 0);
    expect("parent_nlink_file", nlink_of(D), 2 + 8);

    /* trailing slashes, repeated slashes, relative paths */
    expect("trailing_slash", mkdir_(D "/s1/", 0700), 0);
    expect("trailing_slashes", mkdir_(D "//s2///", 0700), 0);
    expect("s2_exists", mode_of(D "/s2"), S_IFDIR | 0700);
    expect("chdir", syscall1(SYS_chdir, D "/a"), 0);
    expect("relative", mkdir_("rel", 0700), 0);
    expect("relative_dotdot", mkdir_("../a/rel2", 0700), 0);
    expect("relative_dot", mkdir_("./rel3", 0700), 0);
    expect("a_nlink", nlink_of(D "/a"), 5);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    /* nesting */
    expect("nested", mkdir_(D "/a/rel/deep", 0700), 0);
    expect("nested_exists", mode_of(D "/a/rel/deep"), S_IFDIR | 0700);

    /* the parent's mtime and ctime change */
    struct a_timespec ts[2] = {{1000000000, 0}, {1000000000, 0}};
    expect("set_parent_times", syscall4(SYS_utimensat, AT_FDCWD, D "/b", ts, 0), 0);
    expect("stat_parent0", a_stat_path(D "/b", &pst), 0);
    struct a_timespec pause = {0, 20000000};
    syscall2(SYS_nanosleep, &pause, 0);
    expect("mkdir_in_b", mkdir_(D "/b/x", 0700), 0);
    expect("stat_parent1", a_stat_path(D "/b", &pst2), 0);
    check("parent_mtime_changed", pst2.st_mtime_sec > 1000000000, 0);
    check("parent_ctime_changed", pst2.st_ctime_sec > pst.st_ctime_sec ||
                                      (pst2.st_ctime_sec == pst.st_ctime_sec && pst2.st_ctime_nsec > pst.st_ctime_nsec),
          0);
    expect("parent_atime_kept", pst2.st_atime_sec, 1000000000);
    expect("stat_child", a_stat_path(D "/b/x", &st), 0);
    check("child_mtime_recent", st.st_mtime_sec > 1000000000, 0);

    static const char *rm[] = {D "/b/x", D "/a/rel/deep", D "/a/rel3", D "/a/rel2", D "/a/rel", D "/s2", D "/s1",
                               D "/u7", D "/u0", D "/h", D "/t", D "/z", D "/c", D "/b", D "/a"};
    for (unsigned i = 0; i < sizeof rm / sizeof rm[0]; i++) expect("cleanup_rmdir_sub", a_rmdir(rm[i]), 0);
    expect("cleanup_file", a_unlink(D "/file"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
