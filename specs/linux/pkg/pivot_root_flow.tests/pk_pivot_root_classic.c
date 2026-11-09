/* pivot_root(new_root, put_old) with a directory below new_root as the
 * place for the old root: new_root must be a mount point (a tmpfs here).
 * Afterwards new_root is "/", the old root is reachable at put_old, and
 * after umount2(put_old, MNT_DETACH) put_old is an empty directory that can
 * be removed. A process whose working directory is the old root's top
 * directory ("/" at the time of the call, like this one) is moved to the
 * new root: getcwd gives "/" and relative names resolve there.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_pivot_root_classic"

static void body(const char *base) {
    char p[128], q[128];
    expect("mkdir_old", pm_mkdir(pm_path(p, base, "old")), 0);
    expect("file_f", a_make_file(pm_path(p, base, "f"), "x", 0644), 0);
    expect("cwd_is_old_root", syscall1(SYS_chdir, "/"), 0);
    expect("pivot_root", pm_pivot_root(base, pm_path(q, base, "old")), 0);
    expect("new_root_visible", pm_exists("/f"), 0);
    expect("old_tmp_gone", pm_exists("/tmp"), -ENOENT);
    expect("old_root_at_put_old_tmp", pm_exists("/old/tmp"), 0);
    expect("old_root_at_put_old_base", pm_exists("/old" BASE), 0);
    expect("old_root_proc", pm_exists("/old/proc"), 0);
    char cwd[64];
    memset(cwd, 0, sizeof cwd);
    i64 n = syscall2(SYS_getcwd, cwd, sizeof cwd);
    check("cwd_moved_to_new_root", n > 0 && str_eq(cwd, "/"), n);
    expect("relative_f", pm_exists("f"), 0);
    expect("detach_put_old", pm_umount2("/old", MNT_DETACH), 0);
    expect("old_gone", pm_exists("/old/tmp"), -ENOENT);
    expect("put_old_remains_dir", pm_exists("/old"), 0);
    expect("rmdir_put_old", a_rmdir("/old"), 0);
    expect("put_old_removed", pm_exists("/old"), -ENOENT);
}

int test_main(void) { return pm_main(BASE, body); }
