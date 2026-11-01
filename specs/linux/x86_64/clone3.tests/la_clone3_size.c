/* clone3 size checks: size below 64 (0, 8, 63) is EINVAL; above a page
 * (4097) is E2BIG; between 88 and a page, any non-zero byte past the 88
 * known bytes is E2BIG; these are checked before the structure is read
 * except for the tail, which is read: an unreadable args pointer with a
 * valid size is EFAULT, with size 0 still EINVAL. No child is made by a
 * failed call.
 */
#include "a_sys.h"

#define SYS_clone3 435

struct k_clone_args {
    u64 flags, pidfd, child_tid, parent_tid, exit_signal, stack, stack_size, tls, set_tid, set_tid_size, cgroup;
};

static char buf[8192] __attribute__((aligned(8)));

static i64 c3(void *a, u64 size) {
    i64 r = syscall2(SYS_clone3, a, size);
    if (r == 0) syscall1(SYS_exit, 0); /* never expected */
    return r;
}

int test_main(void) {
    a_block_sigchld();
    struct k_clone_args *a = (struct k_clone_args *)buf;
    a->exit_signal = SIGCHLD;

    expect("size_0_einval", c3(a, 0), -EINVAL);
    expect("size_8_einval", c3(a, 8), -EINVAL);
    expect("size_63_einval", c3(a, 63), -EINVAL);
    expect("size_4097_e2big", c3(a, 4097), -E2BIG);
    expect("size_8192_e2big", c3(a, 8192), -E2BIG);
    buf[100] = 1;
    expect("nonzero_tail_e2big", c3(a, 200), -E2BIG);
    expect("nonzero_tail_far_e2big", c3(a, 4096), -E2BIG);
    buf[100] = 0;
    buf[88] = 1;
    expect("nonzero_byte_88_e2big", c3(a, 89), -E2BIG);
    buf[88] = 0;

    expect("null_efault", c3(0, 64), -EFAULT);
    expect("null_size_0_einval", c3(0, 0), -EINVAL);
    expect("null_size_big_e2big", c3(0, 4097), -E2BIG);
    expect("low_address_efault", c3((void *)8, 88), -EFAULT);
    char *page = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, page + A_PAGE, A_PAGE);
    expect("runs_into_unmapped_efault", c3(page + A_PAGE - 40, 88), -EFAULT);

    i64 r = syscall4(SYS_wait4, -1, 0, __WALL | WNOHANG, 0);
    expect("no_child_made", r, -ECHILD);
    return finish();
}
