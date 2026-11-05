/* SECCOMP_RET_TRAP: the ucontext a SIGSYS handler sees, and emulating the
 * call from it. At signal entry the trapped call's registers are as at
 * the `syscall` instruction: RAX holds the call's number (the call was
 * not run), RDI, RSI, RDX, R10, R8 and R9 its six arguments, RIP the
 * address just after the instruction (= si_call_addr). Whatever the
 * handler stores in the ucontext's RAX is what the call returns once the
 * handler returns (a value, zero, or a negative errno).
 */
#include "a_sys.h"

#define SYS_prctl 157
#define SYS_seccomp 317
#define PR_SET_NO_NEW_PRIVS 38 /* linux/prctl.h */
/* linux/seccomp.h, linux/filter.h, linux/bpf_common.h */
#define SECCOMP_SET_MODE_FILTER 1
#define SECCOMP_RET_TRAP 0x00030000U
#define SECCOMP_RET_ALLOW 0x7fff0000U
#define BPF_LD_W_ABS 0x20
#define BPF_JEQ_K 0x15
#define BPF_RET_K 0x06
/* asm-generic/siginfo.h */
#define SIGSYS 31
#define SA_SIGINFO 4
/* asm/ucontext.h, asm/sigcontext.h: struct sigcontext after 40 bytes;
 * register order r8, r9, r10, r11, r12, r13, r14, r15, rdi, rsi, rbp,
 * rbx, rdx, rax, rcx, rsp, rip */
#define UC_GREGS 40
#define G_R8 0
#define G_R9 1
#define G_R10 2
#define G_RDI 8
#define G_RSI 9
#define G_RDX 12
#define G_RAX 13
#define G_RIP 16
#define EPERM 1 /* asm-generic/errno-base.h */

struct sock_filter {
    u16 code;
    u8 jt, jf;
    u32 k;
};
struct sock_fprog {
    u16 len;
    struct sock_filter *filter;
};
struct sigsys_info {
    i32 si_signo, si_errno, si_code, pad;
    u64 call_addr;
    i32 syscall;
    u32 arch;
};

static struct sock_filter insns[4] = {
    {BPF_LD_W_ABS, 0, 0, 0},
    {BPF_JEQ_K, 0, 1, SYS_getppid},
    {BPF_RET_K, 0, 0, SECCOMP_RET_TRAP | 5},
    {BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW},
};
static struct sock_fprog prog = {4, insns};
static volatile i32 hits;
static volatile u64 regs[17];
static volatile u64 call_addr;
static volatile i64 set_rax;
static volatile i32 set_mode;

void ld_restorer(void);
__asm__(".text\nld_restorer:\n    mov $15, %eax\n    syscall\n    hlt\n");

/* ld_call6(n, a1..a5): a raw `syscall` of n with arguments a1..a5 and
 * 0x6666 as the sixth; returns rax. */
i64 ld_call6(i64 n, i64 a1, i64 a2, i64 a3, i64 a4, i64 a5);
extern char ld_after6[];
__asm__(".text\n"
        "ld_call6:\n    mov %rdi, %rax\n    mov %rsi, %rdi\n    mov %rdx, %rsi\n    mov %rcx, %rdx\n"
        "    mov %r8, %r10\n    mov %r9, %r8\n    mov $0x6666, %r9\n    syscall\nld_after6:\n    ret\n");

static void handler(int sig, struct sigsys_info *info, void *uc) {
    (void)sig;
    u64 *g = (u64 *)((char *)uc + UC_GREGS);
    hits++;
    call_addr = info->call_addr;
    for (int i = 0; i < 17; i++) regs[i] = g[i];
    if (set_mode) g[G_RAX] = (u64)set_rax;
}

static int child(void) {
    struct a_sigaction sa = {(u64)handler, SA_SIGINFO | SA_RESTORER, (u64)ld_restorer, 0};
    expect("sigaction", syscall4(SYS_rt_sigaction, SIGSYS, &sa, 0, 8), 0);
    expect("set_nnp", syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("install_trap", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog), 0);

    expect("returns_its_number", ld_call6(SYS_getppid, 0x1111, 0x2222, 0x3333, 0x4444, 0x5555), SYS_getppid);
    expect("hits", hits, 1);
    expect("uc_rax_is_nr", (i64)regs[G_RAX], SYS_getppid);
    expect("uc_rdi_arg1", (i64)regs[G_RDI], 0x1111);
    expect("uc_rsi_arg2", (i64)regs[G_RSI], 0x2222);
    expect("uc_rdx_arg3", (i64)regs[G_RDX], 0x3333);
    expect("uc_r10_arg4", (i64)regs[G_R10], 0x4444);
    expect("uc_r8_arg5", (i64)regs[G_R8], 0x5555);
    expect("uc_r9_arg6", (i64)regs[G_R9], 0x6666);
    check("uc_rip_after_insn", regs[G_RIP] == (u64)ld_after6, (i64)(regs[G_RIP] - (u64)ld_after6));
    check("call_addr_is_uc_rip", call_addr == regs[G_RIP], (i64)(call_addr - regs[G_RIP]));

    /* Emulation: the handler's RAX is the call's result. */
    set_mode = 1;
    set_rax = 1234;
    expect("emulated_value", ld_call6(SYS_getppid, 0, 0, 0, 0, 0), 1234);
    set_rax = -EPERM;
    expect("emulated_errno", ld_call6(SYS_getppid, 0, 0, 0, 0, 0), -EPERM);
    set_rax = 0;
    expect("emulated_zero", ld_call6(SYS_getppid, 0, 0, 0, 0, 0), 0);
    expect("hits_4", hits, 4);
    set_mode = 0;
    expect("unchanged_again", ld_call6(SYS_getppid, 0, 0, 0, 0, 0), SYS_getppid);
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
