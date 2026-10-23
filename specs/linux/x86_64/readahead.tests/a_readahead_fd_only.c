/* Only the open descriptor matters: the call still works after the
 * file was made unreadable, renamed or unlinked, on dups, and on an
 * empty file. */
#include "a_sys.h"

#define SYS_readahead 187

static i64 ra(i64 fd, i64 off, i64 count) { return syscall3(SYS_readahead, fd, off, count); }

#define F "/tmp/a_readahead_fd_only"
#define G "/tmp/a_readahead_fd_only_renamed"
#define E "/tmp/a_readahead_fd_only_empty"

int test_main(void) {
    expect("make", a_make_file(F, "fd only", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("chmod_000", syscall2(SYS_chmod, F, 0), 0);
    expect("mode_000", ra(fd, 0, 7), 0);
    expect("rename", syscall2(SYS_rename, F, G), 0);
    expect("renamed", ra(fd, 0, 7), 0);
    expect("unlink", a_unlink(G), 0);
    expect("unlinked", ra(fd, 0, 7), 0);
    i64 d = syscall1(SYS_dup, fd);
    i64 d2 = syscall3(SYS_fcntl, fd, F_DUPFD_CLOEXEC, 50);
    expect("dupfd_50", d2, 50);
    expect("close_original", a_close(fd), 0);
    expect("dup", ra(d, 0, 7), 0);
    expect("dupfd_cloexec", ra(d2, 0, 7), 0);
    a_close(d);
    a_close(d2);
    i64 e = a_open(E, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("empty", ra(e, 0, 1 << 20), 0);
    struct a_stat st;
    expect("fstat", syscall2(SYS_fstat, e, &st), 0);
    expect("empty_size_0", st.st_size, 0);
    a_close(e);
    expect("cleanup", a_unlink(E), 0);
    return finish();
}
