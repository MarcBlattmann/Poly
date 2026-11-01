/* pkey_alloc argument checks: flags must be 0 and access_rights may only
 * hold PKEY_DISABLE_ACCESS (1) and PKEY_DISABLE_WRITE (2); both are full
 * 64-bit values (bit 32 set is not 0). Anything else is EINVAL (22), and
 * EINVAL is reported before ENOSPC (28) on a CPU without protection
 * keys. On a CPU with OSPKE the test prints a skip line and passes.
 */
#include "a_sys.h"

#define SYS_pkey_alloc 330
#define ENOSPC 28

static int has_ospke(void) {
    u32 a, b, c, d;
    __asm__ volatile("cpuid" : "=a"(a), "=b"(b), "=c"(c), "=d"(d) : "a"(0), "c"(0));
    if (a < 7) return 0;
    __asm__ volatile("cpuid" : "=a"(a), "=b"(b), "=c"(c), "=d"(d) : "a"(7), "c"(0));
    return (c >> 4) & 1;
}

static i64 alloc(u64 flags, u64 rights) { return syscall2(SYS_pkey_alloc, flags, rights); }

int test_main(void) {
    if (has_ospke()) {
        put("skip: this CPU has protection keys (OSPKE); the test covers CPUs without them\n");
        return 0;
    }
    expect("flags_1", alloc(1, 0), -EINVAL);
    expect("flags_2", alloc(2, 0), -EINVAL);
    expect("flags_minus1", alloc(~0UL, 0), -EINVAL);
    expect("flags_bit32", alloc(1UL << 32, 0), -EINVAL);
    expect("rights_4", alloc(0, 4), -EINVAL);
    expect("rights_7", alloc(0, 7), -EINVAL);
    expect("rights_0x100", alloc(0, 0x100), -EINVAL);
    expect("rights_bit32", alloc(0, 1UL << 32), -EINVAL);
    expect("rights_minus1", alloc(0, ~0UL), -EINVAL);
    expect("flags_and_rights_bad", alloc(1, 4), -EINVAL);
    expect("bad_flags_good_rights", alloc(1, 1), -EINVAL);
    expect("valid_is_enospc", alloc(0, 3), -ENOSPC);
    return finish();
}
