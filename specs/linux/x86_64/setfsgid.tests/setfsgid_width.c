/* The argument is a 32-bit gid: the upper half of the register is ignored,
 * so 0xffffffff in the low half is the "only read" value whatever the
 * upper half holds, and egid plus upper bits is the egid. Any user. */
#include "poly_ipc.h"

int test_main(void) {
    i64 e = syscall0(SYS_getegid);
    i64 v = syscall1(SYS_setfsgid, (i64)0xffffffffL);
    check("u32_minus_one_reads", v == e, v);
    v = syscall1(SYS_setfsgid, (i64)-1L);
    check("i64_minus_one_reads", v == e, v);
    v = syscall1(SYS_setfsgid, (i64)0x12345678ffffffffL);
    check("upper_bits_minus_one_reads", v == e, v);
    v = syscall1(SYS_setfsgid, (i64)(0x100000000L | (u64)e));
    check("upper_bits_egid_returns_old", v == e, v);
    check("still_egid", syscall1(SYS_setfsgid, -1) == e, 0);
    return finish();
}
