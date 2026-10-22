/* After setgroups with an unsorted list, getgroups reports the groups in
 * ascending unsigned order with duplicates kept: 0 first, 4294967294
 * last. The count query agrees.
 * poly-diff: needs=root (setgroups needs CAP_SETGID)
 */
#include "poly_ipc.h"

int test_main(void) {
    unsigned set[7] = { 500, 0xfffffffe, 7, 0, 500, 0x80000000, 3 };
    i64 r = syscall2(SYS_setgroups, 7, set);
    check("setgroups_ok", r == 0, r);
    r = syscall2(SYS_getgroups, 0, 0);
    check("count_seven", r == 7, r);
    unsigned got[10];
    for (int i = 0; i < 10; i++) got[i] = 0x77777777;
    r = syscall2(SYS_getgroups, 10, got);
    check("returns_seven", r == 7, r);
    unsigned want[7] = { 0, 3, 7, 500, 500, 0x80000000, 0xfffffffe };
    check("ascending_unsigned_dups_kept", mem_eq(got, want, sizeof want), (i64)got[0]);
    check("tail_untouched", got[7] == 0x77777777 && got[9] == 0x77777777, (i64)got[7]);
    r = syscall2(SYS_getgroups, 6, got);
    check("size_six_einval", r == -EINVAL, r);
    r = syscall2(SYS_getgroups, 7, got);
    check("exact_size_ok", r == 7, r);
    return finish();
}
