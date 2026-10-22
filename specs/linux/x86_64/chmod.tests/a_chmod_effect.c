/* chmod changes the inode: every hard link and every open descriptor sees
 * the new mode at once (fstat), later opens are checked against it (for a
 * normal user; root reads and writes regardless), and descriptors opened
 * before keep their access. The expectations depend on the uid; the check
 * names do not. */
#include "a_sys.h"

#define D "/tmp/a_chmod_effect"

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    i64 eacces = root ? 0 : -EACCES;
    struct a_stat st;
    char c;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "ab", 0644), 0);
    expect("setup_link", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/g", 0), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("open_rw", fd >= 0, fd);
    expect("chmod_0", syscall2(SYS_chmod, D "/g", 0), 0);
    expect("fstat_sees_0", syscall2(SYS_fstat, fd, &st) == 0 ? (i64)st.st_mode : -1, S_IFREG);
    expect("other_link_sees_0", a_stat_path(D "/f", &st) == 0 ? (i64)st.st_mode : -1, S_IFREG);
    expect("old_fd_reads", a_read(fd, &c, 1), 1);
    expect("old_fd_writes", a_write(fd, "z", 1), 1);
    i64 r = a_open(D "/f", O_RDONLY, 0);
    expect("new_open_read", r < 0 ? r : 0, eacces);
    if (r >= 0) a_close(r);
    r = a_open(D "/f", O_WRONLY, 0);
    expect("new_open_write", r < 0 ? r : 0, eacces);
    if (r >= 0) a_close(r);
    expect("chmod_0400", syscall2(SYS_chmod, D "/f", 0400), 0);
    r = a_open(D "/g", O_RDONLY, 0);
    expect("read_allowed", r < 0 ? r : 0, 0);
    if (r >= 0) a_close(r);
    r = a_open(D "/g", O_WRONLY, 0);
    expect("write_refused", r < 0 ? r : 0, eacces);
    if (r >= 0) a_close(r);
    expect("chmod_0200", syscall2(SYS_chmod, D "/f", 0200), 0);
    r = a_open(D "/g", O_WRONLY, 0);
    expect("write_allowed", r < 0 ? r : 0, 0);
    if (r >= 0) a_close(r);
    r = a_open(D "/g", O_RDONLY, 0);
    expect("read_refused", r < 0 ? r : 0, eacces);
    if (r >= 0) a_close(r);
    expect("fstat_sees_0200", syscall2(SYS_fstat, fd, &st) == 0 ? (i64)st.st_mode : -1, S_IFREG | 0200);
    expect("access_r_after", syscall2(SYS_access, D "/f", R_OK), eacces);
    expect("access_w_after", syscall2(SYS_access, D "/f", W_OK), 0);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
