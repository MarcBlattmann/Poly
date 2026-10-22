/* With no supplementary groups (setgroups(0, NULL)) getgroups returns 0
 * for every nonnegative size and never touches the buffer, so even a bad
 * pointer is fine. A negative size is still EINVAL.
 * poly-diff: needs=root (setgroups needs CAP_SETGID)
 */
#include "poly_ipc.h"

int test_main(void) {
    i64 r = syscall2(SYS_setgroups, 0, 0);
    check("clear_ok", r == 0, r);
    r = syscall2(SYS_getgroups, 0, 0);
    check("count_zero", r == 0, r);
    unsigned buf[4] = { 9, 9, 9, 9 };
    r = syscall2(SYS_getgroups, 4, buf);
    check("returns_zero", r == 0, r);
    check("buffer_untouched", buf[0] == 9 && buf[3] == 9, (i64)buf[0]);
    r = syscall2(SYS_getgroups, 1, 8);
    check("bad_pointer_no_efault", r == 0, r);
    r = syscall2(SYS_getgroups, 65536, 0);
    check("null_large_size_zero", r == 0, r);
    r = syscall2(SYS_getgroups, -1, buf);
    check("negative_still_einval", r == -EINVAL, r);
    return finish();
}
