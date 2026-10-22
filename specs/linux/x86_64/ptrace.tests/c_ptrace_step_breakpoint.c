/* PTRACE_SINGLESTEP runs one instruction of the tracee and stops it with
 * SIGTRAP (si_code TRAP_TRACE, si_addr the next instruction), rip at the
 * next instruction and its effect done. An int3 (0xcc) written into the
 * tracee's code with PTRACE_POKETEXT stops it with SIGTRAP (si_code
 * SI_KERNEL) with rip just after the int3; the tracer can put the
 * original byte back, set rip back with PTRACE_POKEUSR and let the
 * tracee run on as if nothing happened. Discarding the SIGTRAP (PTRACE_CONT
 * with 0) is what lets it go on.
 * poly-diff: syscalls=off (several processes, in a subtree the reference's tracer does not follow)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_vfork 58
#define SYS_execve 59
#define SYS_ptrace 101
#define SYS_waitid 247
/* linux/sched.h */
#define CLONE_UNTRACED 0x00800000
/* linux/wait.h */
#define WUNTRACED 2
#define WEXITED 4
#define WNOWAIT 0x01000000
#define P_PID 1
/* asm-generic/siginfo.h */
#define TRAP_TRACE 2
#define CLD_TRAPPED 4
/* linux/prctl.h */
#define PR_SET_CHILD_SUBREAPER 36
/* linux/ptrace.h, asm/ptrace-abi.h (x86-64) */
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
#define PTRACE_ARCH_PRCTL 30
#define PTRACE_SYSEMU 31
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
#define PTRACE_EVENT_VFORK_DONE 5
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
/* asm/prctl.h */
#define ARCH_GET_FS 0x1003

/* struct user_regs_struct (sys/user.h, x86-64): 27 words, 216 bytes. */
struct k_regs {
    u64 r15, r14, r13, r12, rbp, rbx, r11, r10, r9, r8, rax, rcx, rdx, rsi, rdi, orig_rax, rip, cs, eflags, rsp, ss,
        fs_base, gs_base, ds, es, fs, gs;
};
/* Offsets in struct user (sys/user.h, x86-64): regs at 0, u_debugreg[8]
 * at 848, sizeof 928. */
#define USER_REG(name) ((u64) & ((struct k_regs *)0)->name)
#define USER_DEBUGREG(n) (848 + 8 * (n))
#define USER_SIZE 928

/* struct ptrace_syscall_info (linux/ptrace.h) */
struct k_syscall_info {
    unsigned char op;
    unsigned char pad[3];
    unsigned arch;
    u64 instruction_pointer;
    u64 stack_pointer;
    union {
        struct {
            u64 nr;
            u64 args[6];
        } entry;
        struct {
            i64 rval;
            unsigned char is_error;
        } exit;
    } u;
};

/* Wait statuses of ptrace stops (ptrace(2)). */
#define STOPPED(sig) (((sig) << 8) | 0x7f)
#define EVENT_STOPPED(sig, ev) (((ev) << 16) | ((sig) << 8) | 0x7f)
#define EXITED(code) ((code) << 8)
#define NOTHING 0x7fffffff

static __attribute__((unused)) i64 ptrace(i64 req, i64 pid, u64 addr, u64 data) {
    return syscall4(SYS_ptrace, req, pid, addr, data);
}

/* Raw wait status of `pid` (child, thread or tracee), or the negative errno. */
static __attribute__((unused)) int wait_task(i64 pid) {
    int s = 0;
    i64 r = syscall4(SYS_wait4, pid, &s, WALL, 0);
    return r == pid ? s : (int)r;
}

/* The same without sleeping: NOTHING when there is nothing to report. */
static __attribute__((unused)) int poll_task(i64 pid) {
    int s = 0;
    i64 r = syscall4(SYS_wait4, pid, &s, WALL | WNOHANG, 0);
    return r == pid ? s : r == 0 ? NOTHING : (int)r;
}

static __attribute__((unused)) void expect(const char *name, i64 got, i64 want) { check(name, got == want, got); }

/* A word of the tracee's memory, or the negative errno ORed in high. */
static __attribute__((unused)) i64 peek(i64 pid, const volatile void *addr) {
    u64 w = 0;
    i64 r = ptrace(PTRACE_PEEKDATA, pid, (u64)addr, (u64)&w);
    return r < 0 ? r : (i64)w;
}

/* A child that asks to be traced and stops itself with SIGSTOP; returns
 * its pid once that stop was reported, or a negative value. The child
 * goes on with fn() and exits with its result. */
static __attribute__((unused)) i64 traced_child(int (*fn)(void)) {
    i64 c = fork();
    if (c == 0) {
        if (ptrace(PTRACE_TRACEME, 0, 0, 0) != 0) exit_group(100);
        raise_sig(SIGSTOP);
        exit_group(fn());
    }
    return wait_task(c) == STOPPED(SIGSTOP) ? c : -1;
}

