/* link: relative names start at the working directory, each name on its
 * own; "." and ".." components work. */
#include "a_sys.h"

#define D "/tmp/a_link_relative"

int test_main(void) {
    struct a_stat a, b;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("chdir", syscall1(SYS_chdir, D), 0);
    expect("both_relative", syscall2(SYS_link, "f", "g"), 0);
    expect("into_subdir", syscall2(SYS_link, "f", "sub/h"), 0);
    expect("dot_components", syscall2(SYS_link, "./sub/../f", "./sub/./i"), 0);
    expect("chdir_sub", syscall1(SYS_chdir, D "/sub"), 0);
    expect("dotdot_source", syscall2(SYS_link, "../g", "j"), 0);
    expect("absolute_and_relative", syscall2(SYS_link, D "/f", "../k"), 0);
    expect("relative_missing", syscall2(SYS_link, "f", "z"), -ENOENT);
    a_stat_path(D "/f", &a);
    expect("nlink_6", a.st_nlink, 6);
    const char *names[] = {D "/g", D "/sub/h", D "/sub/i", D "/sub/j", D "/k"};
    int ok = 1;
    for (int i = 0; i < 5; i++) {
        ok &= a_stat_path(names[i], &b) == 0 && b.st_ino == a.st_ino;
    }
    check("all_same_inode", ok, 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    ok = 1;
    for (int i = 0; i < 5; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
