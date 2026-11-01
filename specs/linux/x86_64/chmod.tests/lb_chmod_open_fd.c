/* chmod(2) and open descriptors: a descriptor opened before the mode
 * dropped to 0 keeps its access; later opens are checked against the new
 * mode (root passes the read/write check). Hard links share the mode. */
#include "a_sys.h"

#define D "/tmp/lb_chmod_open_fd"

static i64 chmod_(const char *p, i64 mode) { return syscall2(SYS_chmod, p, mode); }

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    struct a_stat st;
    char c[8];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);
    expect("setup_hardlink", syscall2(SYS_link, D "/f", D "/h"), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("open_rdwr", fd >= 0, fd);
    expect("chmod_0", chmod_(D "/f", 0), 0);
    expect("fstat_open_fd", syscall2(SYS_fstat, fd, &st), 0);
    expect("fstat_sees_0", st.st_mode, S_IFREG);
    expect("hardlink_sees_0", a_stat_path(D "/h", &st) == 0 && st.st_mode == S_IFREG, 1);
    expect("old_fd_reads", a_read(fd, c, 3), 3);
    expect("old_fd_writes", a_write(fd, "def", 3), 3);
    expect("new_open_rdonly", a_open(D "/f", O_RDONLY, 0) >= 0, root);
    expect("new_open_via_hardlink", a_open(D "/h", O_WRONLY, 0) >= 0, root);
    expect("close_old", a_close(fd), 0);
    /* opens made just above by root are closed by the exit */
    expect("chmod_via_hardlink", chmod_(D "/h", 0444), 0);
    expect("f_sees_0444", a_stat_path(D "/f", &st) == 0 && st.st_mode == (S_IFREG | 0444), 1);
    i64 rfd = a_open(D "/f", O_RDONLY, 0);
    check("open_rdonly_0444", rfd >= 0, rfd);
    expect("read_all", a_read(rfd, c, 8), 6);
    expect("contents", mem_eq(c, "abcdef", 6), 1);
    expect("open_wronly_0444", a_open(D "/f", O_WRONLY, 0) >= 0, root);
    expect("close_r", a_close(rfd), 0);
    expect("cleanup_h", a_unlink(D "/h"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
