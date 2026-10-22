/* The answer does not depend on the caller: a child that switched itself
 * to SCHED_BATCH, and one that switched to SCHED_IDLE (both allowed
 * unprivileged), get the same values as the parent. */
#include "a_sys.h"

#define SYS_sched_get_priority_min 147 /* arch/x86/entry/syscalls/syscall_64.tbl */
#define A_EINVAL 22                    /* asm-generic/errno-base.h */

#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

static int probe(void) {
    int bad = 0;
    if (syscall1(SYS_sched_get_priority_min, 0) != 0) bad |= 1;
    if (syscall1(SYS_sched_get_priority_min, 1) != 1) bad |= 2;
    if (syscall1(SYS_sched_get_priority_min, 2) != 1) bad |= 4;
    if (syscall1(SYS_sched_get_priority_min, 3) != 0) bad |= 8;
    if (syscall1(SYS_sched_get_priority_min, 4) != -A_EINVAL) bad |= 16;
    return bad;
}

static i64 in_child(i64 policy) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i32 param = 0;
        if (syscall3(SYS_sched_setscheduler, 0, policy, &param) != 0) syscall1(SYS_exit_group, 100);
        if ((syscall1(SYS_sched_getscheduler, 0) & 0xff) != policy) syscall1(SYS_exit_group, 101);
        syscall1(SYS_exit_group, probe());
    }
    i32 st = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &st, 0, 0) != pid) return -1;
    return A_WIFEXITED(st) ? A_WEXITSTATUS(st) : -2;
}

int test_main(void) {
    a_block_sigchld();
    expect("parent_values", probe(), 0);
    expect("batch_child_values", in_child(3), 0);
    expect("idle_child_values", in_child(5), 0);
    return finish();
}
