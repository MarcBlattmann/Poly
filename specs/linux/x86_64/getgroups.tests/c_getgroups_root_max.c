/* 65536 groups (NGROUPS_MAX) can be set and read back in full, in
 * ascending order; size 65535 is EINVAL, 65536 works.
 * poly-diff: needs=root (setgroups needs CAP_SETGID)
 */
#include "poly_ipc.h"

static unsigned set[65536], got[65540];

int test_main(void) {
    for (unsigned i = 0; i < 65536; i++) set[i] = 65535 - i + 100000;
    i64 r = syscall2(SYS_setgroups, 65536, set);
    check("set_max_ok", r == 0, r);
    r = syscall2(SYS_getgroups, 0, 0);
    check("count_max", r == 65536, r);
    r = syscall2(SYS_getgroups, 65535, got);
    check("size_one_short_einval", r == -EINVAL, r);
    for (int i = 0; i < 65540; i++) got[i] = 0x33333333;
    r = syscall2(SYS_getgroups, 65536, got);
    check("returns_max", r == 65536, r);
    int ok = 1;
    for (unsigned i = 0; i < 65536; i++)
        if (got[i] != 100000 + i) ok = 0;
    check("all_ascending", ok, (i64)got[0]);
    check("tail_untouched", got[65536] == 0x33333333, (i64)got[65536]);
    r = syscall2(SYS_getgroups, 0x7fffffff, got);
    check("int_max_size_ok", r == 65536, r);
    return finish();
}
