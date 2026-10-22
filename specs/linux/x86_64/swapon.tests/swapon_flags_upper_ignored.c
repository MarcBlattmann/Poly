/* The flags argument is a 32-bit int: bits 32-63 of the register are
 * ignored. Only the low 32 bits decide between EINVAL (22, an unknown
 * bit) and EPERM (1, no CAP_SYS_ADMIN; every capability is dropped with
 * capset first). */
#include "poly_ipc.h"

#define SYS_swapon 167

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

/* Empty all three capability sets: any user may do that, and root then
 * fails the privilege check like everybody else. */
static i64 drop_caps(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    return syscall2(SYS_capset, &h, d);
}

#define P "/tmp/swapon_flags_upper_missing"

int test_main(void) {
    check("drop_caps", drop_caps() == 0, 0);
    i64 r = syscall2(SYS_swapon, P, 0x100000000L);
    check("bit32_ignored_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, P, (i64)0xffffffff00000000UL);
    check("high_half_ignored_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, P, (i64)0x8000000000008005UL);
    check("bit63_with_prefer_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, P, (i64)0xffffffff00080000UL);
    check("low_bit19_einval", r == -EINVAL, r);
    r = syscall2(SYS_swapon, P, 0x1ffffffffL);
    check("low_all_ones_einval", r == -EINVAL, r);
    r = syscall2(SYS_swapon, P, 0x0000000180000000L);
    check("low_bit31_einval", r == -EINVAL, r);
    return finish();
}
