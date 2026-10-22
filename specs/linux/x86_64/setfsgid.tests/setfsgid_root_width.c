/* The gid is 32 bits: upper register bits are dropped. 0xfffffffe is a
 * valid gid and comes back as the unsigned value 4294967294, not -2.
 * poly-diff: needs=root (setting arbitrary gids needs CAP_SETGID)
 */
#include "poly_ipc.h"

int test_main(void) {
    i64 v = syscall1(SYS_setfsgid, (i64)0x10000004dL);
    check("upper_bits_dropped_returns_old", v == 0, v);
    v = syscall1(SYS_setfsgid, -1);
    check("set_to_77", v == 77, v);
    v = syscall1(SYS_setfsgid, (i64)0xabcd0000ffffffffL);
    check("upper_bits_minus_one_reads", v == 77, v);
    check("still_77", syscall1(SYS_setfsgid, -1) == 77, 0);
    syscall1(SYS_setfsgid, 0xfffffffeL);
    v = syscall1(SYS_setfsgid, -1);
    check("fffffffe_returned_unsigned", v == 4294967294L, v);
    v = syscall1(SYS_setfsgid, 0x80000000L);
    check("returns_fffffffe_again", v == 4294967294L, v);
    v = syscall1(SYS_setfsgid, 0);
    check("high_bit_gid_unsigned", v == 2147483648L, v);
    return finish();
}
