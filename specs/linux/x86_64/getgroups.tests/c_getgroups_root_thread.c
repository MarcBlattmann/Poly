/* Credentials are per thread at the system call level: a raw setgroups in
 * one thread changes what getgroups reports in that thread only; the
 * main thread still sees its old list.
 * poly-diff: needs=root (setgroups needs CAP_SETGID)
 * poly-diff: syscalls=off (two threads)
 */
#include "poly_ipc.h"

static char stack[65536] __attribute__((aligned(16)));
static volatile unsigned changed, checked, done;
static volatile int ctid;
static volatile i64 t_set, t_count, t_first, t_after_main;

static int thread_main(void *arg) {
    (void)arg;
    unsigned mine[1] = { 31337 };
    t_set = syscall2(SYS_setgroups, 1, mine);
    unsigned got[4];
    t_count = syscall2(SYS_getgroups, 4, got);
    t_first = got[0];
    gate_open(&changed);
    gate_wait(&checked);
    t_after_main = syscall2(SYS_getgroups, 0, 0);
    gate_open(&done);
    return 0;
}

int test_main(void) {
    unsigned set[2] = { 1, 2 };
    i64 r = syscall2(SYS_setgroups, 2, set);
    check("setgroups_ok", r == 0, r);
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&ctid, (int *)&ctid, thread_main, 0);
    check("clone_thread", t > 0, t);
    gate_wait(&changed);
    check("thread_setgroups_ok", t_set == 0, t_set);
    check("thread_sees_own_change", t_count == 1 && t_first == 31337, t_count);
    unsigned got[4];
    r = syscall2(SYS_getgroups, 4, got);
    check("main_thread_unchanged", r == 2 && got[0] == 1 && got[1] == 2, r);
    unsigned more[3] = { 4, 5, 6 };
    syscall2(SYS_setgroups, 3, more);
    gate_open(&checked);
    gate_wait(&done);
    check("thread_unaffected_by_main", t_after_main == 1, t_after_main);
    return finish();
}
