/* clone: invalid flag combinations are EINVAL and create nothing. */
#include "a_sys.h"

static int never(void *arg) {
    (void)arg;
    return 99;
}

static long try_clone(u64 flags) {
    return a_clone(flags, a_stacks[0] + A_STACK_SIZE, 0, 0, 0, never, 0);
}

int test_main(void) {
    a_block_sigchld();
    expect("thread_without_sighand_einval", try_clone(CLONE_VM | CLONE_THREAD), -EINVAL);
    expect("sighand_without_vm_einval", try_clone(CLONE_SIGHAND | SIGCHLD), -EINVAL);
    expect("thread_without_vm_einval", try_clone(CLONE_SIGHAND | CLONE_THREAD), -EINVAL);
    /* CLONE_NEWNS (0x20000) with CLONE_FS is refused. */
    expect("newns_with_fs_einval", try_clone(0x20000 | CLONE_FS | SIGCHLD), -EINVAL);
    /* CLONE_PIDFD (0x1000) with CLONE_PARENT_SETTID: both use parent_tid. */
    expect("pidfd_with_parent_settid_einval", try_clone(0x1000 | CLONE_PARENT_SETTID | SIGCHLD), -EINVAL);
    i32 st;
    expect("nothing_was_created", syscall4(SYS_wait4, -1, &st, WNOHANG | __WALL, 0), -ECHILD);
    return finish();
}
