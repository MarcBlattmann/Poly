/* uretprobe(2) (335) exists only to be called from the kernel's own
 * uretprobe trampoline. Called from anywhere else the kernel forces
 * SIGILL on the caller: with the default action the process dies of
 * SIGILL, as root and as any user, and the call does not return. A child
 * that blocks SIGILL or ignores it dies of SIGILL all the same (the signal
 * is forced: unblocked, and its action reset to the default).
 * The children make themselves non-dumpable first, so no core dump is
 * written.
 * poly-diff: syscalls=off (child processes killed by a signal)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_ptrace 101
#define SYS_execve 59
#define SYS_getppid 110
#define SYS_setrlimit 160
#define SYS_modify_ldt 154
#define SYS_vhangup 153
#define SYS_uretprobe 335
#define SYS_uprobe 336
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define ENOSYS 38
#define ENOEXEC 8
/* linux/sched.h */
#define CLONE_UNTRACED 0x00800000
/* linux/ptrace.h, asm/ptrace-abi.h */
#define PTRACE_TRACEME 0
#define PTRACE_PEEKTEXT 1
#define PTRACE_PEEKDATA 2
#define PTRACE_PEEKUSR 3
#define PTRACE_POKETEXT 4
#define PTRACE_POKEDATA 5
#define PTRACE_POKEUSR 6
#define PTRACE_CONT 7
#define PTRACE_KILL 8
#define PTRACE_SINGLESTEP 9
#define PTRACE_GETREGS 12
#define PTRACE_SETREGS 13
#define PTRACE_GETFPREGS 14
#define PTRACE_SETFPREGS 15
#define PTRACE_ATTACH 16
#define PTRACE_DETACH 17
#define PTRACE_SYSCALL 24
#define PTRACE_SETOPTIONS 0x4200
#define PTRACE_GETEVENTMSG 0x4201
#define PTRACE_GETSIGINFO 0x4202
#define PTRACE_SETSIGINFO 0x4203
#define PTRACE_GETREGSET 0x4204
#define PTRACE_SETREGSET 0x4205
#define PTRACE_SEIZE 0x4206
#define PTRACE_INTERRUPT 0x4207
#define PTRACE_LISTEN 0x4208
#define PTRACE_GETSIGMASK 0x420a
#define PTRACE_SETSIGMASK 0x420b
#define PTRACE_GET_SYSCALL_INFO 0x420e
#define PTRACE_O_TRACESYSGOOD 0x1
#define PTRACE_O_TRACEFORK 0x2
#define PTRACE_O_TRACEVFORK 0x4
#define PTRACE_O_TRACECLONE 0x8
#define PTRACE_O_TRACEEXEC 0x10
#define PTRACE_O_TRACEVFORKDONE 0x20
#define PTRACE_O_TRACEEXIT 0x40
#define PTRACE_O_EXITKILL 0x100000
#define PTRACE_EVENT_FORK 1
#define PTRACE_EVENT_VFORK 2
#define PTRACE_EVENT_CLONE 3
#define PTRACE_EVENT_EXEC 4
#define PTRACE_EVENT_EXIT 6
#define PTRACE_EVENT_STOP 128
#define PTRACE_SYSCALL_INFO_NONE 0
#define PTRACE_SYSCALL_INFO_ENTRY 1
#define PTRACE_SYSCALL_INFO_EXIT 2
/* linux/elf.h */
#define NT_PRSTATUS 1
#define NT_PRFPREG 2
/* linux/audit.h */
#define AUDIT_ARCH_X86_64 0xc000003eU
/* linux/wait.h: a stopped child's status */
#define W_STOPPED(s) (((s) & 0xff) == 0x7f)
#define W_STOPSIG(s) (((s) >> 8) & 0xff)
#define W_EVENT(s) (((s) >> 16) & 0xff)
/* asm-generic/resource.h */
#define RLIMIT_CORE 4

/* struct user_regs_struct, asm/user.h (x86-64): 27 registers, 216 bytes. */
struct k_regs {
    u64 r15, r14, r13, r12, rbp, rbx, r11, r10, r9, r8, rax, rcx, rdx, rsi, rdi, orig_rax, rip, cs, eflags, rsp, ss,
        fs_base, gs_base, ds, es, fs, gs;
};
#define REG_OFF(name) ((u64) & ((struct k_regs *)0)->name)
/* struct user_fpregs_struct, asm/user.h: 512 bytes (fxsave layout). */
struct k_fpregs {
    unsigned short cwd, swd, ftw, fop;
    u64 rip, rdp;
    unsigned mxcsr, mxcr_mask;
    unsigned st_space[32], xmm_space[64], padding[24];
};
/* struct ptrace_syscall_info, linux/ptrace.h */
struct k_syscall_info {
    unsigned char op, pad[3];
    unsigned arch;
    u64 instruction_pointer, stack_pointer;
    union {
        struct {
            u64 nr, args[6];
        } entry;
        struct {
            i64 rval;
            unsigned char is_error;
        } exit;
    } u;
};

static __attribute__((unused)) i64 ptrace(i64 req, i64 pid, u64 addr, u64 data) {
    return syscall4(SYS_ptrace, req, pid, addr, data);
}
/* A child the test's own tracer (if any) does not follow, so the test may
 * trace it: fork with CLONE_UNTRACED. */
static __attribute__((unused)) i64 fork_untraced(void) {
    return syscall5(SYS_clone, CLONE_UNTRACED | SIGCHLD, 0, 0, 0, 0);
}
/* Raw wait status of `pid` (any state), -1 when the wait failed. */
static __attribute__((unused)) int wait_any(i64 pid) {
    int st = 0;
    i64 r = syscall4(SYS_wait4, pid, &st, WALL, 0);
    return r == pid ? st : -1;
}
static __attribute__((unused)) int stopped_by(int st, int sig) { return st != -1 && W_STOPPED(st) && W_STOPSIG(st) == sig; }
static __attribute__((unused)) i64 self_stop(void) { return syscall2(SYS_kill, getpid(), SIGSTOP); }
static __attribute__((unused)) void no_core(void) {
    u64 lim[2] = { 0, 0 };
    syscall2(SYS_setrlimit, RLIMIT_CORE, lim);
}
static __attribute__((unused)) void expect(const char *name, i64 got, i64 want) { check(name, got == want, got); }

#define PR_SET_DUMPABLE 4

static int run(int how) {
    i64 pid = fork();
    if (pid == 0) {
        syscall2(SYS_prctl, PR_SET_DUMPABLE, 0);
        if (how == 1) sig_mask(SIG_BLOCK, SIGBIT(SIGILL), 0);
        if (how == 2) sig_action(SIGILL, SIG_IGN, 0, 0);
        syscall0(SYS_uretprobe);
        exit_group(9);
    }
    return wait_status(pid);
}

int test_main(void) {
    int st = run(0);
    check("default_killed_by_sigill", st != -1 && W_SIGNALED(st) && W_TERMSIG(st) == SIGILL, st);
    st = run(1);
    check("blocked_killed_by_sigill", st != -1 && W_SIGNALED(st) && W_TERMSIG(st) == SIGILL, st);
    st = run(2);
    check("ignored_killed_by_sigill", st != -1 && W_SIGNALED(st) && W_TERMSIG(st) == SIGILL, st);
    return finish();
}
