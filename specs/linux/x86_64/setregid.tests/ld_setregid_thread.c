/* The setregid system call changes the credentials of the calling thread
 * only (C libraries make every thread call it; the kernel does not): after
 * a second thread calls setregid(11, 22), that thread reads 11/22/22 with
 * getresgid and the main thread still reads its old ids. A child forked
 * by the main thread then has the main thread's ids.
 * poly-diff: needs=root (sets group ids of other groups)
 * poly-diff: syscalls=off (a thread runs beside the main thread)
 */
#include "a_sys.h"

#define SYS_setregid 114
#define SYS_setresgid 119
#define SYS_getresgid 120
#define SYS_futex 202

static volatile i64 t_ret;
static volatile u32 t_ids[3];
static volatile i32 thread_done;

static int thread_fn(void *arg) {
    (void)arg;
    t_ret = syscall2(SYS_setregid, 11, 22);
    u32 r = 0, e = 0, s = 0;
    syscall3(SYS_getresgid, &r, &e, &s);
    t_ids[0] = r;
    t_ids[1] = e;
    t_ids[2] = s;
    return 0;
}

int test_main(void) {
    i64 r = syscall3(SYS_setresgid, 100, 200, 300);
    check("main_setresgid", r == 0, r);
    thread_done = -1;
    i64 t = a_clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM |
                        CLONE_CHILD_CLEARTID,
                    a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&thread_done, 0, thread_fn, 0);
    check("thread_started", t > 0, t);
    while (thread_done != 0) syscall6(SYS_futex, (i64)&thread_done, 0, thread_done, 0, 0, 0);
    check("thread_setregid_ok", t_ret == 0, t_ret);
    check("thread_sees_new_ids", t_ids[0] == 11 && t_ids[1] == 22 && t_ids[2] == 22, (i64)t_ids[0]);
    u32 a = 0, b = 0, c = 0;
    syscall3(SYS_getresgid, &a, &b, &c);
    check("main_keeps_ids", a == 100 && b == 200 && c == 300, (i64)a);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        u32 x = 0, y = 0, z = 0;
        syscall3(SYS_getresgid, &x, &y, &z);
        exit_group(x == 100 && y == 200 && z == 300 ? 0 : 1);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    check("child_has_main_ids", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    return finish();
}
