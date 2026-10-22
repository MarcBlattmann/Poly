/* lchown(2) needs search (x) permission on every directory on the way to
 * the name: without it EACCES, even when the name does not exist or the
 * ids would be EPERM. Read permission on the directory is not needed, and
 * no permission on the file itself. Root (CAP_DAC_OVERRIDE) passes the
 * search check. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define D "/tmp/a_lchown_eacces"

static i64 lchown(const char *p, i64 uid, i64 gid) { return syscall3(SYS_lchown, p, uid, gid); }

static int root;
/* Root expects `root_want`, everyone else `want`. */
static void as(const char *name, i64 got, i64 want, i64 root_want) { check(name, got == (root ? root_want : want), got); }

int test_main(void) {
    root = syscall0(SYS_geteuid) == 0;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_inner", a_mkdir(D "/in", 0755), 0);
    expect("setup_file", a_make_file(D "/in/f", "", 0644), 0);
    expect("setup_deep", a_mkdir(D "/in/deep", 0755), 0);
    expect("setup_deep_file", a_make_file(D "/in/deep/g", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "in/f", AT_FDCWD, D "/l"), 0);

    expect("no_search", syscall2(SYS_chmod, D "/in", 0644), 0);
    as("no_search_file_eacces", lchown(D "/in/f", uid, gid), -EACCES, 0);
    as("no_search_missing_eacces", lchown(D "/in/nope", -1, -1), -EACCES, -ENOENT);
    as("no_search_deeper_eacces", lchown(D "/in/deep/g", -1, -1), -EACCES, 0);
    as("no_search_other_ids_eacces", lchown(D "/in/f", 4242, 4242), -EACCES, 0);
    as("no_search_trailing_slash_eacces", lchown(D "/in/deep/", -1, -1), -EACCES, 0);
    /* The directory itself is reached through D, which is searchable. */
    expect("no_search_dir_itself", lchown(D "/in", -1, -1), 0);
    /* Only the last component of the link is not followed: the link
     * itself is reachable, its target is not needed. */
    expect("link_into_unsearchable", lchown(D "/l", -1, -1), 0);

    expect("search_only", syscall2(SYS_chmod, D "/in", 0111), 0);
    expect("search_only_file", lchown(D "/in/f", uid, gid), 0);
    expect("search_only_deeper", lchown(D "/in/deep/g", -1, -1), 0);
    expect("search_only_missing_enoent", lchown(D "/in/nope", -1, -1), -ENOENT);

    expect("dir_mode_0", syscall2(SYS_chmod, D "/in", 0), 0);
    expect("mode_0_dir_itself", lchown(D "/in", uid, gid), 0);
    as("mode_0_dir_dot_eacces", lchown(D "/in/.", -1, -1), -EACCES, 0);

    expect("restore", syscall2(SYS_chmod, D "/in", 0755), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_deep_g", a_unlink(D "/in/deep/g"), 0);
    expect("cleanup_deep", a_rmdir(D "/in/deep"), 0);
    expect("cleanup_f", a_unlink(D "/in/f"), 0);
    expect("cleanup_in", a_rmdir(D "/in"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
