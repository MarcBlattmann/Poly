/* unlinkat: relative to a directory descriptor, bad descriptors; the
 * link count of a file with several names. */
#include "a_sys.h"

#define D "/tmp/a_unlinkat_dirfd"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("link_1", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/f1", 0), 0);
    expect("link_2", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/f2", 0), 0);
    i64 keep = a_open(D "/f", O_RDONLY, 0);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("unlink_relative", syscall3(SYS_unlinkat, dfd, "f", 0), 0);
    syscall2(SYS_fstat, keep, &st);
    expect("nlink_2_after_one", st.st_nlink, 2);
    expect("unlink_relative_f1", syscall3(SYS_unlinkat, dfd, "f1", 0), 0);
    syscall2(SYS_fstat, keep, &st);
    expect("nlink_1_after_two", st.st_nlink, 1);
    expect("f2_still_reads", a_stat_path(D "/f2", &st), 0);
    expect("bad_dirfd_ebadf", syscall3(SYS_unlinkat, 99, "f2", 0), -EBADF);
    i64 ffd = a_open(D "/f2", O_RDONLY, 0);
    expect("file_dirfd_enotdir", syscall3(SYS_unlinkat, ffd, "x", 0), -ENOTDIR);
    a_close(ffd);
    expect("unlink_absolute_bad_dirfd", syscall3(SYS_unlinkat, 99, D "/f2", 0), 0);
    syscall2(SYS_fstat, keep, &st);
    expect("nlink_0_last_name_gone", st.st_nlink, 0);
    a_close(keep);
    expect("mkdir_sub", syscall3(SYS_mkdirat, dfd, "sub", 0700), 0);
    expect("rmdir_relative", syscall3(SYS_unlinkat, dfd, "sub", AT_REMOVEDIR), 0);
    a_close(dfd);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
