/* link: a new name in a directory that has been removed (the working
 * directory, after rmdir) is ENOENT; the old name can still be reached
 * by an absolute path. Linking from a removed working directory's "."
 * cannot find names either. */
#include "a_sys.h"

#define D "/tmp/a_link_removed_dir"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_gone", a_mkdir(D "/gone", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("chdir_gone", syscall1(SYS_chdir, D "/gone"), 0);
    expect("rmdir_gone", a_rmdir(D "/gone"), 0);
    expect("new_in_removed_cwd", syscall2(SYS_link, D "/f", "x"), -ENOENT);
    expect("new_dot_in_removed_cwd", syscall2(SYS_link, D "/f", "./x"), -ENOENT);
    expect("source_in_removed_cwd", syscall2(SYS_link, "f", D "/x"), -ENOENT);
    expect("dotdot_still_works", syscall2(SYS_link, "../f", D "/y"), 0);
    a_stat_path(D "/f", &st);
    expect("nlink_2", st.st_nlink, 2);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_y", a_unlink(D "/y"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
