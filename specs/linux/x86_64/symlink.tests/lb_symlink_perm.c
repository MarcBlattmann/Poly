/* symlink(2) needs write and search permission on the directory it
 * creates in (EACCES; root passes); an existing name is EEXIST first; in a
 * set-group-ID directory the link takes the directory's group. */
#include "a_sys.h"

#define D "/tmp/lb_symlink_perm"

static i64 symlink_(const char *t, const char *l) { return syscall2(SYS_symlink, t, l); }

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_mkdir(D "/ro", 0755), 0);
    expect("setup_ro_existing", a_make_file(D "/ro/existing", "x", 0644), 0);
    expect("setup_nox", a_mkdir(D "/nox", 0755), 0);
    expect("chmod_ro", syscall2(SYS_chmod, D "/ro", 0555), 0);
    expect("chmod_nox", syscall2(SYS_chmod, D "/nox", 0600), 0);
    expect("ro_new", symlink_("x", D "/ro/new"), root ? 0 : -EACCES);
    expect("ro_existing", symlink_("x", D "/ro/existing"), -EEXIST);
    expect("nox_new", symlink_("x", D "/nox/new"), root ? 0 : -EACCES);
    expect("chmod_wx", syscall2(SYS_chmod, D "/ro", 0300), 0);
    expect("wx_new", symlink_("x", D "/ro/new2"), 0);
    expect("chmod_ro_back", syscall2(SYS_chmod, D "/ro", 0755), 0);
    expect("chmod_nox_back", syscall2(SYS_chmod, D "/nox", 0755), 0);

    /* set-group-ID directory (the caller's own group, so any user can) */
    expect("setup_sg", a_mkdir(D "/sg", 0755), 0);
    expect("chmod_sg", syscall2(SYS_chmod, D "/sg", 02755), 0);
    expect("sg_link", symlink_("x", D "/sg/l"), 0);
    expect("sg_lstat", a_lstat_path(D "/sg/l", &st), 0);
    expect("sg_link_gid", st.st_gid, syscall0(SYS_getegid));
    expect("sg_link_mode", st.st_mode, S_IFLNK | 0777);

    /* a removed working directory */
    expect("setup_gone", a_mkdir(D "/gone", 0755), 0);
    expect("chdir_gone", syscall1(SYS_chdir, D "/gone"), 0);
    expect("rmdir_gone", a_rmdir(D "/gone"), 0);
    expect("in_removed_cwd", symlink_("x", "l"), -ENOENT);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    if (root) {
        expect("cleanup_ro_new", a_unlink(D "/ro/new"), 0);
        expect("cleanup_nox_new", a_unlink(D "/nox/new"), 0);
    }
    expect("cleanup_sg_l", a_unlink(D "/sg/l"), 0);
    expect("cleanup_sg", a_rmdir(D "/sg"), 0);
    expect("cleanup_new2", a_unlink(D "/ro/new2"), 0);
    expect("cleanup_existing", a_unlink(D "/ro/existing"), 0);
    expect("cleanup_ro", a_rmdir(D "/ro"), 0);
    expect("cleanup_nox", a_rmdir(D "/nox"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
