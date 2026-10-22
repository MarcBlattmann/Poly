/* mlockall: a mapping locked by MCL_CURRENT stays locked when mremap grows
 * or moves it, so the new pages are populated; munmap of locked memory
 * works. */
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
    i64 a = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    ((char *)a)[0] = 'm';
    expect("mlockall", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    expect("locked", resident(a, 2), 2);
    i64 m = syscall5(SYS_mremap, a, 2 * A_PAGE, 6 * A_PAGE, MREMAP_MAYMOVE, 0);
    check("mremap", m > 0, m);
    expect("grown_part_resident", resident(m, 6), 6);
    check("contents_moved", ((char *)m)[0] == 'm', ((char *)m)[0]);
    i64 later = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    i64 g = syscall5(SYS_mremap, later, 2 * A_PAGE, 4 * A_PAGE, MREMAP_MAYMOVE, 0);
    expect("unlocked_mapping_grown_not_resident", resident(g, 4), 0);
    expect("munmap_locked", syscall2(SYS_munmap, m, 6 * A_PAGE), 0);
    return finish();
}
