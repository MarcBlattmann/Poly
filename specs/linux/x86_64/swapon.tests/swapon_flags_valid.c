/* Every valid flag value passes the flag check and so reaches the
 * privilege check: without CAP_SYS_ADMIN (every capability dropped with
 * capset first) each single valid bit (0 to 18), the priority alone
 * without SWAP_FLAG_PREFER, SWAP_FLAG_PREFER with priorities 0 and 32767,
 * the three discard flags together and all valid bits at once are EPERM
 * (1). */
#include "poly_ipc.h"

#define SYS_swapon 167
#define SWAP_FLAG_PREFER 0x8000         /* sys/swap.h */
#define SWAP_FLAG_PRIO_MASK 0x7fff      /* sys/swap.h */
#define SWAP_FLAG_DISCARD 0x10000       /* linux/swap.h */
#define SWAP_FLAG_DISCARD_ONCE 0x20000  /* linux/swap.h */
#define SWAP_FLAG_DISCARD_PAGES 0x40000 /* linux/swap.h */

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

#define P "/tmp/swapon_flags_valid_missing"

int test_main(void) {
    char name[] = "bit_NN_eperm";
    check("drop_caps", drop_caps() == 0, 0);
    for (int bit = 0; bit < 19; bit++) {
        i64 r = syscall2(SYS_swapon, P, 1L << bit);
        name[4] = (char)('0' + bit / 10);
        name[5] = (char)('0' + bit % 10);
        check(name, r == -EPERM, r);
    }
    i64 r = syscall2(SYS_swapon, P, 100);
    check("priority_without_prefer_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, P, SWAP_FLAG_PREFER | 0);
    check("prefer_prio0_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, P, SWAP_FLAG_PREFER | SWAP_FLAG_PRIO_MASK);
    check("prefer_prio_max_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, P, SWAP_FLAG_DISCARD | SWAP_FLAG_DISCARD_ONCE | SWAP_FLAG_DISCARD_PAGES);
    check("all_discard_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, P, SWAP_FLAG_DISCARD_ONCE);
    check("discard_once_alone_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, P, 0x7ffff);
    check("all_valid_eperm", r == -EPERM, r);
    return finish();
}
