/* A directory made inside a set-group-ID directory takes the parent's
 * group (not the caller's) and is set-group-ID itself, whatever the mode
 * argument says; this carries on to its own subdirectories. Outside such
 * a parent, the group is the caller's effective group and a requested
 * set-group-ID bit is dropped. Seeing the group taken from the parent
 * needs a parent group other than the caller's, which only root can set
 * up here (chown to group 4242); other callers check the bits only. */
#include "a_sys.h"

#define D "/tmp/c_mkdir_setgid_parent"
#define G 4242

static i64 mk(const char *p, i64 m) { return syscall2(SYS_mkdir, p, m); }
static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}
static i64 gid_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)st.st_gid;
}

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    i64 egid = syscall0(SYS_getegid);
    i64 pgid = root ? G : egid;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_sg", mk(D "/sg", 0755), 0);
    if (root) expect("root_chown_sg", syscall3(SYS_chown, D "/sg", -1, G), 0);
    expect("chmod_sg_02755", syscall2(SYS_chmod, D "/sg", 02755), 0);
    expect("sg_mode", mode_of(D "/sg"), S_IFDIR | 02755);

    expect("mk_child", mk(D "/sg/c", 0700), 0);
    expect("child_inherits_sgid", mode_of(D "/sg/c"), S_IFDIR | 02700);
    expect("child_group_from_parent", gid_of(D "/sg/c"), pgid);
    expect("mk_child_sgid_asked", mk(D "/sg/c2", 02750), 0);
    expect("child2_sgid", mode_of(D "/sg/c2"), S_IFDIR | 02750);
    expect("mk_child_suid_sticky", mk(D "/sg/c3", 05777), 0);
    expect("child3_suid_dropped_sticky_kept", mode_of(D "/sg/c3"), S_IFDIR | 03755);
    expect("mk_grandchild", mk(D "/sg/c/g", 0755), 0);
    expect("grandchild_sgid", mode_of(D "/sg/c/g"), S_IFDIR | 02755);
    expect("grandchild_group", gid_of(D "/sg/c/g"), pgid);
    expect("file_in_sg", a_make_file(D "/sg/f", "", 0644), 0);
    expect("file_group_from_parent", gid_of(D "/sg/f"), pgid);

    /* Clearing the parent's bit: new children are plain again. */
    expect("chmod_sg_0755", syscall2(SYS_chmod, D "/sg", 0755), 0);
    expect("mk_after_clear", mk(D "/sg/p", 02755), 0);
    expect("after_clear_no_sgid", mode_of(D "/sg/p"), S_IFDIR | 0755);
    expect("after_clear_group_is_egid", gid_of(D "/sg/p"), egid);
    expect("old_child_keeps_sgid", mode_of(D "/sg/c"), S_IFDIR | 02700);

    expect("mk_plain", mk(D "/plain", 02777), 0);
    expect("plain_no_sgid", mode_of(D "/plain"), S_IFDIR | 0755);
    expect("plain_group_is_egid", gid_of(D "/plain"), egid);

    expect("rm_g", a_rmdir(D "/sg/c/g"), 0);
    expect("rm_c", a_rmdir(D "/sg/c"), 0);
    expect("rm_c2", a_rmdir(D "/sg/c2"), 0);
    expect("rm_c3", a_rmdir(D "/sg/c3"), 0);
    expect("rm_p", a_rmdir(D "/sg/p"), 0);
    expect("rm_f", a_unlink(D "/sg/f"), 0);
    expect("rm_sg", a_rmdir(D "/sg"), 0);
    expect("rm_plain", a_rmdir(D "/plain"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
