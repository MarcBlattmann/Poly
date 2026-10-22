/* link: both names must be on the same mount: /tmp to /dev/shm (another
 * file system) is EXDEV, either way round. An existing new name is
 * reported first (EEXIST); EXDEV comes before a permission problem with
 * the new directory and before refusing a directory as the old name. */
#include "a_sys.h"

#define D "/tmp/a_link_xdev"
#define S "/dev/shm/a_link_xdev"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("setup_shm_dir", a_mkdir(S, 0755), 0);
    expect("setup_shm_f", a_make_file(S "/f", "", 0644), 0);
    expect("setup_shm_ro", a_mkdir(S "/ro", 0555), 0);
    expect("tmp_to_shm", syscall2(SYS_link, D "/f", S "/x"), -EXDEV);
    expect("shm_to_tmp", syscall2(SYS_link, S "/f", D "/x"), -EXDEV);
    expect("exists_beats_xdev", syscall2(SYS_link, D "/f", S "/f"), -EEXIST);
    expect("xdev_beats_directory", syscall2(SYS_link, D "/sub", S "/x"), -EXDEV);
    expect("xdev_beats_readonly_dir", syscall2(SYS_link, D "/f", S "/ro/x"), -EXDEV);
    expect("same_mount_shm", syscall2(SYS_link, S "/f", S "/g"), 0);
    expect("nothing_made_shm", a_lstat_path(S "/x", &st), -ENOENT);
    expect("nothing_made_tmp", a_lstat_path(D "/x", &st), -ENOENT);
    a_stat_path(D "/f", &st);
    expect("tmp_nlink_1", st.st_nlink, 1);
    a_stat_path(S "/f", &st);
    expect("shm_nlink_2", st.st_nlink, 2);
    expect("cleanup_shm_f", a_unlink(S "/f"), 0);
    expect("cleanup_shm_g", a_unlink(S "/g"), 0);
    expect("cleanup_shm_ro", a_rmdir(S "/ro"), 0);
    expect("cleanup_shm_dir", a_rmdir(S), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
