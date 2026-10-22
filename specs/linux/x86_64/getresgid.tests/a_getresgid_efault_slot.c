/* getresgid: a bad pointer in any of the three positions is EFAULT. The
 * ids are stored in order real, effective, saved and the call stops at
 * the first bad one: slots before it hold their id, slots after it are
 * not written. */
#include "a_sys.h"

#define SYS_getresgid 120 /* asm/unistd_64.h */
#define MARK 0xdeadbeefU

int test_main(void) {
    u32 r0, e0, s0;
    syscall3(SYS_getresgid, &r0, &e0, &s0);
    u32 r = MARK, e = MARK, s = MARK;
    expect("null_real_efault", syscall3(SYS_getresgid, 0, &e, &s), -EFAULT);
    check("null_real_rest_unwritten", e == MARK && s == MARK, (i64)e);
    r = e = s = MARK;
    expect("null_effective_efault", syscall3(SYS_getresgid, &r, 0, &s), -EFAULT);
    expect("null_effective_real_written", r, r0);
    expect("null_effective_saved_unwritten", s, MARK);
    r = e = s = MARK;
    expect("null_saved_efault", syscall3(SYS_getresgid, &r, &e, 0), -EFAULT);
    check("null_saved_first_two_written", r == r0 && e == e0, (i64)r);
    expect("all_null_efault", syscall3(SYS_getresgid, 0, 0, 0), -EFAULT);
    /* Low addresses other than 0 are not mapped either. */
    r = e = s = MARK;
    expect("addr_8_effective_efault", syscall3(SYS_getresgid, &r, 8, &s), -EFAULT);
    check("addr_8_real_written_saved_not", r == r0 && s == MARK, (i64)s);
    return finish();
}
