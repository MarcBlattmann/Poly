/* munlockall: turns MCL_FUTURE off, so mappings made afterwards are no
 * longer populated (also with MCL_ONFAULT, and brk growth). */
#include "a_sys.h"

static i64 resident(i64 addr, i64 pages) {
    unsigned char v[16];
    i64 r = syscall3(SYS_mincore, addr, pages * A_PAGE, v);
    if (r) return r;
    i64 n = 0;
    for (i64 i = 0; i < pages; i++) n += v[i] & 1;
    return n;
}

static i64 anon(i64 pages) {
    return syscall6(SYS_mmap, 0, pages * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int test_main(void) {
    expect("future", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    expect("future_populates", resident(anon(4), 4), 4);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    i64 a = anon(4);
    expect("not_populated_after", resident(a, 4), 0);
    ((volatile char *)a)[A_PAGE] = 1;
    expect("touched_page_only", resident(a, 4), 1);
    i64 b0 = syscall1(SYS_brk, 0);
    i64 b1 = syscall1(SYS_brk, b0 + 4 * A_PAGE);
    expect("brk_grows", b1 - b0, 4 * A_PAGE);
    expect("brk_not_populated", resident((b0 + A_PAGE - 1) & ~(A_PAGE - 1), 3), 0);
    expect("future_onfault", syscall1(SYS_mlockall, MCL_FUTURE | MCL_ONFAULT), 0);
    expect("munlockall_again", syscall0(SYS_munlockall), 0);
    i64 c = anon(2);
    expect("dontneed_after_onfault_cleared", syscall3(SYS_madvise, c, A_PAGE, MADV_DONTNEED), 0);
    expect("future_again", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    expect("future_works_again", resident(anon(4), 4), 4);
    return finish();
}
