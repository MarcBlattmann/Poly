/* mlock2(addr, len, flags) locks the pages of [addr, addr+len) (addr
 * rounded down, the end rounded up to pages) and returns 0. With flags 0
 * every page is made present at once (mincore shows fresh anonymous pages
 * resident afterwards, also on a PROT_READ mapping); with MLOCK_ONFAULT
 * (1) nothing is populated: pages become present (and locked) only when
 * touched. Locking a locked range again works; with flags 0 it populates
 * pages an earlier MLOCK_ONFAULT left out. len 0 succeeds even for an
 * unmapped address. flags is a 32-bit int: bits 32..63 are ignored.
 * Uses well under 64 KiB of locked memory.
 */
#include "a_sys.h"

#define SYS_mlock2 325
/* asm-generic/mman-common.h */
#define MLOCK_ONFAULT 0x01

static unsigned char vec[16];
static i64 resident(char *p, int pages) {
    i64 r = syscall3(SYS_mincore, p, (u64)pages * A_PAGE, vec);
    if (r < 0) return r;
    i64 n = 0;
    for (int i = 0; i < pages; i++) n += vec[i] & 1;
    return n;
}
static char *map(int pages, int prot) {
    return (char *)syscall6(SYS_mmap, 0, (u64)pages * A_PAGE, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int test_main(void) {
    char *a = map(4, PROT_READ | PROT_WRITE);
    expect("fresh_not_resident", resident(a, 4), 0);
    expect("lock_2_pages", syscall3(SYS_mlock2, a, 2 * A_PAGE, 0), 0);
    expect("locked_pages_present", resident(a, 4), 2);
    expect("lock_onfault", syscall3(SYS_mlock2, a + 2 * A_PAGE, 2 * A_PAGE, MLOCK_ONFAULT), 0);
    expect("onfault_populates_nothing", resident(a, 4), 2);
    a[3 * A_PAGE] = 1;
    expect("touched_page_present", resident(a, 4), 3);
    expect("relock_flags0", syscall3(SYS_mlock2, a + 2 * A_PAGE, 2 * A_PAGE, 0), 0);
    expect("relock_populates_rest", resident(a, 4), 4);
    expect("relock_same_range", syscall3(SYS_mlock2, a, 4 * A_PAGE, MLOCK_ONFAULT), 0);
    expect("munlock_a", syscall2(SYS_munlock, a, 4 * A_PAGE), 0);

    char *r = map(3, PROT_READ);
    expect("lock_readonly", syscall3(SYS_mlock2, r, 3 * A_PAGE, 0), 0);
    expect("readonly_pages_present", resident(r, 3), 3);
    expect("munlock_r", syscall2(SYS_munlock, r, 3 * A_PAGE), 0);

    /* Rounding: a few bytes inside the second page lock (and populate)
     * that page only; a range crossing a page boundary takes both. */
    char *u = map(4, PROT_READ | PROT_WRITE);
    expect("lock_unaligned_inside", syscall3(SYS_mlock2, u + A_PAGE + 100, 10, 0), 0);
    expect("only_that_page", resident(u, 4), 1);
    check("it_is_page_1", vec[1] & 1, vec[1]);
    expect("lock_unaligned_cross", syscall3(SYS_mlock2, u + 3 * A_PAGE - 1, 2, 0), 0);
    expect("crossing_takes_two", resident(u, 4), 3);
    expect("munlock_u", syscall2(SYS_munlock, u, 4 * A_PAGE), 0);

    char *z = map(2, PROT_READ | PROT_WRITE);
    expect("len0", syscall3(SYS_mlock2, z, 0, 0), 0);
    expect("len0_populates_nothing", resident(z, 2), 0);
    expect("len0_unmapped", syscall3(SYS_mlock2, 0x10000, 0, 0), 0);
    expect("len0_kernel_address", syscall3(SYS_mlock2, 0xffff800000000000UL, 0, 0), 0);
    expect("flags_high_bits_ignored", syscall3(SYS_mlock2, z, A_PAGE, (1UL << 32) | MLOCK_ONFAULT), 0);
    expect("high_bits_onfault_kept", resident(z, 2), 0);
    expect("munlock_z", syscall2(SYS_munlock, z, 2 * A_PAGE), 0);
    return finish();
}
