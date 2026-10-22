/* The valid swapon flags are the priority (bits 0-14), SWAP_FLAG_PREFER
 * (0x8000), SWAP_FLAG_DISCARD (0x10000), SWAP_FLAG_DISCARD_ONCE (0x20000)
 * and SWAP_FLAG_DISCARD_PAGES (0x40000). Any other bit of the 32-bit
 * flags (bits 19 to 31, alone or mixed with valid ones, and -1) is
 * EINVAL (22), even without CAP_SYS_ADMIN (every capability dropped with
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

#define P "/tmp/swapon_flags_invalid_missing"

int test_main(void) {
    char name[] = "bit_NN_einval";
    check("drop_caps", drop_caps() == 0, 0);
    for (int bit = 19; bit < 32; bit++) {
        i64 r = syscall2(SYS_swapon, P, (i64)(int)(1u << bit));
        name[4] = (char)('0' + bit / 10);
        name[5] = (char)('0' + bit % 10);
        check(name, r == -EINVAL, r);
    }
    i64 r = syscall2(SYS_swapon, P, -1);
    check("minus_one_einval", r == -EINVAL, r);
    r = syscall2(SYS_swapon, P, 0x7ffff | 0x80000);
    check("valid_plus_bit19_einval", r == -EINVAL, r);
    r = syscall2(SYS_swapon, P, 0x8000 | 0x100000);
    check("prefer_plus_bit20_einval", r == -EINVAL, r);
    r = syscall2(SYS_swapon, P, 0xfff80000);
    check("all_invalid_bits_einval", r == -EINVAL, r);
    r = syscall2(SYS_swapon, P, 0x7ffff);
    check("all_valid_bits_eperm", r == -EPERM, r);
    return finish();
}
