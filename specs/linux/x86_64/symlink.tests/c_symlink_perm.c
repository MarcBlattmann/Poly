/* symlink: permissions. Creating a name needs write and search permission
 * on the parent and search permission on every directory on the way; a
 * caller without them gets EACCES (root is not refused). An existing name
 * is EEXIST even in a directory the caller may not write; a missing
 * directory below it is ENOENT; an empty or too long target is reported
 * before any permission check. The checks print different names for root
 * and for other users, so the same user must run both sides. */
#include "a_sys.h"

#define D "/tmp/c_symlink_perm"

static char big[4200];

int test_main(void) {
    struct a_stat st;
    int root = syscall0(SYS_geteuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_mkdir(D "/ro", 0755), 0);
    expect("setup_ro_file", a_make_file(D "/ro/f", "d", 0644), 0);
    expect("setup_ro_chmod", syscall3(SYS_fchmodat, AT_FDCWD, D "/ro", 0555), 0);
    expect("setup_nox", a_mkdir(D "/nox", 0755), 0);
    expect("setup_nox_file", a_make_file(D "/nox/f", "d", 0644), 0);
    expect("setup_nox_sub", a_mkdir(D "/nox/sub", 0755), 0);
    expect("setup_nox_chmod", syscall3(SYS_fchmodat, AT_FDCWD, D "/nox", 0666), 0);

    if (root) {
        expect("root_readonly_dir_ok", syscall2(SYS_symlink, "x", D "/ro/x"), 0);
        expect("root_no_search_ok", syscall2(SYS_symlink, "x", D "/nox/x"), 0);
        expect("root_no_search_deep_ok", syscall2(SYS_symlink, "x", D "/nox/sub/x"), 0);
        expect("root_nox_existing_eexist", syscall2(SYS_symlink, "x", D "/nox/f"), -EEXIST);
        int ok = a_unlink(D "/ro/x") == 0;
        ok &= a_unlink(D "/nox/x") == 0;
        ok &= a_unlink(D "/nox/sub/x") == 0;
        check("root_cleanup", ok, 0);
    } else {
        expect("readonly_dir_eacces", syscall2(SYS_symlink, "x", D "/ro/x"), -EACCES);
        expect("no_search_eacces", syscall2(SYS_symlink, "x", D "/nox/x"), -EACCES);
        expect("no_search_deep_eacces", syscall2(SYS_symlink, "x", D "/nox/sub/x"), -EACCES);
        expect("no_search_missing_eacces", syscall2(SYS_symlink, "x", D "/nox/no/x"), -EACCES);
        expect("no_search_existing_eacces", syscall2(SYS_symlink, "x", D "/nox/f"), -EACCES);
        expect("readonly_not_made", a_lstat_path(D "/ro/x", &st), -ENOENT);
    }
    expect("readonly_existing_eexist", syscall2(SYS_symlink, "x", D "/ro/f"), -EEXIST);
    expect("readonly_dot_eexist", syscall2(SYS_symlink, "x", D "/ro/."), -EEXIST);
    expect("readonly_missing_dir_enoent", syscall2(SYS_symlink, "x", D "/ro/no/x"), -ENOENT);
    expect("readonly_empty_target_enoent", syscall2(SYS_symlink, "", D "/ro/x"), -ENOENT);
    for (int i = 0; i < 4096; i++) big[i] = 'a';
    big[4096] = 0;
    expect("readonly_long_target_enametoolong", syscall2(SYS_symlink, big, D "/ro/x"), -ENAMETOOLONG);
    expect("readonly_bad_target_efault", syscall2(SYS_symlink, (const char *)8, D "/ro/x"), -EFAULT);
    expect("no_search_empty_target_enoent", syscall2(SYS_symlink, "", D "/nox/x"), -ENOENT);

    int ok = syscall3(SYS_fchmodat, AT_FDCWD, D "/ro", 0755) == 0;
    ok &= syscall3(SYS_fchmodat, AT_FDCWD, D "/nox", 0755) == 0;
    ok &= a_unlink(D "/ro/f") == 0;
    ok &= a_unlink(D "/nox/f") == 0;
    ok &= a_rmdir(D "/nox/sub") == 0;
    ok &= a_rmdir(D "/ro") == 0;
    ok &= a_rmdir(D "/nox") == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
