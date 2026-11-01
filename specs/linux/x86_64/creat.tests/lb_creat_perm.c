/* creat(2) permissions: a new name needs write and search permission on
 * the directory, an existing file write permission on the file (O_TRUNC);
 * root passes both; a failed truncation leaves the contents alone. */
#include "a_sys.h"

#define D "/tmp/lb_creat_perm"

static i64 creat_(const char *p, i64 mode) { return syscall2(85, p, mode); }

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_mkdir(D "/ro", 0755), 0);
    expect("setup_ro_f", a_make_file(D "/ro/f", "keep", 0644), 0);
    expect("setup_ro_w", a_make_file(D "/ro/w", "gone", 0666), 0);
    expect("chmod_w", syscall2(SYS_chmod, D "/ro/w", 0666), 0);
    expect("chmod_ro", syscall2(SYS_chmod, D "/ro", 0555), 0);

    i64 fd = creat_(D "/ro/new", 0644);
    expect("ro_dir_new", fd >= 0 ? 0 : fd, root ? 0 : -EACCES);
    if (fd >= 0) a_close(fd);
    /* an existing writable file in a read-only directory: fine */
    fd = creat_(D "/ro/w", 0644);
    check("ro_dir_existing_writable", fd >= 0, fd);
    if (fd >= 0) a_close(fd);
    expect("w_truncated", a_stat_path(D "/ro/w", &st) == 0 && st.st_size == 0, 1);
    /* an existing file without write permission */
    expect("chmod_f_0444", syscall2(SYS_chmod, D "/ro/f", 0444), 0);
    fd = creat_(D "/ro/f", 0644);
    expect("readonly_file", fd >= 0 ? 0 : fd, root ? 0 : -EACCES);
    if (fd >= 0) a_close(fd);
    expect("f_size", a_stat_path(D "/ro/f", &st) == 0 ? st.st_size : -1, root ? 0 : 4);
    /* no search permission */
    expect("chmod_ro_0600", syscall2(SYS_chmod, D "/ro", 0600), 0);
    fd = creat_(D "/ro/w", 0644);
    expect("no_search", fd >= 0 ? 0 : fd, root ? 0 : -EACCES);
    if (fd >= 0) a_close(fd);
    expect("chmod_ro_back", syscall2(SYS_chmod, D "/ro", 0755), 0);

    /* a removed working directory */
    expect("setup_gone", a_mkdir(D "/gone", 0755), 0);
    expect("chdir_gone", syscall1(SYS_chdir, D "/gone"), 0);
    expect("rmdir_gone", a_rmdir(D "/gone"), 0);
    expect("in_removed_cwd", creat_("x", 0644), -ENOENT);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    if (root) expect("cleanup_new", a_unlink(D "/ro/new"), 0);
    expect("cleanup_w", a_unlink(D "/ro/w"), 0);
    expect("cleanup_f", a_unlink(D "/ro/f"), 0);
    expect("cleanup_ro", a_rmdir(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
