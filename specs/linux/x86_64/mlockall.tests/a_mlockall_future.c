/* mlockall(MCL_FUTURE): mappings made afterwards (mmap, brk) are
 * populated at once; PROT_NONE ones only once mprotect makes them
 * accessible. Existing mappings are left as they are. */
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
    i64 old = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("mlockall", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    expect("existing_untouched", resident(old, 4), 0);
    i64 a = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", a > 0, a);
    expect("new_mapping_resident", resident(a, 4), 4);
    check("new_mapping_zero", ((volatile char *)a)[2 * A_PAGE] == 0, 0);
    i64 n = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("prot_none_not_resident", resident(n, 4), 0);
    expect("mprotect", syscall3(SYS_mprotect, n, 4 * A_PAGE, PROT_READ | PROT_WRITE), 0);
    expect("after_mprotect_resident", resident(n, 4), 4);
    i64 brk0 = syscall1(SYS_brk, 0);
    i64 brk1 = syscall1(SYS_brk, brk0 + 8 * A_PAGE);
    expect("brk_grew", brk1 - brk0, 8 * A_PAGE);
    i64 heap = (brk0 + A_PAGE - 1) & ~(i64)(A_PAGE - 1);
    expect("brk_area_resident", resident(heap, 4), 4);
    return finish();
}
