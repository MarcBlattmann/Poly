/* munlockall: a mapping that was locked by mlock is plain afterwards, so
 * growing it with mremap does not populate the new pages. */
#include "a_sys.h"

static i64 resident(i64 addr, i64 pages) {
    unsigned char v[16];
    i64 r = syscall3(SYS_mincore, addr, pages * A_PAGE, v);
    if (r) return r;
    i64 n = 0;
    for (i64 i = 0; i < pages; i++) n += v[i] & 1;
    return n;
}

int test_main(void) {
    i64 m = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", m > 0, m);
    expect("mlock", syscall2(SYS_mlock, m, 2 * A_PAGE), 0);
    i64 g = syscall5(SYS_mremap, m, 2 * A_PAGE, 4 * A_PAGE, MREMAP_MAYMOVE, 0);
    check("grow_locked", g > 0, g);
    expect("locked_growth_populated", resident(g, 4), 4);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    i64 h = syscall5(SYS_mremap, g, 4 * A_PAGE, 8 * A_PAGE, MREMAP_MAYMOVE, 0);
    check("grow_unlocked", h > 0, h);
    expect("old_pages_resident", resident(h, 4), 4);
    expect("new_pages_not_populated", resident(h + 4 * A_PAGE, 4), 0);
    return finish();
}
