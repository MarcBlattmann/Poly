/* getresgid: children (fork, vfork) and threads report the same three
 * group ids as their parent. */
#include "a_sys.h"

#define SYS_getresgid 120 /* asm/unistd_64.h */
#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid;
static u32 in_thread[3] = {1, 1, 1};
static volatile i64 thread_rc = -1;

static int thread(void *arg) {
    (void)arg;
    thread_rc = syscall3(SYS_getresgid, &in_thread[0], &in_thread[1], &in_thread[2]);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    u32 m[3];
    syscall3(SYS_getresgid, &m[0], &m[1], &m[2]);
    volatile i32 *sh = a_shared();
    sh[0] = sh[1] = sh[2] = sh[3] = -1;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        u32 c[3];
        sh[3] = (i32)syscall3(SYS_getresgid, &c[0], &c[1], &c[2]);
        sh[0] = (i32)c[0];
        sh[1] = (i32)c[1];
        sh[2] = (i32)c[2];
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("fork_child_ok", sh[3], 0);
    check("fork_child_same", (u32)sh[0] == m[0] && (u32)sh[1] == m[1] && (u32)sh[2] == m[2], sh[0]);
    static u32 v[3] = {1, 1, 1};
    static volatile i64 vrc = -1;
    pid = syscall0(SYS_vfork);
    if (pid == 0) {
        vrc = syscall3(SYS_getresgid, &v[0], &v[1], &v[2]);
        syscall1(SYS_exit_group, 0);
    }
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("vfork_child_ok", vrc, 0);
    check("vfork_child_same", v[0] == m[0] && v[1] == m[1] && v[2] == m[2], (i64)v[0]);
    ctid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&ctid, 0, thread, 0);
    a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    expect("thread_ok", thread_rc, 0);
    check("thread_same", in_thread[0] == m[0] && in_thread[1] == m[1] && in_thread[2] == m[2], (i64)in_thread[0]);
    return finish();
}
