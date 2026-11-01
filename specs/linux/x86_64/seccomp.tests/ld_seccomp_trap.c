/* SECCOMP_RET_TRAP | data with a SIGSYS handler: the matched call is not
 * run; SIGSYS is delivered with si_code SYS_SECCOMP (1), si_errno = the
 * filter's data, si_syscall = the call's number and si_arch =
 * AUDIT_ARCH_X86_64 (0xc000003e); after the handler returns, the
 * interrupted call appears to return its own syscall number (the
 * registers are as they were at entry). The process goes on normally.
 */
#include "a_sys.h"

#define SYS_rt_sigreturn 15
#define SYS_prctl 157
#define SYS_seccomp 317
#define PR_SET_NO_NEW_PRIVS 38 /* linux/prctl.h */
/* linux/seccomp.h, linux/filter.h, linux/bpf_common.h, linux/audit.h */
#define SECCOMP_SET_MODE_FILTER 1
#define SECCOMP_RET_TRAP 0x00030000U
#define SECCOMP_RET_ALLOW 0x7fff0000U
#define BPF_LD_W_ABS 0x20
#define BPF_JEQ_K 0x15
#define BPF_RET_K 0x06
#define AUDIT_ARCH_X86_64 0xc000003eU
/* asm-generic/siginfo.h */
#define SIGSYS 31
#define SA_SIGINFO 4
#define SYS_SECCOMP 1

struct sock_filter {
    u16 code;
    u8 jt, jf;
    u32 k;
};
struct sock_fprog {
    u16 len;
    struct sock_filter *filter;
};
/* siginfo_t with the _sigsys member */
struct sigsys_info {
    i32 si_signo, si_errno, si_code, pad;
    u64 call_addr;
    i32 syscall;
    u32 arch;
};

static struct sock_filter insns[4] = {
    {BPF_LD_W_ABS, 0, 0, 0},
    {BPF_JEQ_K, 0, 1, SYS_getppid},
    {BPF_RET_K, 0, 0, SECCOMP_RET_TRAP | 77},
    {BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW},
};
static struct sock_fprog prog = {4, insns};
static volatile i32 hits, signo, code, err, nr;
static volatile u32 arch;

void ld_restorer(void);
__asm__(".text\nld_restorer:\n    mov $15, %eax\n    syscall\n    hlt\n");

static void handler(int sig, struct sigsys_info *info, void *uc) {
    (void)uc;
    hits++;
    signo = sig;
    code = info->si_code;
    err = info->si_errno;
    nr = info->syscall;
    arch = info->arch;
}

static int child(void) {
    struct a_sigaction sa = {(u64)handler, SA_SIGINFO | SA_RESTORER, (u64)ld_restorer, 0};
    expect("sigaction", syscall4(SYS_rt_sigaction, SIGSYS, &sa, 0, 8), 0);
    expect("set_nnp", syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("install_trap", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog), 0);
    expect("trapped_call_returns_its_number", syscall0(SYS_getppid), SYS_getppid);
    expect("handler_ran_once", hits, 1);
    expect("si_signo", signo, SIGSYS);
    expect("si_code_sys_seccomp", code, SYS_SECCOMP);
    expect("si_errno_is_data", err, 77);
    expect("si_syscall", nr, SYS_getppid);
    expect("si_arch", arch, AUDIT_ARCH_X86_64);
    check("other_calls_fine", syscall0(SYS_getuid) >= 0, 0);
    expect("trapped_again", syscall0(SYS_getppid), SYS_getppid);
    expect("handler_ran_twice", hits, 2);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
