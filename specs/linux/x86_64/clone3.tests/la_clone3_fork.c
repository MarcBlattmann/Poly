/* clone3(args, size) with only exit_signal = SIGCHLD set (no stack)
 * makes a child like fork: it returns the child's pid in the parent and
 * 0 in the child, which runs on a copy of the parent's memory (its
 * stores are not seen by the parent). The parent gets SIGCHLD and can
 * wait for it. size may be 64 (CLONE_ARGS_SIZE_VER0), 80 (VER1), 88
 * (VER2) or larger up to a page when the extra bytes are 0.
 * exit_signal 0 makes a child that sends no signal at its end; it is
 * waited for with __WALL. (CLONE_VFORK is set in every call so the
 * parent only continues once the child has ended.)
 */
#include "a_sys.h"

#define SYS_clone3 435

/* struct clone_args (linux/sched.h), 88 bytes */
struct k_clone_args {
    u64 flags, pidfd, child_tid, parent_tid, exit_signal, stack, stack_size, tls, set_tid, set_tid_size, cgroup;
};

static volatile int marker = 1;
static char big[4096];

static i64 child_exit(i64 pid) {
    i32 status = -1;
    i64 r = syscall4(SYS_wait4, pid, &status, __WALL, 0);
    return r == pid && A_WIFEXITED(status) ? A_WEXITSTATUS(status) : -1;
}

int test_main(void) {
    a_block_sigchld();
    struct k_clone_args a;
    memset(&a, 0, sizeof a);
    a.flags = CLONE_VFORK; /* the parent waits for the child: a fixed call order */
    a.exit_signal = SIGCHLD;
    i64 me = syscall0(SYS_getpid);
    i64 pid = syscall2(SYS_clone3, &a, 64);
    if (pid == 0) {
        marker = 2;
        syscall1(SYS_exit, syscall0(SYS_getppid) == me ? 7 : 1);
    }
    check("parent_gets_pid", pid > 0 && pid != me, pid);
    expect("child_exit_code", child_exit(pid), 7);
    expect("memory_copied_not_shared", marker, 1);
    u64 pending = 0;
    syscall2(SYS_rt_sigpending, &pending, 8);
    check("sigchld_pending", (pending >> (SIGCHLD - 1)) & 1, (i64)pending);

    pid = syscall2(SYS_clone3, &a, 80);
    if (pid == 0) syscall1(SYS_exit, 8);
    expect("size_80", child_exit(pid), 8);
    pid = syscall2(SYS_clone3, &a, 88);
    if (pid == 0) syscall1(SYS_exit, 9);
    expect("size_88", child_exit(pid), 9);
    memcpy(big, &a, sizeof a);
    pid = syscall2(SYS_clone3, big, 4096);
    if (pid == 0) syscall1(SYS_exit, 10);
    expect("size_4096_zero_tail", child_exit(pid), 10);

    a.exit_signal = 0;
    pid = syscall2(SYS_clone3, &a, 88);
    if (pid == 0) syscall1(SYS_exit, 11);
    expect("no_exit_signal_wall", child_exit(pid), 11);
    return finish();
}
