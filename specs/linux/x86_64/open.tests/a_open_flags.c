/* open(2): status flags F_GETFL reports, O_CLOEXEC, unknown flag bits and
 * the upper half of the flags and mode registers. */
#include "a_sys.h"

#define SYS_open 2
#define D "/tmp/a_open_flags"

static i64 op(const char *p, i64 flags, i64 mode) { return syscall3(SYS_open, p, flags, mode); }

static i64 getfl(i64 fd) { return syscall3(SYS_fcntl, fd, F_GETFL, 0); }
static i64 getfd(i64 fd) { return syscall3(SYS_fcntl, fd, F_GETFD, 0); }

int test_main(void) {
    i64 fd;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0600), 0);

    fd = op(D "/f", O_RDONLY, 0);
    expect("rdonly_getfl", getfl(fd), O_RDONLY | O_LARGEFILE);
    expect("rdonly_no_cloexec", getfd(fd), 0);
    a_close(fd);
    fd = op(D "/f", O_WRONLY, 0);
    expect("wronly_getfl", getfl(fd), O_WRONLY | O_LARGEFILE);
    a_close(fd);
    fd = op(D "/f", O_RDWR, 0);
    expect("rdwr_getfl", getfl(fd), O_RDWR | O_LARGEFILE);
    a_close(fd);
    fd = op(D "/f", 3, 0);
    expect("accmode3_getfl", getfl(fd), 3 | O_LARGEFILE);
    a_close(fd);

    fd = op(D "/f", O_RDWR | O_APPEND | O_NONBLOCK, 0);
    expect("append_nonblock_getfl", getfl(fd), O_RDWR | O_APPEND | O_NONBLOCK | O_LARGEFILE);
    a_close(fd);

    /* Open-time flags are not reported. */
    fd = op(D "/f", O_RDWR | O_CREAT | O_TRUNC | O_NOCTTY, 0600);
    expect("creat_trunc_noctty_not_reported", getfl(fd), O_RDWR | O_LARGEFILE);
    a_close(fd);
    fd = op(D "/f", O_RDWR | O_DSYNC, 0);
    expect("dsync_reported", getfl(fd), O_RDWR | O_DSYNC | O_LARGEFILE);
    a_close(fd);

    /* O_CLOEXEC sets FD_CLOEXEC and is not a status flag. */
    fd = op(D "/f", O_RDONLY | O_CLOEXEC, 0);
    expect("cloexec_fd", fd, 3);
    expect("cloexec_getfd", getfd(fd), FD_CLOEXEC);
    expect("cloexec_getfl", getfl(fd), O_RDONLY | O_LARGEFILE);
    a_close(fd);

    /* O_LARGEFILE given explicitly changes nothing. */
    fd = op(D "/f", O_RDONLY | O_LARGEFILE, 0);
    expect("largefile_getfl", getfl(fd), O_RDONLY | O_LARGEFILE);
    a_close(fd);

    /* Unknown flag bits are ignored. */
    fd = op(D "/f", O_RDONLY | 0x40000000, 0);
    expect("unknown_bit_30_opens", fd, 3);
    expect("unknown_bit_30_getfl", getfl(fd), O_RDONLY | O_LARGEFILE);
    a_close(fd);

    /* flags and mode are 32-bit: the upper half of the register is ignored. */
    fd = syscall3(SYS_open, D "/f", (i64)0xffffffff00000000UL | O_RDWR, 0);
    expect("flags_upper_half_ignored", fd, 3);
    expect("flags_upper_half_getfl", getfl(fd), O_RDWR | O_LARGEFILE);
    a_close(fd);
    fd = syscall3(SYS_open, D "/g", O_WRONLY | O_CREAT | O_EXCL, (i64)((0x12345678UL << 32) | 0600));
    expect("mode_upper_half_fd", fd, 3);
    a_close(fd);
    struct a_stat st;
    a_stat_path(D "/g", &st);
    expect("mode_upper_half_ignored", st.st_mode, S_IFREG | 0600);

    /* A new open has its own status flags; F_SETFL on it leaves others. */
    fd = op(D "/f", O_RDWR, 0);
    i64 fd2 = op(D "/f", O_RDWR, 0);
    expect("setfl_append", syscall3(SYS_fcntl, fd, F_SETFL, O_APPEND), 0);
    expect("setfl_seen_on_first", getfl(fd), O_RDWR | O_APPEND | O_LARGEFILE);
    expect("setfl_not_seen_on_second", getfl(fd2), O_RDWR | O_LARGEFILE);
    a_close(fd);
    a_close(fd2);

    expect("cleanup_unlink_f", a_unlink(D "/f"), 0);
    expect("cleanup_unlink_g", a_unlink(D "/g"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
