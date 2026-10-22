/* fchmod: the descriptor is a 32-bit value; the upper half of the
 * register is ignored, so (1 << 32) + fd names fd, and (1 << 32) - 1 is
 * fd -1 (EBADF). */
#include "a_sys.h"

#define F "/tmp/c_fchmod_fd_register"

int test_main(void) {
    struct a_stat st;
    a_unlink(F);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_EXCL, 0600);
    check("setup_open", fd >= 0, fd);
    expect("upper_half_ignored", syscall2(SYS_fchmod, (i64)0x100000000L + fd, 0614), 0);
    expect("fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("mode_set_through_fd", st.st_mode, S_IFREG | 0614);
    expect("all_upper_bits", syscall2(SYS_fchmod, (i64)0xffffffff00000000L | fd, 0641), 0);
    expect("fstat_2", syscall2(SYS_fstat, fd, &st), 0);
    expect("mode_set_again", st.st_mode, S_IFREG | 0641);
    expect("low_half_minus_1", syscall2(SYS_fchmod, 0xffffffffL, 0600), -EBADF);
    expect("upper_half_plus_99", syscall2(SYS_fchmod, (i64)0x100000000L + 99, 0600), -EBADF);
    expect("fstat_3", syscall2(SYS_fstat, fd, &st), 0);
    expect("mode_unchanged", st.st_mode, S_IFREG | 0641);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
