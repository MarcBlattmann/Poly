/* sched_getaffinity of other threads: after the caller pinned itself
 * to one CPU, its own mask has only that bit; a fork child and a
 * CLONE_VM task inherit it (read by the child itself and by the parent
 * through the child's pid). No permission is needed to read another
 * process's mask.
 * poly-diff: syscalls=off (several tasks; the result is the kernel's mask size, which depends on the machine)
 */
#include "a_sys.h"

#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204
#define SYS_getcpu 309

static u64 one[16], got[16];
static volatile i32 done, go;
static volatile u64 task_mask[16];
static volatile i64 task_n;

static int task(void *arg) {
    (void)arg;
    u64 m[16] = { 0 };
    task_n = syscall3(SYS_sched_getaffinity, 0, sizeof m, m);
    for (int i = 0; i < 16; i++) task_mask[i] = m[i];
    while (!go) a_relax();
    return 0;
}

static int only_bit(const u64 *m, i64 n, u32 cpu) {
    for (int i = 0; i < n / 8; i++)
        if (m[i] != (i == (int)(cpu / 64) ? 1UL << (cpu % 64) : 0)) return 0;
    return n > 0;
}

int test_main(void) {
    a_block_sigchld();
    u32 cpu = 0;
    syscall3(SYS_getcpu, &cpu, 0, 0);
    one[cpu / 64] = 1UL << (cpu % 64);
    expect("pin", syscall3(SYS_sched_setaffinity, 0, sizeof one, one), 0);
    i64 n = syscall3(SYS_sched_getaffinity, 0, sizeof got, got);
    check("own_mask_one_bit", only_bit(got, n, cpu), n);

    long pid = a_spawn(0, &done, task, 0);
    while (task_n == 0) a_relax();
    memset(got, 0xff, sizeof got);
    i64 m = syscall3(SYS_sched_getaffinity, pid, sizeof got, got);
    check("task_mask_by_pid", only_bit(got, m, cpu), m);
    go = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    check("task_own_view", only_bit((const u64 *)task_mask, task_n, cpu), task_n);

    i64 child = a_fork_sync();
    if (child == 0) {
        u64 c[16] = { 0 };
        i64 k = syscall3(SYS_sched_getaffinity, 0, sizeof c, c);
        syscall1(SYS_exit, only_bit(c, k, cpu) ? 0 : 1);
    }
    i32 status = -1;
    syscall4(SYS_wait4, child, &status, 0, 0);
    expect("fork_child_inherits", status, 0);
    return finish();
}
