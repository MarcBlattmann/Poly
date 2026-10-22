/* link: other kinds of files take more names too: an empty file, a FIFO
 * (it stays a FIFO), a file with mode 0 that the caller owns, and a file
 * that is still open (the open descriptor sees the new link count). */
#include "a_sys.h"

#define D "/tmp/a_link_kinds"
/* asm/unistd_64.h */
#define SYS_mknodat 259

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_empty", a_make_file(D "/empty", "", 0644), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    expect("setup_mode0", a_make_file(D "/zero", "z", 0), 0);
    expect("link_empty", syscall2(SYS_link, D "/empty", D "/empty2"), 0);
    expect("link_fifo", syscall2(SYS_link, D "/fifo", D "/fifo2"), 0);
    a_lstat_path(D "/fifo2", &st);
    check("fifo_kind", (st.st_mode & S_IFMT) == S_IFIFO, st.st_mode);
    expect("fifo_nlink", st.st_nlink, 2);
    expect("link_mode0", syscall2(SYS_link, D "/zero", D "/zero2"), 0);
    a_lstat_path(D "/zero2", &st);
    expect("mode0_kept", st.st_mode & 07777, 0);
    i64 fd = a_open(D "/empty", O_RDONLY, 0);
    expect("link_while_open", syscall2(SYS_link, D "/empty", D "/empty3"), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("fstat_sees_nlink_3", st.st_nlink, 3);
    a_unlink(D "/empty");
    a_unlink(D "/empty2");
    a_unlink(D "/empty3");
    syscall2(SYS_fstat, fd, &st);
    expect("fstat_nlink_0_after_unlinks", st.st_nlink, 0);
    a_close(fd);
    const char *names[] = {D "/fifo", D "/fifo2", D "/zero", D "/zero2"};
    int ok = 1;
    for (int i = 0; i < 4; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
