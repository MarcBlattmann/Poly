/* write error cases: an invalid buffer is EFAULT, a closed descriptor is
 * EBADF, a zero-length write is 0; an unknown syscall number is ENOSYS. */
#include "poly_test.h"

int test_main(void) {
    i64 r = syscall3(SYS_write, 1, 8, 5);
    check("write_invalid_pointer_efault", r == -EFAULT, r);
    r = syscall3(SYS_write, 1, 0xffff800000000000UL, 5);
    check("write_kernel_pointer_efault", r == -EFAULT, r);
    r = syscall3(SYS_write, 1000, "x", 1);
    check("write_closed_fd_ebadf", r == -EBADF, r);
    r = syscall3(SYS_write, -1, "x", 1);
    check("write_negative_fd_ebadf", r == -EBADF, r);
    r = syscall3(SYS_write, 1, 8, 0);
    check("write_zero_length_returns_zero", r == 0, r);
    r = syscall0(1000);
    check("unknown_syscall_enosys", r == -ENOSYS, r);
    return finish();
}
