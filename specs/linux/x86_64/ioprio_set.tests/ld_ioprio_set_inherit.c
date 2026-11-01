/* The I/O priority is per thread and inherited: a child made by fork, or
 * by clone with CLONE_VM (a separate task sharing the memory), starts with
 * the value its parent had, and later changes in either task do not
 * affect the other. ioprio_set(IOPRIO_WHO_PROCESS, 0, ...) in the child
 * changes only the child.
 */
#include "a_sys.h"

#define SYS_ioprio_set 251
#define SYS_ioprio_get 252
#define IOPRIO_WHO_PROCESS 1 /* linux/ioprio.h */
#define PRIO(c, l) (((c) << 13) | (l))

static i64 set(i64 prio) { return syscall3(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, prio); }
static i64 get(void) { return syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0); }

static volatile i32 done, step;
static volatile i64 seen1, seen2, setres;

static int vm_child(void *arg) {
    (void)arg;
    seen1 = get();
    setres = set(PRIO(3, 1));
    seen2 = get();
    step = 1;
    while (step != 2) a_relax();
    seen1 = get();
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    expect("set_be3", set(PRIO(2, 3)), 0);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i64 v = get();
        i64 r = set(PRIO(2, 6));
        syscall1(SYS_exit, (v == PRIO(2, 3) ? 0 : 1) | (r == 0 && get() == PRIO(2, 6) ? 0 : 2));
    }
    i32 status = -1;
    expect("fork_child_reaped", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("fork_child_inherited_and_set_own", status, 0);
    expect("parent_unchanged_by_child", get(), PRIO(2, 3));

    expect("set_idle4", set(PRIO(3, 4)), 0);
    a_spawn(0, &done, vm_child, 0);
    if (!a_spin_until(&step, 1, A_SPIN_LIMIT)) return finish();
    expect("vm_child_inherited", seen1, PRIO(3, 4));
    expect("vm_child_set_ok", setres, 0);
    expect("vm_child_own_value", seen2, PRIO(3, 1));
    expect("parent_unchanged_by_vm_child", get(), PRIO(3, 4));
    expect("parent_sets_be0", set(PRIO(2, 0)), 0);
    step = 2;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    expect("vm_child_unchanged_by_parent", seen1, PRIO(3, 1));
    return finish();
}
