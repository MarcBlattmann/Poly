/* setregid calls any user may make: setregid(-1, -1), setregid(rgid,
 * egid) with the caller's own ids, setregid(egid, -1), setregid(-1,
 * rgid), and the same with high register bits set (the arguments are
 * 32-bit gids, so (1<<32)|gid is gid and 0x1ffffffff is -1). All return 0
 * and, for a caller whose real, effective and saved gids are equal, change
 * nothing; the filesystem gid stays the effective gid.
 */
#include "a_sys.h"

#define SYS_setregid 114
#define SYS_getresgid 120
#define SYS_setfsgid 123

static u32 r0, e0, s0;

static int unchanged(void) {
    u32 r = 1, e = 1, s = 1;
    syscall3(SYS_getresgid, &r, &e, &s);
    return r == r0 && e == e0 && s == s0 && syscall1(SYS_setfsgid, -1) == e0;
}

int test_main(void) {
    syscall3(SYS_getresgid, &r0, &e0, &s0);
    check("program_ids_equal", r0 == e0 && e0 == s0, 0);
    i64 r = syscall2(SYS_setregid, -1, -1);
    check("neither_ok", r == 0, r);
    check("neither_unchanged", unchanged(), 0);
    r = syscall2(SYS_setregid, r0, e0);
    check("own_ids_ok", r == 0, r);
    check("own_ids_unchanged", unchanged(), 0);
    r = syscall2(SYS_setregid, e0, -1);
    check("rgid_to_egid_ok", r == 0, r);
    r = syscall2(SYS_setregid, -1, r0);
    check("egid_to_rgid_ok", r == 0, r);
    r = syscall2(SYS_setregid, -1, s0);
    check("egid_to_sgid_ok", r == 0, r);
    r = syscall2(SYS_setregid, (1L << 32) | r0, (3L << 32) | e0);
    check("high_bits_ignored", r == 0, r);
    r = syscall2(SYS_setregid, 0x1ffffffffL, 0xffffffffffffffffL);
    check("high_bits_minus1", r == 0, r);
    check("still_unchanged", unchanged(), 0);
    return finish();
}