static int body(void);

/*
 * The checks run in a process made with CLONE_UNTRACED: a tracer that
 * follows this program's forks (the diff runner's reference) does not
 * follow it, so it and its children can be traced by the test itself.
 */
static __attribute__((unused)) int run_untraced(void) {
    i64 c = syscall6(SYS_clone, CLONE_UNTRACED | SIGCHLD, 0, 0, 0, 0, 0);
    if (c < 0) {
        check("clone_untraced", 0, c);
        return finish();
    }
    if (c == 0) exit_group(body());
    int s = wait_status(c);
    return s != -1 && W_EXITED(s) ? W_EXITCODE(s) : 1;
}
extern char step_0[], step_1[], step_2[], step_3[], bp_site[], bp_next[];
static volatile u64 out;

static __attribute__((noinline)) int tracee(void) {
    i64 me = getpid();
    u64 v;
    __asm__ volatile("mov $5, %%r12\n"
                     "mov $62, %%eax\n"
                     "mov %1, %%rdi\n"
                     "mov $19, %%esi\n"
                     "syscall\n"
                     ".globl step_0\n step_0: nop\n"
                     ".globl step_1\n step_1: add $10, %%r12\n"
                     ".globl step_2\n step_2: add $100, %%r12\n"
                     ".globl step_3\n step_3: nop\n"
                     ".globl bp_site\n bp_site: add $1000, %%r12\n"
                     ".globl bp_next\n bp_next: mov %%r12, %0\n"
                     : "=r"(v)
                     : "r"(me)
                     : "rax", "rdi", "rsi", "rcx", "r11", "r12", "memory");
    out = v;
    raise_sig(SIGSTOP);
    return 0;
}

static int body(void) {
    struct k_regs r;
    struct k_siginfo k;
    i64 c = traced_child(tracee);
    check("traced_child", c > 0, c);
    ptrace(PTRACE_CONT, c, 0, 0);
    expect("stopped_in_asm", wait_task(c), STOPPED(SIGSTOP));
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    check("at_step_0", r.rip == (u64)step_0, (i64)r.rip);

    expect("step_1", ptrace(PTRACE_SINGLESTEP, c, 0, 0), 0);
    expect("step_1_stop", wait_task(c), STOPPED(SIGTRAP));
    memset(&k, 0, sizeof k);
    ptrace(PTRACE_GETSIGINFO, c, 0, (u64)&k);
    expect("step_si_signo", k.si_signo, SIGTRAP);
    expect("step_si_code", k.si_code, TRAP_TRACE);
    check("step_si_addr", k.u.addr == (u64)step_1, (i64)k.u.addr);
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    check("at_step_1", r.rip == (u64)step_1, (i64)r.rip);
    expect("r12_before", (i64)r.r12, 5);
    ptrace(PTRACE_SINGLESTEP, c, 0, 0);
    expect("step_2_stop", wait_task(c), STOPPED(SIGTRAP));
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    check("at_step_2", r.rip == (u64)step_2, (i64)r.rip);
    expect("r12_add_10", (i64)r.r12, 15);
    ptrace(PTRACE_SINGLESTEP, c, 0, 0);
    expect("step_3_stop", wait_task(c), STOPPED(SIGTRAP));
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    check("at_step_3", r.rip == (u64)step_3, (i64)r.rip);
    expect("r12_add_100", (i64)r.r12, 115);

    i64 orig = peek(c, bp_site);
    expect("poke_int3", ptrace(PTRACE_POKETEXT, c, (u64)bp_site, ((u64)orig & ~0xffUL) | 0xcc), 0);
    expect("peek_int3", peek(c, bp_site) & 0xff, 0xcc);
    expect("cont_to_bp", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("bp_stop", wait_task(c), STOPPED(SIGTRAP));
    memset(&k, 0, sizeof k);
    ptrace(PTRACE_GETSIGINFO, c, 0, (u64)&k);
    expect("bp_si_code", k.si_code, SI_KERNEL);
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    check("rip_after_int3", r.rip == (u64)bp_site + 1, (i64)r.rip);
    expect("r12_at_bp", (i64)r.r12, 115);
    expect("restore_code", ptrace(PTRACE_POKETEXT, c, (u64)bp_site, (u64)orig), 0);
    expect("rip_back", ptrace(PTRACE_POKEUSR, c, USER_REG(rip), (u64)bp_site), 0);
    expect("cont_discard_trap", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("final_stop", wait_task(c), STOPPED(SIGSTOP));
    expect("result", peek(c, &out), 1115);
    ptrace(PTRACE_CONT, c, 0, 0);
    expect("exit", wait_task(c), EXITED(0));
    return finish();
}

int test_main(void) { return run_untraced(); }
