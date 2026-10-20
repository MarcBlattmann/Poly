/* dup2: redirecting standard output into a file and back. */
#include "a_sys.h"

#define F "/tmp/a_dup2_stdout"

int test_main(void) {
    char buf[32];
    i64 saved = syscall1(SYS_dup, 1);
    expect("save_stdout_3", saved, 3);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("open_4", fd, 4);
    i64 r = syscall2(SYS_dup2, fd, 1);
    a_write(1, "into the file\n", 14);
    i64 back = syscall2(SYS_dup2, saved, 1);
    expect("dup2_file_onto_stdout", r, 1);
    expect("dup2_back", back, 1);
    expect("file_got_the_write", syscall4(SYS_pread64, fd, buf, sizeof buf, 0), 14);
    check("file_contents", mem_eq(buf, "into the file\n", 14), buf[0]);
    expect("file_offset_moved", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 14);
    /* dup2 onto 2 as well, then restore from the saved copy. */
    i64 saved2 = syscall1(SYS_dup, 2);
    expect("dup2_onto_stderr", syscall2(SYS_dup2, fd, 2), 2);
    a_write(2, "err\n", 4);
    expect("restore_stderr", syscall2(SYS_dup2, saved2, 2), 2);
    expect("file_got_stderr", syscall4(SYS_pread64, fd, buf, sizeof buf, 0), 18);
    a_close(saved2);
    a_close(saved);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
