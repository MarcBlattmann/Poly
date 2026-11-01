/* get_robust_list of another thread: a thread of the same process (by
 * tid) and a child process of the same user can be read (their own
 * heads, not the caller's). Reading another process needs ptrace read
 * access: once the child has made itself non-dumpable
 * (prctl(PR_SET_DUMPABLE, 0)), a caller without CAP_SYS_PTRACE gets
 * EPERM; root still reads it.
 * poly-diff: syscalls=off (three tasks; a child's exit and the parent's wait interleave by scheduling)
 */
#include "a_sys.h"

#define SYS_prctl 157
#define SYS_set_robust_list 273
#define SYS_get_robust_list 274
/* linux/prctl.h */
#define PR_SET_DUMPABLE 4

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM | CLONE_CHILD_CLEARTID)

static volatile i32 thread_done, child_done, *stage;
static volatile i32 go;

static int thread_fn(void *arg) {
    (void)arg;
    syscall2(SYS_set_robust_list, 0x7770, 24);
    stage[0] = 1;
    while (!go) a_relax();
    return 0;
}

static int child_fn(void *arg) {
    (void)arg;
    syscall2(SYS_set_robust_list, 0x8880, 24);
    stage[1] = 1;
    while (stage[2] == 0) a_relax();
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 0);
    stage[3] = 1;
    while (stage[4] == 0) a_relax();
    return 0;
}

static u64 head, len;
static i64 get(i64 pid) {
    head = 0x5555;
    len = 0x5555;
    return syscall3(SYS_get_robust_list, pid, &head, &len);
}

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    a_block_sigchld();
    stage = a_shared();
    syscall2(SYS_set_robust_list, 0x6660, 24);

    thread_done = -1;
    long tid = a_clone(THREAD_FLAGS, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&thread_done, 0, thread_fn, 0);
    a_spin_until(stage, 1, A_SPIN_LIMIT);
    expect("thread_read", get(tid), 0);
    expect("thread_head", (i64)head, 0x7770);
    expect("thread_len", (i64)len, 24);
    go = 1;
    a_spin_until(&thread_done, 0, A_SPIN_LIMIT);
    get(0);
    expect("own_head_unchanged", (i64)head, 0x6660);

    /* A child process with its own memory. */
    long pid = a_clone(SIGCHLD, a_stacks[1] + A_STACK_SIZE, 0, 0, 0, child_fn, 0);
    a_spin_until(stage + 1, 1, A_SPIN_LIMIT);
    expect("child_read", get(pid), 0);
    expect("child_head", (i64)head, 0x8880);
    stage[2] = 1;
    a_spin_until(stage + 3, 1, A_SPIN_LIMIT);
    i64 r = get(pid);
    expect("nondumpable_child", r, root ? 0 : -EPERM);
    expect("nondumpable_head", (i64)head, root ? 0x8880 : 0x5555);
    stage[4] = 1;
    syscall4(SYS_wait4, pid, 0, 0, 0);
    return finish();
}
