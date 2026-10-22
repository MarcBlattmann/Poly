/* munlockall takes no arguments: whatever is in the argument registers is
 * ignored and the call still unlocks everything. */
#include "a_sys.h"

int test_main(void) {
    i64 m = syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", m > 0, m);
    expect("mlockall", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    expect("munlockall_junk", syscall6(SYS_munlockall, -1, 0xdeadbeef, 1, 2, 3, 4), 0);
    expect("unlocked", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), 0);
    expect("mlockall_2", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    expect("munlockall_flags_like", syscall1(SYS_munlockall, MCL_CURRENT | MCL_FUTURE), 0);
    expect("unlocked_2", syscall3(SYS_madvise, m, A_PAGE, MADV_DONTNEED), 0);
    return finish();
}
