/* Reading needs no permission: a child that dropped to uid/gid 65534 reads
 * its root parent's SCHED_FIFO policy, and the parent reads the child's
 * SCHED_BATCH.
 * poly-diff: needs=root (switching to another user and real-time policies)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_setresuid 117
#define SYS_setresgid 119

/* linux/sched.h */
#define SCHED_FIFO 1
#define SCHED_BATCH 3

int test_main(void) {
    a_block_sigchld();
    i32 p = 15;
    expect("to_fifo", syscall3(SYS_sched_setscheduler, 0, SCHED_FIFO, &p), 0);
    i64 c = a_fork_sync();
    if (c == 0) {
        i32 zero = 0;
        expect("child_gid", syscall3(SYS_setresgid, 65534, 65534, 65534), 0);
        expect("child_uid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
        expect("child_euid", syscall0(SYS_geteuid), 65534);
        expect("child_reads_root", syscall1(SYS_sched_getscheduler, syscall0(SYS_getppid)), SCHED_FIFO);
        expect("child_reads_pid1", syscall1(SYS_sched_getscheduler, 1) >= 0, 1);
        expect("child_to_batch", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &zero), 0);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    expect("parent_reads_child", syscall1(SYS_sched_getscheduler, c), SCHED_BATCH);
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, c, &st, 0, 0), c);
    check("child_ok", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    return finish();
}
