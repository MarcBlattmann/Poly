/* setgroups and getgroups: setgroups(n, list) replaces the supplementary
 * groups (CAP_SETGID needed: EPERM without it); getgroups reports them
 * in ascending order with duplicates kept; getgroups(0, NULL) returns
 * the count; a buffer smaller than the count is EINVAL; setgroups(0) clears them; a negative count or one above
 * 65536 is EINVAL; a bad pointer is EFAULT. */
#include "poly_ipc.h"

int test_main(void) {
    unsigned list[4] = { 30, 10, 20, 10 };
    i64 r = syscall2(SYS_setgroups, 4, list);
    check("setgroups_ok", r == 0, r);
    r = syscall2(SYS_getgroups, 0, 0);
    check("count_query", r == 4, r);
    unsigned got[8];
    memset(got, 0xff, sizeof got);
    r = syscall2(SYS_getgroups, 8, got);
    check("getgroups_returns_count", r == 4, r);
    check("groups_sorted_duplicates_kept", got[0] == 10 && got[1] == 10 && got[2] == 20 && got[3] == 30, got[0]);
    check("rest_untouched", got[4] == 0xffffffff, (i64)got[4]);
    r = syscall2(SYS_getgroups, 3, got);
    check("small_buffer_einval", r == -EINVAL, r);
    r = syscall2(SYS_getgroups, -1, got);
    check("negative_size_einval", r == -EINVAL, r);
    r = syscall2(SYS_getgroups, 8, 8);
    check("bad_buffer_efault", r == -EFAULT, r);

    r = syscall2(SYS_setgroups, -1, list);
    check("set_negative_einval", r == -EINVAL, r);
    r = syscall2(SYS_setgroups, 65537, list);
    check("set_too_many_einval", r == -EINVAL, r);
    r = syscall2(SYS_setgroups, 2, 8);
    check("set_bad_pointer_efault", r == -EFAULT, r);

    syscall3(SYS_setresuid, -1, 1000, -1);
    r = syscall2(SYS_setgroups, 1, list);
    check("without_cap_setgid_eperm", r == -EPERM, r);
    r = syscall2(SYS_getgroups, 0, 0);
    check("getgroups_needs_no_privilege", r == 4, r);
    syscall3(SYS_setresuid, -1, 0, -1);

    r = syscall2(SYS_setgroups, 0, 0);
    check("clear_ok", r == 0, r);
    r = syscall2(SYS_getgroups, 0, 0);
    check("cleared_count_zero", r == 0, r);
    return finish();
}
