/* link: the new name must not exist, whatever it is: a regular file
 * (another inode or the same one), a directory, a symbolic link (even a
 * dangling one, which is not followed), "." or "..". The existing name
 * is left unchanged. */
#include "a_sys.h"

#define D "/tmp/a_link_eexist"

int test_main(void) {
    struct a_stat a, b;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "f", 0644), 0);
    expect("setup_other", a_make_file(D "/other", "other", 0644), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "gone", D "/dl"), 0);
    expect("setup_g", syscall2(SYS_link, D "/f", D "/g"), 0);
    expect("onto_other_file", syscall2(SYS_link, D "/f", D "/other"), -EEXIST);
    expect("onto_itself", syscall2(SYS_link, D "/f", D "/f"), -EEXIST);
    expect("onto_existing_link", syscall2(SYS_link, D "/f", D "/g"), -EEXIST);
    expect("onto_directory", syscall2(SYS_link, D "/f", D "/sub"), -EEXIST);
    expect("onto_directory_slash", syscall2(SYS_link, D "/f", D "/sub/"), -EEXIST);
    expect("onto_dangling_symlink", syscall2(SYS_link, D "/f", D "/dl"), -EEXIST);
    expect("dangling_target_not_made", a_stat_path(D "/gone", &a), -ENOENT);
    expect("onto_dot", syscall2(SYS_link, D "/f", D "/sub/."), -EEXIST);
    expect("onto_dotdot", syscall2(SYS_link, D "/f", D "/sub/.."), -EEXIST);
    expect("onto_root", syscall2(SYS_link, D "/f", "/"), -EEXIST);
    a_stat_path(D "/other", &a);
    expect("other_size_kept", a.st_size, 5);
    a_stat_path(D "/f", &b);
    check("other_not_replaced", a.st_ino != b.st_ino, 0);
    expect("nlink_unchanged", b.st_nlink, 2);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_other", a_unlink(D "/other"), 0);
    expect("cleanup_dl", a_unlink(D "/dl"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
