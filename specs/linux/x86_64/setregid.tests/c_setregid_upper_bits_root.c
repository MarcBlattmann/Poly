/* With CAP_SETGID too, only the low 32 bits of each argument count:
 * 0x1_0000_0005 sets gid 5 and 0x1_ffff_ffff keeps the field (-1).
 * poly-diff: needs=root (sets gids the process does not have)
 */
#include "poly_ipc.h"

int test_main(void) {
    unsigned r, e, s;
    syscall3(SYS_setresgid, 1, 2, 3);
    i64 rc = syscall2(SYS_setregid, 0x100000005L, 0x1FFFFFFFFL);
    check("real_low_bits_ok", rc == 0, rc);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("real_is_5_effective_kept", r == 5 && e == 2 && s == 2, (i64)r);
    rc = syscall2(SYS_setregid, 0xFFFFFFFF00000000L | 0xFFFFFFFFL, 0x200000007L);
    check("effective_low_bits_ok", rc == 0, rc);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("effective_is_7_real_kept", r == 5 && e == 7 && s == 7, (i64)e);
    return finish();
}
