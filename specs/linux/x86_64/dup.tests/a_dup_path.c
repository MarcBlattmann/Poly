/* dup: of directory and O_PATH descriptors; the copy stays valid after
 * the original is closed and the file unlinked. */
#include "a_sys.h"

#define D "/tmp/a_dup_path"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "12345", 0644), 0);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i64 dd = syscall1(SYS_dup, dfd);
    expect("dup_dir_4", dd, 4);
    i64 fd = syscall4(SYS_openat, dd, "f", O_RDONLY, 0);
    expect("openat_via_dup_dir", fd, 5);
    a_close(fd);
    i64 p = a_open(D "/f", O_PATH, 0);
    expect("open_path_5", p, 5);
    i64 pd = syscall1(SYS_dup, p);
    expect("dup_path_6", pd, 6);
    expect("dup_path_getfl", syscall3(SYS_fcntl, pd, F_GETFL, 0), O_PATH);
    expect("dup_path_fstat", syscall2(SYS_fstat, pd, &st), 0);
    expect("dup_path_size", st.st_size, 5);
    a_close(p);
    a_close(pd);

    fd = a_open(D "/f", O_RDONLY, 0);
    i64 copy = syscall1(SYS_dup, fd);
    a_close(fd);
    expect("unlink", a_unlink(D "/f"), 0);
    char buf[8];
    expect("copy_reads_unlinked", a_read(copy, buf, 8), 5);
    a_close(copy);
    a_close(dd);
    a_close(dfd);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
