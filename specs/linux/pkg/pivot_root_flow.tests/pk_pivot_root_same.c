/* pivot_root(new_root, new_root): put_old may be new_root itself. The old
 * root is then stacked on new_root and hidden by the new tree; after the
 * call the new root's files are "/" and the old tree (/tmp, and the mount
 * point of new_root in it) is not reachable by name. The working directory
 * "/" (the old root's top) is moved to the new root.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_pivot_root_same"

static void body(const char *base) {
    expect("file_g", a_make_file("/tmp/pk_pivot_root_same/g", "y", 0644), 0);
    expect("pivot_root_same", pm_pivot_root(base, base), 0);
    expect("new_root_visible", pm_exists("/g"), 0);
    expect("old_tmp_hidden", pm_exists("/tmp"), -ENOENT);
    expect("old_mount_point_hidden", pm_exists("/pk_pivot_root_same"), -ENOENT);
    char cwd[64];
    memset(cwd, 0, sizeof cwd);
    i64 n = syscall2(SYS_getcwd, cwd, sizeof cwd);
    check("cwd_is_new_root", n > 0 && str_eq(cwd, "/"), n);
    expect("relative_g", pm_exists("g"), 0);
}

int test_main(void) { return pm_main(BASE, body); }
