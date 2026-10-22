/* Without CAP_SYS_NICE the caller's effective uid must match the real or
 * effective uid of the target, else EPERM (1). Argument errors come first:
 * NULL, bad pointer, unknown policy and bad priority give EINVAL/EFAULT
 * even then. Root forks a target that runs with ruid 65534, euid 65533
 * and callers with other ids.
 * poly-diff: needs=root (switching to other uids needs root)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_DEADLINE 6
#define SCHED_RESET_ON_FORK 0x40000000

static A_UNUSED i64 setsch(i64 pid, i64 policy, i32 prio) {
    i32 p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

static A_UNUSED i64 getsch(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }

static A_UNUSED i64 getprio(i64 pid) {
    i32 p = -1;
    i64 r = syscall2(SYS_sched_getparam, pid, &p);
    return r < 0 ? r : p;
}

/* asm/unistd_64.h */
#define SYS_setresuid 117

static i64 run_as(i64 ruid, i64 euid, i64 target, i64 *out) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i64 r = syscall3(SYS_setresuid, ruid, euid, euid);
        if (r == 0) r = setsch(target, SCHED_BATCH, 0);
        else r = 1000;
        syscall1(SYS_exit_group, r == 0 ? 0 : r == -EPERM ? 1 : 2);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    *out = st;
    return pid;
}

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;
    i64 target = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (target == 0) {
        if (syscall3(SYS_setresuid, 65534, 65533, 65533) != 0) sh[1] = -1;
        sh[0] = 1;
        while (sh[0] != 2) syscall0(24);
        syscall1(SYS_exit_group, 0);
    }
    a_spin_until(&sh[0], 1, A_SPIN_LIMIT);
    expect("target_ids", sh[1], 0);
    i64 st;
    run_as(65534, 65534, target, &st);
    expect("euid_matches_ruid", st, 0);
    run_as(65532, 65533, target, &st);
    expect("euid_matches_euid", st, 0);
    run_as(65533, 65532, target, &st);
    expect("ruid_only_matches", st, 1 << 8);
    run_as(65531, 65531, target, &st);
    expect("no_match", st, 1 << 8);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        expect("drop", syscall3(SYS_setresuid, 65531, 65531, 65531), 0);
        expect("other_other", setsch(target, SCHED_OTHER, 0), -EPERM);
        expect("other_idle", setsch(target, SCHED_IDLE, 0), -EPERM);
        expect("other_reset", setsch(target, SCHED_OTHER | SCHED_RESET_ON_FORK, 0), -EPERM);
        expect("other_null", syscall3(SYS_sched_setscheduler, target, SCHED_OTHER, 0), -EINVAL);
        expect("other_efault", syscall3(SYS_sched_setscheduler, target, SCHED_OTHER, 1), -EFAULT);
        expect("other_badpol", setsch(target, 8, 0), -EINVAL);
        expect("other_badprio", setsch(target, SCHED_OTHER, 3), -EINVAL);
        expect("other_fifo_0", setsch(target, SCHED_FIFO, 0), -EINVAL);
        expect("other_fifo_5", setsch(target, SCHED_FIFO, 5), -EPERM);
        expect("self_ok", setsch(0, SCHED_BATCH, 0), 0);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 cst = -1;
    expect("reap_caller", syscall4(SYS_wait4, pid, &cst, 0, 0), pid);
    expect("caller_passed", cst, 0);
    expect("target_batch", getsch(target), SCHED_BATCH);
    expect("root_any", setsch(target, SCHED_OTHER, 0), 0);
    expect("target_other", getsch(target), SCHED_OTHER);
    sh[0] = 2;
    i32 tst = -1;
    expect("reap_target", syscall4(SYS_wait4, target, &tst, 0, 0), target);
    return finish();
}
