/* PR_GET_SECCOMP (21) returns the calling thread's seccomp mode as the
 * result: 0 (none) or 2 (filter; e.g. when the test itself runs inside
 * a container that filters), its other arguments ignored. After
 * installing a filter (here an allow-everything filter, with
 * PR_SET_NO_NEW_PRIVS first so no privilege is needed) it returns 2. In
 * strict mode (PR_SET_SECCOMP 1) prctl is not an allowed call: calling
 * PR_GET_SECCOMP kills the thread with SIGKILL. Strict mode can only be
 * entered from mode 0: under a filter, PR_SET_SECCOMP 1 is EINVAL.
 * poly-diff: syscalls=off (the starting mode, and so which calls a child makes, depends on the environment the test runs in)
 */
#include "a_sys.h"

#define SYS_prctl 157
#define PR_GET_SECCOMP 21
#define PR_SET_SECCOMP 22
#define PR_SET_NO_NEW_PRIVS 38
/* linux/seccomp.h */
#define SECCOMP_MODE_STRICT 1
#define SECCOMP_MODE_FILTER 2
#define SECCOMP_RET_ALLOW 0x7fff0000U
/* linux/filter.h, linux/bpf_common.h */
#define BPF_RET 0x06
#define BPF_K 0x00

struct k_sock_filter {
    u16 code;
    u8 jt, jf;
    u32 k;
};
struct k_sock_fprog {
    u16 len;
    struct k_sock_filter *filter;
};

int test_main(void) {
    a_block_sigchld();
    i64 mode = syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0);
    check("initial_0_or_2", mode == 0 || mode == 2, mode);
    expect("args_ignored", syscall5(SYS_prctl, PR_GET_SECCOMP, 1, 2, 3, 4), mode);

    i64 c = a_fork_sync();
    if (c == 0) {
        struct k_sock_filter allow = { BPF_RET | BPF_K, 0, 0, SECCOMP_RET_ALLOW };
        struct k_sock_fprog prog = { 1, &allow };
        syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        if (syscall3(SYS_prctl, PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &prog) != 0) syscall1(SYS_exit, 100);
        syscall1(SYS_exit, (int)syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0));
    }
    i32 status = -1;
    syscall4(SYS_wait4, c, &status, 0, 0);
    check("filter_mode_2", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 2, status);

    c = a_fork_sync();
    if (c == 0) {
        i64 r = syscall2(SYS_prctl, PR_SET_SECCOMP, SECCOMP_MODE_STRICT);
        if (r != 0) syscall1(SYS_exit, r == -EINVAL ? 22 : 100);
        syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0);
        syscall1(SYS_exit, 1);
    }
    syscall4(SYS_wait4, c, &status, 0, 0);
    if (mode == 0)
        check("strict_mode", A_WIFSIGNALED(status) && A_WTERMSIG(status) == SIGKILL, status);
    else
        check("strict_mode", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 22, status);

    /* With a filter installed (in a child), strict mode is EINVAL. */
    c = a_fork_sync();
    if (c == 0) {
        struct k_sock_filter allow = { BPF_RET | BPF_K, 0, 0, SECCOMP_RET_ALLOW };
        struct k_sock_fprog prog = { 1, &allow };
        syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        syscall3(SYS_prctl, PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &prog);
        syscall1(SYS_exit, syscall2(SYS_prctl, PR_SET_SECCOMP, SECCOMP_MODE_STRICT) == -EINVAL ? 0 : 1);
    }
    syscall4(SYS_wait4, c, &status, 0, 0);
    expect("strict_under_filter_einval", status, 0);
    expect("parent_mode_unchanged", syscall5(SYS_prctl, PR_GET_SECCOMP, 0, 0, 0, 0), mode);
    return finish();
}
