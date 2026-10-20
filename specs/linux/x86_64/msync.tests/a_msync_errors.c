/* msync: invalid flags and alignment, unmapped ranges; anonymous and
 * private mappings are accepted. */
#include "a_sys.h"

int test_main(void) {
    i64 m = syscall6(SYS_mmap, 0, 3 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("anon_private_ok", syscall3(SYS_msync, m, 3 * A_PAGE, MS_SYNC), 0);
    expect("unaligned_einval", syscall3(SYS_msync, m + 4, A_PAGE, MS_SYNC), -EINVAL);
    expect("sync_and_async_einval", syscall3(SYS_msync, m, A_PAGE, MS_SYNC | MS_ASYNC), -EINVAL);
    expect("unknown_flag_einval", syscall3(SYS_msync, m, A_PAGE, 8), -EINVAL);
    expect("length_0_ok", syscall3(SYS_msync, m, 0, MS_SYNC), 0);
    expect("length_unaligned_ok", syscall3(SYS_msync, m, 10, MS_ASYNC), 0);
    syscall2(SYS_munmap, m + A_PAGE, A_PAGE);
    expect("hole_enomem", syscall3(SYS_msync, m, 3 * A_PAGE, MS_SYNC), -ENOMEM);
    expect("unmapped_enomem", syscall3(SYS_msync, m + A_PAGE, A_PAGE, MS_ASYNC), -ENOMEM);
    expect("mapped_part_ok", syscall3(SYS_msync, m + 2 * A_PAGE, A_PAGE, MS_SYNC), 0);
    syscall2(SYS_munmap, m, 3 * A_PAGE);
    i64 s = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    expect("anon_shared_read_only_ok", syscall3(SYS_msync, s, A_PAGE, MS_SYNC | MS_INVALIDATE), 0);
    syscall2(SYS_munmap, s, A_PAGE);
    return finish();
}
