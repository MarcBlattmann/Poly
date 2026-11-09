/* A seccomp filter is inherited by every task its owner creates afterwards:
 * by fork (clone with SIGCHLD), by clone with CLONE_VFORK|CLONE_VM on its
 * own stack, by clone3, and by a child that changes its user ids. It
 * cannot be removed: a later allow-all filter changes nothing. What the
 * children do (installing more filters, changing ids, being killed) never
 * reaches the task that made them.
 * The filter denies sched_yield with ERRNO 5 and allows everything else.
 * poly-diff: syscalls=off (several processes)
 * poly-diff: needs=root (the setresuid step)
 */
#include "p_seccomp.h"

#define SYS_clone3 435
#define SYS_setresuid 117
#define SYS_setresgid 119

struct clone_args { /* linux/sched.h, version 0 (64 bytes) */
    u64 flags, pidfd, child_tid, parent_tid, exit_signal, stack, stack_size, tls;
};

static int yield_is_denied(void) { return syscall0(SYS_sched_yield) == -5; }

static int vfork_child(void *arg) {
    (void)arg;
    return yield_is_denied() ? 0 : 1;
}

static void filtered(void) {
    struct pk_insn f[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_sched_yield, 0, 1), PK_ERR(5), PK_ALLOW};
    struct pk_prog p = {4, f};
    syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    expect("install", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &p), 0);
    expect("self_denied", syscall0(SYS_sched_yield), -5);
    i32 st = -1;

    /* fork */
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, yield_is_denied() ? 0 : 1);
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("fork_child_inherits", st, 0);

    /* an ordinary fork with no vfork wait */
    pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) syscall1(SYS_exit_group, yield_is_denied() ? 0 : 1);
    st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("plain_fork_child_inherits", st, 0);

    /* vfork-style: shares memory, own stack */
    pid = a_clone(CLONE_VM | CLONE_VFORK | SIGCHLD, a_stacks[0] + A_STACK_SIZE, 0, 0, 0, vfork_child, 0);
    st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("vfork_child_inherits", st, 0);

    /* clone3 as fork */
    struct clone_args ca;
    memset(&ca, 0, sizeof ca);
    ca.exit_signal = SIGCHLD;
    pid = syscall2(SYS_clone3, &ca, sizeof ca);
    if (pid == 0) syscall1(SYS_exit_group, yield_is_denied() ? 0 : 1);
    check("clone3_works_under_filter", pid > 0, pid);
    st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("clone3_child_inherits", st, 0);

    /* an allow-all filter added later removes nothing */
    struct pk_insn al[] = {PK_ALLOW};
    struct pk_prog pa = {1, al};
    expect("install_allow_all", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &pa), 0);
    expect("still_denied", syscall0(SYS_sched_yield), -5);

    /* a child's own extra filter does not reach its parent */
    pid = a_fork_sync();
    if (pid == 0) {
        struct pk_insn g[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_getuid, 0, 1), PK_ERR(6), PK_ALLOW};
        struct pk_prog pg = {4, g};
        int ok = syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &pg) == 0;
        ok = ok && syscall0(SYS_getuid) == -6 && yield_is_denied();
        syscall1(SYS_exit_group, ok ? 0 : 1);
    }
    st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_with_extra_filter_ok", st, 0);
    check("parent_getuid_unaffected", syscall0(SYS_getuid) >= 0, 0);

    /* a child that changes its user ids keeps the filter */
    pid = a_fork_sync();
    if (pid == 0) {
        int ok = syscall3(SYS_setresgid, 65534, 65534, 65534) == 0;
        ok = ok && syscall3(SYS_setresuid, 65534, 65534, 65534) == 0;
        ok = ok && yield_is_denied();
        syscall1(SYS_exit_group, ok ? 0 : 1);
    }
    st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("filter_survives_setresuid", st, 0);

    /* a child killed by its own filter leaves the parent running */
    pid = a_fork_sync();
    if (pid == 0) {
        struct pk_insn k[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_getgid, 0, 1), PK_RET(SECCOMP_RET_KILL_PROCESS), PK_ALLOW};
        struct pk_prog pk = {4, k};
        syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &pk);
        syscall0(SYS_getgid);
        syscall1(SYS_exit_group, 0);
    }
    st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    check("grandchild_killed_by_sigsys", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGSYS, st);
    check("parent_survives", syscall0(SYS_getgid) >= 0, 0);
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    pk_in_child(filtered);
    expect("test_process_unfiltered", syscall0(SYS_sched_yield), 0);
    return finish();
}
