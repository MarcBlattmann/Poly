/* prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &sock_fprog), the interface
 * libseccomp and bubblewrap use, installs the same kind of filter as
 * seccomp(SECCOMP_SET_MODE_FILTER, 0, &sock_fprog) and the filter gives the
 * same verdicts: ERRNO makes the probed call return -data, ALLOW runs it,
 * an argument rule sees the 64-bit argument words, KILL ends the process
 * with SIGSYS. The remaining prctl arguments (arg4, arg5) are ignored.
 * PR_GET_SECCOMP then reads 2. Filters installed by the two interfaces
 * stack and the most severe verdict wins. Nothing changes in the parent.
 * poly-diff: syscalls=off (every case forks)
 */
#include "p_seccomp.h"

static i64 verdict(int via, const struct pk_insn *f, u16 n, i64 a0) {
    struct pk_result r;
    pk_try_via(via, &r, f, n, SYS_sched_yield, a0, 0, 0, 0);
    if (r.load != 0) return 100000 + r.load;
    if (!r.ran) return 200000 + (A_WIFSIGNALED(r.status) ? A_WTERMSIG(r.status) : 0);
    return r.ret;
}

static void mode_after_install(void) {
    struct pk_insn allow[] = {PK_ALLOW};
    struct pk_prog p = {1, allow};
    syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    i64 before = syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0);
    check("mode_before_0_or_2", before == 0 || before == 2, before);
    expect("install", syscall5(SYS_prctl, PR_SET_SECCOMP, 2, &p, 0, 0), 0);
    expect("mode_is_2", syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0), 2);
    expect("install_again", syscall5(SYS_prctl, PR_SET_SECCOMP, 2, &p, 0, 0), 0);
    expect("mode_still_2", syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0), 2);
    /* arg4 and arg5 are not looked at */
    expect("install_with_extra_args", syscall5(SYS_prctl, PR_SET_SECCOMP, 2, &p, 0x1234, -1), 0);
}

static void stacked(void) {
    /* first an ERRNO 5 filter through prctl, then an ALLOW-all through seccomp: the ERRNO stays */
    struct pk_insn deny[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_sched_yield, 0, 1), PK_ERR(5), PK_ALLOW};
    struct pk_insn allow[] = {PK_ALLOW};
    struct pk_prog pd = {4, deny};
    struct pk_prog pa = {1, allow};
    syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    expect("install_deny_with_prctl", syscall5(SYS_prctl, PR_SET_SECCOMP, 2, &pd, 0, 0), 0);
    expect("yield_denied", syscall0(SYS_sched_yield), -5);
    expect("install_allow_with_seccomp", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &pa), 0);
    expect("yield_still_denied", syscall0(SYS_sched_yield), -5);
    expect("install_allow_with_prctl", syscall5(SYS_prctl, PR_SET_SECCOMP, 2, &pa, 0, 0), 0);
    expect("yield_still_denied_after_prctl", syscall0(SYS_sched_yield), -5);
    check("other_calls_run", syscall0(SYS_getpid) > 0, 0);
    /* a stricter verdict added later wins for that call */
    struct pk_insn kill[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_getpid, 0, 1), PK_ERR(9), PK_ALLOW};
    struct pk_prog pk = {4, kill};
    expect("install_second_deny", syscall5(SYS_prctl, PR_SET_SECCOMP, 2, &pk, 0, 0), 0);
    expect("getpid_denied", syscall0(SYS_getpid), -9);
    expect("yield_unchanged", syscall0(SYS_sched_yield), -5);
}

int test_main(void) {
    a_block_sigchld();
    {
        struct pk_insn e[] = {PK_ERR(13)};
        expect("errno_via_prctl", verdict(PK_PRCTL, e, 1, 0), -13);
        expect("errno_via_seccomp", verdict(PK_SEC, e, 1, 0), -13);
        struct pk_insn a[] = {PK_ALLOW};
        expect("allow_via_prctl", verdict(PK_PRCTL, a, 1, 0), 0);
        struct pk_insn k[] = {PK_RET(SECCOMP_RET_KILL_PROCESS)};
        expect("kill_process_via_prctl_is_sigsys", verdict(PK_PRCTL, k, 1, 0), 200000 + SIGSYS);
        struct pk_insn kt[] = {PK_RET(SECCOMP_RET_KILL_THREAD)};
        expect("kill_thread_via_prctl_is_sigsys", verdict(PK_PRCTL, kt, 1, 0), 200000 + SIGSYS);
        struct pk_insn arg[] = {PK_LDABS(SD_ARG_LO(0)), PK_JEQ(77, 0, 1), PK_ERR(3), PK_ALLOW};
        expect("argument_rule_hit_via_prctl", verdict(PK_PRCTL, arg, 4, 77), -3);
        expect("argument_rule_miss_via_prctl", verdict(PK_PRCTL, arg, 4, 78), 0);
        expect("argument_rule_hit_via_seccomp", verdict(PK_SEC, arg, 4, 77), -3);
    }
    pk_in_child(mode_after_install);
    pk_in_child(stacked);
    /* the parent was not touched */
    expect("parent_still_runs_the_probed_call", syscall0(SYS_sched_yield), 0);
    return finish();
}
