/* munlock: unlocking locked, never-locked and partly locked ranges
 * succeeds; the memory keeps its contents. */
#include "a_sys.h"

int test_main(void) {
    i64 m = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *p = (char *)m;
    memset(p, 'u', 4 * A_PAGE);
    expect("never_locked_ok", syscall2(SYS_munlock, m, 4 * A_PAGE), 0);
    expect("lock_middle", syscall2(SYS_mlock, m + A_PAGE, 2 * A_PAGE), 0);
    expect("unlock_wider_range", syscall2(SYS_munlock, m, 4 * A_PAGE), 0);
    expect("unlock_twice", syscall2(SYS_munlock, m, 4 * A_PAGE), 0);
    expect("lock_all", syscall2(SYS_mlock, m, 4 * A_PAGE), 0);
    expect("unlock_part", syscall2(SYS_munlock, m + A_PAGE, A_PAGE), 0);
    expect("unlock_unaligned", syscall2(SYS_munlock, m + 2 * A_PAGE + 7, 3), 0);
    expect("unlock_length_0", syscall2(SYS_munlock, m, 0), 0);
    check("contents_kept", p[0] == 'u' && p[4 * A_PAGE - 1] == 'u', p[0]);
    expect("unlock_rest", syscall2(SYS_munlock, m, 4 * A_PAGE), 0);
    syscall2(SYS_munmap, m, 4 * A_PAGE);
    return finish();
}
