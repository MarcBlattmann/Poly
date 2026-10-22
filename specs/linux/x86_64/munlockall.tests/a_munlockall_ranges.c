/* munlockall unlocks ranges locked by mlock and by mlock2(MLOCK_ONFAULT),
 * partial ones included, across several mappings. */
#include "a_sys.h"

#define SYS_mlock2 325 /* asm/unistd_64.h */
#define MLOCK_ONFAULT 1 /* asm-generic/mman-common.h */

int test_main(void) {
    i64 a = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    i64 b = syscall6(SYS_mmap, 0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", a > 0 && b > 0, a);
    expect("mlock_middle", syscall2(SYS_mlock, a + A_PAGE, 2 * A_PAGE), 0);
    expect("mlock2_onfault", syscall3(SYS_mlock2, b, 4 * A_PAGE, MLOCK_ONFAULT), 0);
    ((volatile char *)b)[0] = 1;
    expect("a_locked", syscall3(SYS_madvise, a, 4 * A_PAGE, MADV_DONTNEED), -EINVAL);
    expect("b_locked", syscall3(SYS_madvise, b, A_PAGE, MADV_DONTNEED), -EINVAL);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    expect("a_unlocked", syscall3(SYS_madvise, a, 4 * A_PAGE, MADV_DONTNEED), 0);
    expect("b_unlocked", syscall3(SYS_madvise, b, 4 * A_PAGE, MADV_DONTNEED), 0);
    return finish();
}
