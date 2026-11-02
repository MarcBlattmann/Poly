/* SECCOMP_RET_TRAP: where the SIGSYS says the call was. A raw `syscall`
 * instruction (2 bytes, 0f 05) at a known address is trapped; the SIGSYS's
 * si_call_addr is the address just after the instruction (the call's
 * return address, the instruction's address + 2), not the instruction
 * itself; the ucontext's RIP at signal entry is that same address. Each
 * trapped call reports its own site, and si_syscall, si_arch and si_errno
 * (the filter's 16 data bits, 0 included) come with it.
 */
#include "a_sys.h"

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
/* asm/ucontext.h, asm/sigcontext.h: uc_flags, uc_link, uc_stack (24
 * bytes), then struct sigcontext (r8..r15, rdi, rsi, rbp, rbx, rdx, rax,
 * rcx, rsp, rip, ...) */
#define UC_GREGS 40
#define GREG_RIP 16

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

static struct sock_filter insns[6] = {
    {BPF_LD_W_ABS, 0, 0, 0},
    {BPF_JEQ_K, 0, 1, SYS_getppid},
    {BPF_RET_K, 0, 0, SECCOMP_RET_TRAP | 0x1234},
    {BPF_JEQ_K, 0, 1, SYS_getpgrp},
    {BPF_RET_K, 0, 0, SECCOMP_RET_TRAP | 0},
    {BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW},
};
static struct sock_fprog prog = {6, insns};
static volatile i32 hits, signo, code, err, nr;
static volatile u32 arch;
static volatile u64 call_addr, rip;

void ld_restorer(void);
__asm__(".text\nld_restorer:\n    mov $15, %eax\n    syscall\n    hlt\n");

/* Two call sites, each a raw `syscall` of the number in rdi:
 * ld_site_a / ld_site_b are the instruction, ld_after_a / ld_after_b the
 * byte after it. */
i64 ld_call_a(i64 n);
i64 ld_call_b(i64 n);
extern char ld_site_a[], ld_after_a[], ld_site_b[], ld_after_b[];
__asm__(".text\n"
        "ld_call_a:\n    mov %rdi, %rax\nld_site_a:\n    syscall\nld_after_a:\n    ret\n"
        "ld_call_b:\n    mov %rdi, %rax\n    nop\n    nop\n    nop\nld_site_b:\n    syscall\nld_after_b:\n    ret\n");

static void handler(int sig, struct sigsys_info *info, void *uc) {
    hits++;
    signo = sig;
    code = info->si_code;
    err = info->si_errno;
    nr = info->syscall;
    arch = info->arch;
    call_addr = info->call_addr;
    rip = ((u64 *)((char *)uc + UC_GREGS))[GREG_RIP];
}

static int child(void) {
    struct a_sigaction sa = {(u64)handler, SA_SIGINFO | SA_RESTORER, (u64)ld_restorer, 0};
    expect("sigaction", syscall4(SYS_rt_sigaction, SIGSYS, &sa, 0, 8), 0);
    expect("set_nnp", syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("install_trap", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog), 0);
    expect("site_a_is_2_bytes", ld_after_a - ld_site_a, 2);

    expect("a_returns_its_number", ld_call_a(SYS_getppid), SYS_getppid);
    expect("a_hits", hits, 1);
    expect("a_si_signo", signo, SIGSYS);
    expect("a_si_code", code, SYS_SECCOMP);
    expect("a_si_errno_is_data", err, 0x1234);
    expect("a_si_syscall", nr, SYS_getppid);
    expect("a_si_arch", arch, AUDIT_ARCH_X86_64);
    expect("a_call_addr_minus_insn", (i64)(call_addr - (u64)ld_site_a), 2);
    check("a_call_addr_is_after", call_addr == (u64)ld_after_a, (i64)(call_addr - (u64)ld_after_a));
    check("a_uc_rip_is_after", rip == (u64)ld_after_a, (i64)(rip - (u64)ld_after_a));

    expect("b_returns_its_number", ld_call_b(SYS_getpgrp), SYS_getpgrp);
    expect("b_hits", hits, 2);
    expect("b_si_errno_data_0", err, 0);
    expect("b_si_syscall", nr, SYS_getpgrp);
    expect("b_call_addr_minus_insn", (i64)(call_addr - (u64)ld_site_b), 2);
    check("b_call_addr_is_after", call_addr == (u64)ld_after_b, (i64)(call_addr - (u64)ld_after_b));
    check("b_uc_rip_is_after", rip == (u64)ld_after_b, (i64)(rip - (u64)ld_after_b));

    /* The same number from the other site reports that site. */
    expect("a_getpgrp", ld_call_a(SYS_getpgrp), SYS_getpgrp);
    check("a_getpgrp_call_addr", call_addr == (u64)ld_after_a, (i64)(call_addr - (u64)ld_after_a));
    expect("hits_3", hits, 3);
    check("untrapped_call", ld_call_b(SYS_getuid) >= 0, 0);
    expect("untrapped_no_signal", hits, 3);
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
