/* Supplementary groups belong to each thread's credentials: the raw
 * setgroups system call in a second thread changes what getgroups reads
 * in that thread only; the main thread still reads its own list. A child
 * made by fork inherits the list of the thread that forked.
 * poly-diff: needs=root (setgroups needs CAP_SETGID)
 * poly-diff: syscalls=off (a thread runs beside the main thread)
 */
#include "a_sys.h"

#define SYS_getgroups 115
#define SYS_setgroups 116
#define SYS_futex 202

static volatile i64 t_set, t_count;
static volatile u32 t_list[4];
static volatile i32 thread_done;

static int thread_fn(void *arg) {
    (void)arg;
    u32 g[2] = { 600, 500 };
    t_set = syscall2(SYS_setgroups, 2, g);
    u32 out[4] = { 0, 0, 0, 0 };
    t_count = syscall2(SYS_getgroups, 4, out);
    for (int i = 0; i < 4; i++) t_list[i] = out[i];
    return 0;
}

int test_main(void) {
    u32 mine[3] = { 30, 10, 20 };
    i64 r = syscall2(SYS_setgroups, 3, mine);
    check("main_setgroups", r == 0, r);

    thread_done = -1;
    i64 t = a_clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM |
                        CLONE_CHILD_CLEARTID,
                    a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&thread_done, 0, thread_fn, 0);
    check("thread_started", t > 0, t);
    while (thread_done != 0) syscall6(SYS_futex, (i64)&thread_done, 0, thread_done, 0, 0, 0);
    check("thread_setgroups", t_set == 0, t_set);
    check("thread_reads_its_list", t_count == 2 && t_list[0] == 500 && t_list[1] == 600, t_count);

    u32 out[4] = { 0, 0, 0, 0 };
    r = syscall2(SYS_getgroups, 4, out);
    check("main_keeps_its_list", r == 3 && out[0] == 10 && out[1] == 20 && out[2] == 30, r);

    i64 c = a_fork_sync();
    if (c == 0) {
        u32 o[4] = { 0, 0, 0, 0 };
        i64 n = syscall2(SYS_getgroups, 4, o);
        exit_group(n == 3 && o[0] == 10 && o[1] == 20 && o[2] == 30 ? 0 : 1);
    }
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    check("fork_child_inherits", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    return finish();
}
