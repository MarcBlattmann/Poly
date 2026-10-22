/* munlockall: pages locked by mlockall(MCL_CURRENT) refuse MADV_DONTNEED
 * (EINVAL); after munlockall the advice succeeds and a private anonymous
 * page reads zero again. */
#include "a_sys.h"

int test_main(void) {
    i64 m = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", m > 0, m);
    volatile char *p = (volatile char *)m;
    p[0] = 'x';
    expect("mlockall", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    expect("dontneed_locked_einval", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), -EINVAL);
    check("kept_while_locked", p[0] == 'x', p[0]);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    expect("dontneed_unlocked_ok", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), 0);
    check("zero_after_dontneed", p[0] == 0, p[0]);
    syscall2(SYS_munmap, m, 2 * A_PAGE);
    return finish();
}
