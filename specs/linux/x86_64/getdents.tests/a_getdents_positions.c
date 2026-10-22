/* The directory position getdents reads from and advances belongs to
 * the open file description: a dup()ed descriptor shares it, a second
 * open() of the directory has its own. A directory opened O_RDONLY
 * without O_DIRECTORY works the same. All records here are 24 bytes. */
#include "a_sys.h"

#define SYS_getdents 78

#define D "/tmp/a_getdents_positions"

static char buf[4096] __attribute__((aligned(8)));

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_make_file(D "/a", "", 0644), 0);
    expect("setup_b", a_make_file(D "/b", "", 0644), 0);

    i64 fd = a_open(D, O_RDONLY, 0);
    check("open_without_o_directory", fd >= 0, fd);
    i64 dupfd = syscall1(SYS_dup, fd);
    i64 other = a_open(D, O_RDONLY | O_DIRECTORY, 0);

    expect("first_two_via_fd", syscall3(SYS_getdents, fd, buf, 48), 48);
    expect("dup_continues_with_rest", syscall3(SYS_getdents, dupfd, buf, sizeof buf), 48);
    expect("fd_at_end_too", syscall3(SYS_getdents, fd, buf, sizeof buf), 0);
    expect("other_open_sees_all", syscall3(SYS_getdents, other, buf, sizeof buf), 96);
    expect("other_at_end", syscall3(SYS_getdents, other, buf, sizeof buf), 0);

    /* Rewinding one description leaves the other at the end. */
    expect("rewind_dup", syscall3(SYS_lseek, dupfd, 0, SEEK_SET), 0);
    expect("fd_shares_rewind", syscall3(SYS_getdents, fd, buf, 24), 24);
    expect("other_still_at_end", syscall3(SYS_getdents, other, buf, sizeof buf), 0);
    expect("dup_rest", syscall3(SYS_getdents, dupfd, buf, sizeof buf), 72);

    /* After close of one descriptor the dup keeps working. */
    a_close(fd);
    expect("rewind_after_close", syscall3(SYS_lseek, dupfd, 0, SEEK_SET), 0);
    expect("dup_after_close", syscall3(SYS_getdents, dupfd, buf, sizeof buf), 96);
    a_close(dupfd);
    a_close(other);

    expect("cleanup_a", a_unlink(D "/a"), 0);
    expect("cleanup_b", a_unlink(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
