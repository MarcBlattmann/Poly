/* mremap: MREMAP_MAYMOVE moves a mapping that cannot grow in place, with
 * its contents; MREMAP_FIXED puts it at a chosen address, replacing what
 * was there; the old range is unmapped after a move. */
#include "a_sys.h"

static i64 map(i64 addr, i64 len, i64 extra) {
    return syscall6(SYS_mmap, addr, len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | extra, -1, 0);
}

static int mapped(i64 a) {
    i64 r = syscall6(SYS_mmap, a, A_PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    if (r == a) {
        syscall2(SYS_munmap, a, A_PAGE);
        return 0;
    }
    return r == -EEXIST;
}

int test_main(void) {
    /* Reserve 8 pages, use the first 2, block page 2. */
    i64 m = map(0, 8 * A_PAGE, 0);
    check("reserve", m > 0, m);
    syscall2(SYS_munmap, m + 3 * A_PAGE, 5 * A_PAGE);
    char *p = (char *)m;
    memset(p, 'x', 2 * A_PAGE);
    p[0] = 'S';
    p[2 * A_PAGE - 1] = 'E';
    p[2 * A_PAGE] = 'B';
    i64 r = syscall5(SYS_mremap, m, 2 * A_PAGE, 4 * A_PAGE, MREMAP_MAYMOVE, 0);
    check("maymove_moved", r > 0 && r != m, r);
    if (r <= 0) return finish();
    char *q = (char *)r;
    check("moved_contents", q[0] == 'S' && q[2 * A_PAGE - 1] == 'E' && q[A_PAGE] == 'x', q[0]);
    check("moved_new_part_zero", q[2 * A_PAGE] == 0 && q[4 * A_PAGE - 1] == 0, q[2 * A_PAGE]);
    check("old_range_unmapped", !mapped(m) && !mapped(m + A_PAGE), 0);
    check("blocker_page_untouched", p[2 * A_PAGE] == 'B', p[2 * A_PAGE]);

    /* MREMAP_FIXED to the old address, over a fresh mapping there. */
    i64 there = map(m, 2 * A_PAGE, MAP_FIXED_NOREPLACE);
    expect("map_at_old_address", there, m);
    p[0] = 'O';
    r = syscall5(SYS_mremap, (i64)q, 4 * A_PAGE, 2 * A_PAGE, MREMAP_MAYMOVE | MREMAP_FIXED, m);
    expect("fixed_lands_at_target", r, m);
    check("fixed_replaced_contents", p[0] == 'S' && p[A_PAGE] == 'x', p[0]);
    check("fixed_source_unmapped", !mapped((i64)q) && !mapped((i64)q + 3 * A_PAGE), 0);
    syscall2(SYS_munmap, m, 3 * A_PAGE);
    return finish();
}
