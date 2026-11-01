/* SECCOMP_SET_MODE_FILTER needs no_new_privs or CAP_SYS_ADMIN: without
 * either, a valid filter is EACCES (13), but flags (EINVAL), the program
 * pointer (EFAULT) and the program itself (EINVAL) are checked before
 * that. prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) sets the bit (other
 * arguments non-zero or arg2 other than 1: EINVAL; it cannot be cleared:
 * EINVAL); PR_GET_NO_NEW_PRIVS reads it; children inherit it. Then the
 * filter is accepted and PR_GET_SECCOMP reads 2. Run as a normal user
 * (root first becomes uid/gid 65534 without groups in a child), so the
 * output is the same for every caller.
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_prctl 157
#define SYS_seccomp 317
/* linux/prctl.h */
#define PR_GET_SECCOMP 21
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39
/* linux/seccomp.h, linux/filter.h, linux/bpf_common.h */
#define SECCOMP_SET_MODE_FILTER 1
#define SECCOMP_RET_ALLOW 0x7fff0000U
#define BPF_RET_K 0x06

struct sock_filter {
    u16 code;
    u8 jt, jf;
    u32 k;
};
struct sock_fprog {
    u16 len;
    struct sock_filter *filter;
};

static struct sock_filter allow[1] = {{BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW}};
static struct sock_fprog prog = {1, allow};
static struct sock_fprog empty = {0, allow};

static i64 prctl(i64 op, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, op, a, b, c, d); }

static int child(void) {
    if (syscall0(SYS_geteuid) == 0) {
        if (syscall2(SYS_setgroups, 0, 0) || syscall3(SYS_setresgid, 65534, 65534, 65534) ||
            syscall3(SYS_setresuid, 65534, 65534, 65534))
            return 2;
    }
    expect("nnp_starts_0", prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("filter_without_nnp_eacces", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog), -EACCES);
    expect("bad_flags_before_eacces", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0x40, &prog), -EINVAL);
    expect("null_prog_before_eacces", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, 0), -EFAULT);
    expect("empty_prog_before_eacces", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &empty), -EINVAL);

    expect("set_nnp_arg3_einval", prctl(PR_SET_NO_NEW_PRIVS, 1, 1, 0, 0), -EINVAL);
    expect("set_nnp_arg5_einval", prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 1), -EINVAL);
    expect("set_nnp_2_einval", prctl(PR_SET_NO_NEW_PRIVS, 2, 0, 0, 0), -EINVAL);
    expect("set_nnp_0_einval", prctl(PR_SET_NO_NEW_PRIVS, 0, 0, 0, 0), -EINVAL);
    expect("nnp_still_0", prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("set_nnp", prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("nnp_now_1", prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 1);
    expect("set_nnp_again", prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("clear_nnp_einval", prctl(PR_SET_NO_NEW_PRIVS, 0, 0, 0, 0), -EINVAL);
    expect("get_nnp_arg2_einval", prctl(PR_GET_NO_NEW_PRIVS, 1, 0, 0, 0), -EINVAL);

    i64 c = a_fork_sync();
    if (c == 0) syscall1(SYS_exit_group, (int)prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0));
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    expect("child_inherits_nnp", A_WEXITSTATUS(st), 1);

    expect("filter_with_nnp", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog), 0);
    expect("mode_filter", prctl(PR_GET_SECCOMP, 0, 0, 0, 0), 2);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_shared();
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
