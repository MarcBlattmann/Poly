/* The group ids are inherited: a child made by fork and a thread made by
 * clone(CLONE_THREAD) read the same real, effective and saved gids as the
 * parent with getresgid.
 * poly-diff: syscalls=off (a thread runs beside the main thread)
 */
#include "a_sys.h"

#define SYS_getresgid 120
#define SYS_futex 202

static u32 ids[3];
static volatile u32 thread_ids[3];
static volatile i64 thread_ret = -1;
static volatile i32 thread_done;

static int thread_fn(void *arg) {
    (void)arg;
    u32 a = 0, b = 0, c = 0;
    thread_ret = syscall3(SYS_getresgid, &a, &b, &c);
    thread_ids[0] = a;
    thread_ids[1] = b;
    thread_ids[2] = c;
    return 0;
}

int test_main(void) {
    check("parent_ok", syscall3(SYS_getresgid, &ids[0], &ids[1], &ids[2]) == 0, 0);

    i64 c = a_fork_sync();
    if (c == 0) {
        u32 a = 0, b = 0, s = 0;
        i64 r = syscall3(SYS_getresgid, &a, &b, &s);
        exit_group(r == 0 && a == ids[0] && b == ids[1] && s == ids[2] ? 0 : 1);
    }
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    check("fork_child_same", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);

    thread_done = -1;
    i64 t = a_clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM |
                        CLONE_CHILD_CLEARTID,
                    a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&thread_done, 0, thread_fn, 0);
    check("thread_started", t > 0, t);
    while (thread_done != 0) syscall6(SYS_futex, (i64)&thread_done, 0, thread_done, 0, 0, 0);
    check("thread_ok", thread_ret == 0, thread_ret);
    check("thread_same", thread_ids[0] == ids[0] && thread_ids[1] == ids[1] && thread_ids[2] == ids[2], 0);
    return finish();
}
