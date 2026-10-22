/* chown on a directory keeps all of its mode bits (set-user-ID,
 * set-group-ID, sticky). Directory paths with a trailing slash, "." or
 * ".." work; ".." of "/" is "/". */
#include "a_sys.h"

#define D "/tmp/c_chown_dir_bits"

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }
static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/d", 0755), 0);
    syscall2(SYS_chmod, D "/d", 07755);
    expect("bits_set", mode_of(D "/d"), 07755);
    expect("dir_noop", ch(D "/d", -1, -1), 0);
    expect("dir_bits_kept_noop", mode_of(D "/d"), 07755);
    expect("dir_same_ids", ch(D "/d", uid, gid), 0);
    expect("dir_bits_kept_same_ids", mode_of(D "/d"), 07755);
    syscall2(SYS_chmod, D "/d", 02750);
    expect("dir_trailing_slash", ch(D "/d/", -1, -1), 0);
    expect("dir_sgid_kept", mode_of(D "/d"), 02750);
    expect("dir_dot", ch(D "/d/.", -1, -1), 0);
    expect("dir_dotdot", ch(D "/d/..", -1, -1), 0);
    expect("double_slash", ch(D "//d//", -1, -1), 0);
    expect("dotdot_of_root", ch("/..", -1, -1), 0);
    expect("tmp_dot_slash", ch("/tmp/./", -1, -1), 0);
    expect("cleanup_sub", a_rmdir(D "/d"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
