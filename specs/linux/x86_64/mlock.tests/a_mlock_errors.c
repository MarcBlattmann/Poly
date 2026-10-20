/* mlock: unmapped ranges and PROT_NONE pages are ENOMEM; a length that
 * wraps around the address space is EINVAL. */
#include "a_sys.h"

int test_main(void) {
    i64 m = syscall6(SYS_mmap, 0, 3 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, m + A_PAGE, A_PAGE);
    expect("hole_enomem", syscall2(SYS_mlock, m, 3 * A_PAGE), -ENOMEM);
    expect("unmapped_enomem", syscall2(SYS_mlock, m + A_PAGE, A_PAGE), -ENOMEM);
    expect("mapped_part_ok", syscall2(SYS_mlock, m + 2 * A_PAGE, A_PAGE), 0);
    syscall3(SYS_mprotect, m, A_PAGE, PROT_NONE);
    expect("prot_none_enomem", syscall2(SYS_mlock, m, A_PAGE), -ENOMEM);
    expect("kernel_address_enomem", syscall2(SYS_mlock, 0xffff800000000000L, A_PAGE), -ENOMEM);
    expect("wraparound_einval", syscall2(SYS_mlock, m, -A_PAGE), -EINVAL);
    syscall2(SYS_munmap, m, 3 * A_PAGE);
    return finish();
}
