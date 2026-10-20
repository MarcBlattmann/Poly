/* geteuid: the effective user id; the tests run as root, so it is 0, on every call;
 * it never fails. */
#include "a_sys.h"

int test_main(void) {
    expect("is_root_0", syscall0(SYS_geteuid), 0);
    int same = 1;
    for (int i = 0; i < 5; i++) same &= syscall0(SYS_geteuid) == 0;
    check("stable", same, 0);
    /* Arguments are ignored. */
    expect("ignores_arguments", syscall6(SYS_geteuid, 1, 2, 3, 4, 5, 6), 0);
    return finish();
}
