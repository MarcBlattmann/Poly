/* pivot_root error paths that leave the namespace unchanged. A new_root or
 * put_old on the mount that is the root of the namespace is EBUSY (also
 * new_root = "/" and put_old = "/"); a new_root that is not the top of a
 * mount, a put_old on another mount that is not below new_root, and a
 * put_old above new_root are EINVAL; missing names are ENOENT, a file is
 * ENOTDIR, a bad pointer EFAULT, an over-long name ENAMETOOLONG.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_pivot_root_errors"
#define OTHER "/tmp/pk_pivot_root_errors_o"

static void body(const char *base) {
    char old[128], sub[128], f[128], m2[128], inner[128], p[128];
    pm_path(old, base, "old");
    pm_path(sub, base, "sub");
    pm_path(f, base, "f");
    pm_path(m2, base, "m2");
    expect("mkdir_old", pm_mkdir(old), 0);
    expect("mkdir_sub", pm_mkdir(sub), 0);
    expect("file_f", pm_touch(f), 0);
    expect("mkdir_m2", pm_mkdir(m2), 0);
    expect("mount_m2", pm_mount("m2", m2, "tmpfs", 0, 0), 0);
    expect("mkdir_inner", pm_mkdir(pm_path(inner, m2, "old")), 0);
    expect("mkdir_other", pm_mkdir(OTHER), 0);
    expect("mount_other", pm_mount("o", OTHER, "tmpfs", 0, 0), 0);
    expect("mkdir_other_old", pm_mkdir(OTHER "/old"), 0);

    /* on the namespace's root mount */
    expect("new_root_slash", pm_pivot_root("/", "/"), -EBUSY);
    expect("new_root_tmp_dir", pm_pivot_root("/tmp", "/tmp"), -EBUSY);
    expect("put_old_on_root_mount", pm_pivot_root(base, "/tmp"), -EBUSY);
    expect("put_old_slash", pm_pivot_root(base, "/"), -EBUSY);
    expect("new_root_on_root_mount_put_old_inside", pm_pivot_root("/tmp", old), -EBUSY);
    /* not the top of a mount */
    expect("new_root_not_mount_point", pm_pivot_root(sub, sub), -EINVAL);
    expect("new_root_not_mount_point_put_old_above", pm_pivot_root(sub, base), -EINVAL);
    /* put_old on another mount, not below new_root */
    expect("put_old_other_mount", pm_pivot_root(base, OTHER "/old"), -EINVAL);
    expect("put_old_other_mount_top", pm_pivot_root(base, OTHER), -EINVAL);
    /* new_root is a mount below base; put_old is above it */
    expect("put_old_above_new_root", pm_pivot_root(m2, old), -EINVAL);
    expect("put_old_above_new_root_base", pm_pivot_root(m2, base), -EINVAL);
    /* names */
    expect("new_root_missing", pm_pivot_root(pm_path(p, base, "none"), old), -ENOENT);
    expect("put_old_missing", pm_pivot_root(base, pm_path(p, base, "none")), -ENOENT);
    expect("new_root_empty", pm_pivot_root("", old), -ENOENT);
    expect("put_old_empty", pm_pivot_root(base, ""), -ENOENT);
    expect("new_root_file", pm_pivot_root(f, old), -ENOTDIR);
    expect("put_old_file", pm_pivot_root(base, f), -ENOTDIR);
    expect("new_root_null", pm_pivot_root(0, old), -EFAULT);
    expect("put_old_null", pm_pivot_root(base, 0), -EFAULT);
    expect("new_root_bad_pointer", pm_pivot_root((const char *)1, old), -EFAULT);
    char longname[5000];
    memset(longname, 'a', sizeof longname - 1);
    longname[0] = '/';
    longname[sizeof longname - 1] = 0;
    expect("new_root_name_too_long", pm_pivot_root(longname, old), -ENAMETOOLONG);
    /* the namespace is untouched */
    expect("tmp_still_there", pm_exists("/tmp"), 0);
    expect("base_file_still_there", pm_exists(f), 0);
    expect("proc_still_there", pm_exists("/proc"), 0);
}

int test_main(void) {
    int r = pm_main(BASE, body);
    a_rmdir(OTHER);
    return r;
}
