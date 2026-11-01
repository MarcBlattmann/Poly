/* With CAP_SYS_ADMIN a process may install a seccomp filter without
 * no_new_privs: the call succeeds and no_new_privs stays 0. The filter
 * applies at once (getppid fails with the filter's errno, 13) and is
 * inherited by children; filters stay when the process becomes another
 * user (setresuid), which then cannot add one without no_new_privs.
 * poly-diff: needs=root (CAP_SYS_ADMIN replaces no_new_privs)
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_prctl 157
#define SYS_seccomp 317
/* linux/prctl.h */
#define PR_GET_SECCOMP 21
#define PR_GET_NO_NEW_PRIVS 39
/* linux/seccomp.h, linux/filter.h, linux/bpf_common.h */
#define SECCOMP_SET_MODE_FILTER 1
#define SECCOMP_RET_ERRNO 0x00050000U
#define SECCOMP_RET_ALLOW 0x7fff0000U
#define BPF_LD_W_ABS 0x20
#define BPF_JEQ_K 0x15
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

static struct sock_filter insns[4] = {
    {BPF_LD_W_ABS, 0, 0, 0},
    {BPF_JEQ_K, 0, 1, SYS_getppid},
    {BPF_RET_K, 0, 0, SECCOMP_RET_ERRNO | EACCES},
    {BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW},
};
static struct sock_fprog prog = {4, insns};

static int child(void) {
    expect("nnp_0", syscall5(SYS_prctl, PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("install_without_nnp", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog), 0);
    expect("nnp_still_0", syscall5(SYS_prctl, PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("mode_2", syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0), 2);
    expect("getppid_eacces", syscall0(SYS_getppid), -EACCES);

    i64 c = a_fork_sync();
    if (c == 0) syscall1(SYS_exit_group, syscall0(SYS_getppid) == -EACCES ? 0 : 1);
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    expect("child_inherits", st, 0);

    expect("drop_groups", syscall2(SYS_setgroups, 0, 0), 0);
    expect("drop_gid", syscall3(SYS_setresgid, 65534, 65534, 65534), 0);
    expect("drop_uid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
    expect("filter_kept_after_setuid", syscall0(SYS_getppid), -EACCES);
    expect("user_needs_nnp_eacces", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog), -EACCES);
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
