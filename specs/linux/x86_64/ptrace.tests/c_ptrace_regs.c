/* Registers of a tracee stopped by a signal it sent itself with the
 * kill syscall: PTRACE_GETREGS (struct user_regs_struct) shows the
 * syscall's number in orig_rax, its result in rax, its arguments in
 * rdi/rsi, rip just after the syscall instruction (also in rcx),
 * user code and stack selectors 0x33 and 0x2b, IF set in eflags, and the
 * caller's other registers. PTRACE_SETREGS and PTRACE_SETREGSET change
 * what the tracee sees when it resumes (here the kill's result and a
 * callee-saved register). PTRACE_GETREGSET NT_PRSTATUS fills at most
 * iov_len bytes (a multiple of 8) of the same 216 bytes and sets iov_len
 * to what it filled; NT_PRFPREG is the 512-byte FXSAVE area that
 * PTRACE_GETFPREGS gives (x87 control word 0x37f and MXCSR 0x1f80 in a
 * fresh process); PTRACE_SETFPREGS changes MXCSR as the tracee sees it
 * and rejects reserved MXCSR bits with EINVAL. PTRACE_ARCH_PRCTL
 * ARCH_GET_FS gives the fs_base of the registers.
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
extern char after_kill[];
static volatile u64 out_r12, out_rax, out_mxcsr;

static __attribute__((noinline)) int tracee(void) {
    i64 me = getpid();
    u64 r12, rax;
    unsigned mxcsr;
    __asm__ volatile("mov $0x1234, %%r12\n"
                     "mov $62, %%eax\n"
                     "mov %2, %%rdi\n"
                     "mov $19, %%esi\n"
                     "syscall\n"
                     ".globl after_kill\n"
                     "after_kill:\n"
                     "inc %%r12\n"
                     "mov %%r12, %0\n"
                     "mov %%rax, %1\n"
                     : "=r"(r12), "=r"(rax)
                     : "r"(me)
                     : "rax", "rdi", "rsi", "rcx", "r11", "r12", "memory");
    __asm__ volatile("stmxcsr %0" : "=m"(mxcsr));
    out_r12 = r12;
    out_rax = rax;
    out_mxcsr = mxcsr;
    raise_sig(SIGSTOP);
    return 0;
}

static int body(void) {
    i64 c = traced_child(tracee);
    check("traced_child", c > 0, c);
    expect("cont_to_kill", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("stopped_by_kill", wait_task(c), STOPPED(SIGSTOP));

    struct k_regs r;
    memset(&r, 0, sizeof r);
    expect("getregs", ptrace(PTRACE_GETREGS, c, 0, (u64)&r), 0);
    expect("orig_rax_kill", (i64)r.orig_rax, 62);
    expect("rax_result", (i64)r.rax, 0);
    check("rdi_pid", r.rdi == (u64)c, (i64)r.rdi);
    expect("rsi_sig", (i64)r.rsi, SIGSTOP);
    expect("r12_kept", (i64)r.r12, 0x1234);
    check("rip_after_syscall", r.rip == (u64)after_kill, (i64)r.rip);
    check("rcx_is_rip", r.rcx == r.rip, (i64)r.rcx);
    expect("cs", (i64)r.cs, 0x33);
    expect("ss", (i64)r.ss, 0x2b);
    expect("eflags_if", (i64)(r.eflags & 0x202), 0x202);

    u64 buf[64];
    struct k_iovec v = { buf, sizeof buf };
    memset(buf, 0xee, sizeof buf);
    expect("getregset_prstatus", ptrace(PTRACE_GETREGSET, c, NT_PRSTATUS, (u64)&v), 0);
    expect("getregset_len", (i64)v.len, 216);
    check("getregset_same_as_getregs", mem_eq(buf, &r, sizeof r), 0);
    check("getregset_rest_untouched", buf[27] == 0xeeeeeeeeeeeeeeeeUL, (i64)buf[27]);
    memset(buf, 0xee, sizeof buf);
    v.len = 16;
    expect("getregset_16", ptrace(PTRACE_GETREGSET, c, NT_PRSTATUS, (u64)&v), 0);
    expect("getregset_16_len", (i64)v.len, 16);
    check("getregset_16_words", buf[0] == r.r15 && buf[1] == r.r14 && buf[2] == 0xeeeeeeeeeeeeeeeeUL, 0);
    v.len = 0;
    expect("getregset_0", ptrace(PTRACE_GETREGSET, c, NT_PRSTATUS, (u64)&v), 0);
    expect("getregset_0_len", (i64)v.len, 0);

    unsigned char fp[512], fpset[sizeof buf];
    memset(fp, 0, sizeof fp);
    expect("getfpregs", ptrace(PTRACE_GETFPREGS, c, 0, (u64)fp), 0);
    expect("fcw", *(unsigned short *)fp, 0x37f);
    expect("mxcsr", *(unsigned *)(fp + 24), 0x1f80);
    v.base = fpset;
    v.len = sizeof fpset;
    expect("getregset_fp", ptrace(PTRACE_GETREGSET, c, NT_PRFPREG, (u64)&v), 0);
    expect("getregset_fp_len", (i64)v.len, 512);
    check("getregset_fp_same", mem_eq(fpset, fp, 512), 0);

    u64 fs = 1;
    expect("arch_prctl_get_fs", ptrace(PTRACE_ARCH_PRCTL, c, (u64)&fs, ARCH_GET_FS), 0);
    check("fs_matches_regs", fs == r.fs_base, (i64)fs);

    *(unsigned *)(fp + 24) = 0xffffffff;
    expect("setfpregs_bad_mxcsr", ptrace(PTRACE_SETFPREGS, c, 0, (u64)fp), -EINVAL);
    *(unsigned *)(fp + 24) = 0x7f80;
    expect("setfpregs", ptrace(PTRACE_SETFPREGS, c, 0, (u64)fp), 0);
    r.r12 = 0x5000;
    expect("setregs", ptrace(PTRACE_SETREGS, c, 0, (u64)&r), 0);
    v.base = buf;
    v.len = sizeof r;
    memcpy(buf, &r, sizeof r);
    ((struct k_regs *)buf)->rax = 77;
    expect("setregset", ptrace(PTRACE_SETREGSET, c, NT_PRSTATUS, (u64)&v), 0);
    struct k_regs now;
    ptrace(PTRACE_GETREGS, c, 0, (u64)&now);
    expect("set_r12_read_back", (i64)now.r12, 0x5000);
    expect("set_rax_read_back", (i64)now.rax, 77);

    expect("cont", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("stop_with_results", wait_task(c), STOPPED(SIGSTOP));
    expect("tracee_r12", peek(c, &out_r12), 0x5001);
    expect("tracee_kill_result", peek(c, &out_rax), 77);
    expect("tracee_mxcsr", peek(c, &out_mxcsr), 0x7f80);
    expect("cont_to_exit", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("exit", wait_task(c), EXITED(0));
    return finish();
}

int test_main(void) { return run_untraced(); }
