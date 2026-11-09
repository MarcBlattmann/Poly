/* Errors of prctl(PR_SET_SECCOMP, mode, arg, ...) for the filter mode.
 * mode 0 and modes above 2 (3, 99, -1) are EINVAL, also for a process with
 * privilege and whatever arg is; mode 2 with arg NULL or an unreadable
 * struct is EFAULT; a sock_fprog with length 0 or a NULL filter pointer or
 * a program with a bad opcode is EINVAL. Without no_new_privs and without
 * CAP_SYS_ADMIN the install is EACCES (13) - checked after the mode
 * (EINVAL), the copy of the struct (EFAULT) and the length (EINVAL), but
 * before the instructions are checked (a bad opcode is then EACCES, not
 * EINVAL; the same through seccomp()). With
 * CAP_SYS_ADMIN (root) no_new_privs is not needed. The test runs as root
 * and drops to uid/gid 65534 in children to see the unprivileged answers.
 * poly-diff: needs=root
 * poly-diff: syscalls=off (every case forks)
 */
#include "p_seccomp.h"

#define SYS_setresuid 117
#define SYS_setresgid 119

static struct pk_insn allow[1] = {PK_ALLOW};
static struct pk_insn badprog[2] = {PK_STMT(BPF_LD | BPF_B | BPF_ABS, 0), PK_ALLOW};

#define SET(mode, arg) syscall5(SYS_prctl, PR_SET_SECCOMP, (mode), (arg), 0, 0)

static void as_root(void) {
    struct pk_prog good = {1, allow};
    struct pk_prog len0 = {0, allow};
    struct pk_prog nul = {1, 0};
    struct pk_prog bad = {2, badprog};
    expect("mode_0_einval", SET(0, &good), -EINVAL);
    expect("mode_3_einval", SET(3, &good), -EINVAL);
    expect("mode_99_einval", SET(99, &good), -EINVAL);
    expect("mode_minus_1_einval", SET(-1, &good), -EINVAL);
    expect("mode_2_null_efault", SET(2, 0), -EFAULT);
    expect("mode_2_low_address_efault", SET(2, 8), -EFAULT);
    expect("mode_2_len_0_einval", SET(2, &len0), -EINVAL);
    expect("mode_2_filter_null_einval", SET(2, &nul), -EINVAL);
    expect("mode_2_bad_program_einval", SET(2, &bad), -EINVAL);
    /* root needs no no_new_privs */
    expect("no_nnp_yet", syscall5(SYS_prctl, PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("root_installs_without_nnp", SET(2, &good), 0);
    expect("nnp_still_clear", syscall5(SYS_prctl, PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("mode_2", syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0), 2);
}

static void as_user_without_nnp(void) {
    struct pk_prog good = {1, allow};
    struct pk_prog len0 = {0, allow};
    struct pk_prog bad = {2, badprog};
    expect("setresgid", syscall3(SYS_setresgid, 65534, 65534, 65534), 0);
    expect("setresuid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
    expect("nnp_clear", syscall5(SYS_prctl, PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("user_no_nnp_eacces", SET(2, &good), -EACCES);
    expect("user_mode_0_einval_first", SET(0, &good), -EINVAL);
    expect("user_mode_3_einval_first", SET(3, &good), -EINVAL);
    expect("user_null_efault_first", SET(2, 0), -EFAULT);
    expect("user_len_0_einval_before_eacces", SET(2, &len0), -EINVAL);
    expect("user_bad_opcode_is_eacces_first", SET(2, &bad), -EACCES);
    expect("user_bad_opcode_via_seccomp_is_eacces_first", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &bad), -EACCES);
    /* with no_new_privs the same call works */
    expect("set_nnp", syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("user_with_nnp_installs", SET(2, &good), 0);
    expect("user_with_nnp_mode_2", syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0), 2);
}

static void failed_install_leaves_no_filter(void) {
    /* a rejected program installs nothing: the probed call still runs */
    struct pk_insn deny_then_bad[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_sched_yield, 0, 1), PK_ERR(7), PK_STMT(BPF_LD | BPF_H | BPF_ABS, 0)};
    struct pk_prog p = {4, deny_then_bad};
    syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    expect("rejected", SET(2, &p), -EINVAL);
    expect("probed_call_still_runs", syscall0(SYS_sched_yield), 0);
}

int test_main(void) {
    a_block_sigchld();
    pk_in_child(as_root);
    pk_in_child(as_user_without_nnp);
    pk_in_child(failed_install_leaves_no_filter);
    expect("parent_unfiltered", syscall0(SYS_sched_yield), 0);
    return finish();
}
