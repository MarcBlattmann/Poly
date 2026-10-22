/* mkdir and permissions: the caller needs write and search permission on
 * the parent directory and search permission on every directory on the
 * way; read permission is not needed. Without them: EACCES, unless the
 * caller is root (who passes these checks). Order: an existing name in a
 * parent that is searchable but not writable is EEXIST (also "."), a
 * missing directory under it ENOENT; in a parent that is not searchable
 * every name is EACCES (for root: the usual result). A name over 255
 * bytes in a read-only parent is ENAMETOOLONG; in an unsearchable one
 * EACCES. The new directory's own mode does not matter for its creation,
 * but a mode-0 directory cannot be entered (EACCES, root excepted); it
 * can still be removed. */
#include "a_sys.h"

#define D "/tmp/c_mkdir_perms"

static i64 mk(const char *p, i64 m) { return syscall2(SYS_mkdir, p, m); }
static i64 ch(const char *p, i64 m) { return syscall2(SYS_chmod, p, m); }
static char buf[512];

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    struct a_stat st;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_ro", mk(D "/ro", 0755), 0);
    expect("setup_ro_e", mk(D "/ro/e", 0755), 0);
    expect("chmod_ro_0555", ch(D "/ro", 0555), 0);

    expect("ro_new", mk(D "/ro/x", 0700), root ? 0 : -EACCES);
    expect("ro_new_made_only_by_root", a_lstat_path(D "/ro/x", &st), root ? 0 : -ENOENT);
    if (root) expect("root_rm_x", a_rmdir(D "/ro/x"), 0);
    expect("ro_existing", mk(D "/ro/e", 0700), -EEXIST);
    expect("ro_dot", mk(D "/ro/.", 0700), -EEXIST);
    expect("ro_missing_dir", mk(D "/ro/no/x", 0700), -ENOENT);
    expect("ro_sub_writable", mk(D "/ro/e/y", 0700), 0);
    str_cpy(buf, D "/ro/");
    memset(buf + str_len(buf), 'n', 256);
    expect("ro_long_name", mk(buf, 0700), -ENAMETOOLONG);

    /* Write and search without read is enough. */
    expect("chmod_ro_0333", ch(D "/ro", 0333), 0);
    expect("wx_only", mk(D "/ro/w", 0700), 0);
    expect("chmod_ro_0111", ch(D "/ro", 0111), 0);
    expect("x_only_new", mk(D "/ro/z", 0700), root ? 0 : -EACCES);
    if (root) expect("root_rm_z", a_rmdir(D "/ro/z"), 0);
    expect("x_only_sub_writable", mk(D "/ro/e/y2", 0700), 0);

    /* Read and write without search is not. */
    expect("chmod_ro_0666", ch(D "/ro", 0666), 0);
    expect("rw_new", mk(D "/ro/q", 0700), root ? 0 : -EACCES);
    if (root) expect("root_rm_q", a_rmdir(D "/ro/q"), 0);
    expect("rw_existing", mk(D "/ro/e", 0700), root ? -EEXIST : -EACCES);
    expect("rw_dot", mk(D "/ro/.", 0700), root ? -EEXIST : -EACCES);
    expect("rw_missing_dir", mk(D "/ro/no/x", 0700), root ? -ENOENT : -EACCES);
    expect("rw_sub_beyond", mk(D "/ro/e/y3", 0700), root ? 0 : -EACCES);
    if (root) expect("root_rm_y3", a_rmdir(D "/ro/e/y3"), 0);
    str_cpy(buf, D "/ro/");
    memset(buf + str_len(buf), 'n', 256);
    expect("rw_long_name", mk(buf, 0700), root ? -ENAMETOOLONG : -EACCES);
    expect("chmod_ro_0755", ch(D "/ro", 0755), 0);

    /* A directory made with mode 0. */
    expect("mk_mode_0", mk(D "/z0", 0), 0);
    expect("in_mode_0", mk(D "/z0/x", 0700), root ? 0 : -EACCES);
    if (root) expect("root_rm_z0_x", a_rmdir(D "/z0/x"), 0);
    expect("rmdir_mode_0", a_rmdir(D "/z0"), 0);

    expect("rm_y", a_rmdir(D "/ro/e/y"), 0);
    expect("rm_y2", a_rmdir(D "/ro/e/y2"), 0);
    expect("rm_w", a_rmdir(D "/ro/w"), 0);
    expect("rm_e", a_rmdir(D "/ro/e"), 0);
    expect("rm_ro", a_rmdir(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
