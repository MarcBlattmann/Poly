/* mremap: shrinking stays in place and unmaps the tail; growing in place
 * works when the pages after it are free and fails with ENOMEM without
 * MREMAP_MAYMOVE when they are not. */
#include "a_sys.h"

static i64 map(i64 addr, i64 len, i64 extra) {
    return syscall6(SYS_mmap, addr, len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | extra, -1, 0);
}

/* 1 if the page at `a` is mapped, 0 if not (probed with MAP_FIXED_NOREPLACE). */
static int mapped(i64 a) {
    i64 r = syscall6(SYS_mmap, a, A_PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    if (r == a) {
        syscall2(SYS_munmap, a, A_PAGE);
        return 0;
    }
    return r == -EEXIST;
}

int test_main(void) {
    /* Reserve 6 pages and give back the last 2, so the 2 pages after the
     * 4-page mapping are known to be free. */
    i64 m = map(0, 6 * A_PAGE, 0);
    check("map_4_pages", m > 0 && syscall2(SYS_munmap, m + 4 * A_PAGE, 2 * A_PAGE) == 0, m);
    char *p = (char *)m;
    for (int i = 0; i < 4; i++) p[i * A_PAGE] = (char)('a' + i);
    i64 r = syscall5(SYS_mremap, m, 4 * A_PAGE, 2 * A_PAGE, 0, 0);
    expect("shrink_in_place", r, m);
    check("shrunk_data_kept", p[0] == 'a' && p[A_PAGE] == 'b', p[0]);
    check("tail_unmapped", !mapped(m + 2 * A_PAGE) && !mapped(m + 3 * A_PAGE), 0);
    r = syscall5(SYS_mremap, m, 2 * A_PAGE, 4 * A_PAGE, 0, 0);
    expect("grow_in_place_into_free_pages", r, m);
    check("grown_data_kept", p[0] == 'a' && p[A_PAGE] == 'b', p[0]);
    check("grown_pages_zero", p[2 * A_PAGE] == 0 && p[4 * A_PAGE - 1] == 0, p[2 * A_PAGE]);
    p[3 * A_PAGE] = 'z';
    /* Block the page after the mapping. */
    i64 blocker = map(m + 4 * A_PAGE, A_PAGE, MAP_FIXED_NOREPLACE);
    expect("blocker_placed", blocker, m + 4 * A_PAGE);
    expect("grow_blocked_enomem", syscall5(SYS_mremap, m, 4 * A_PAGE, 5 * A_PAGE, 0, 0), -ENOMEM);
    check("failed_grow_kept_data", p[3 * A_PAGE] == 'z', p[3 * A_PAGE]);
    expect("same_size_noop", syscall5(SYS_mremap, m, 4 * A_PAGE, 4 * A_PAGE, 0, 0), m);
    /* Sizes are rounded up to whole pages. */
    expect("shrink_unaligned_size", syscall5(SYS_mremap, m, 4 * A_PAGE, A_PAGE + 1, 0, 0), m);
    check("rounded_keeps_two_pages", mapped(m + A_PAGE) && !mapped(m + 2 * A_PAGE), 0);
    syscall2(SYS_munmap, m, 2 * A_PAGE);
    syscall2(SYS_munmap, blocker, A_PAGE);
    return finish();
}
