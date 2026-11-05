/* PTRACE_O_TRACEFORK: a tracee that forks stops with PTRACE_EVENT_FORK
 * (WSTOPSIG SIGTRAP, event 1); PTRACE_GETEVENTMSG gives the new child's
 * pid. The new child is traced by the same tracer from its start (a seized
 * parent's child starts in a PTRACE_EVENT_STOP). The tracer sees the
 * grandchild's exit status; its real parent still reaps it afterwards.
 * PTRACE_O_TRACEVFORK and PTRACE_O_TRACEVFORKDONE likewise for vfork
 * (PTRACE_EVENT_VFORK, then PTRACE_EVENT_VFORK_DONE when the child has
 * released the parent, both with the child's pid as message). The
 * SIGCHLD the parent gets from its children also stops it (signal-delivery
 * stops, passed on with PTRACE_CONT).
 * poly-diff: syscalls=off (the traced processes are not followed by the reference's tracer)
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

/* The next stop or exit of `pid`, passing on SIGCHLD stops. */
static int wait_skip_chld(i64 pid) {
    for (;;) {
        int st = wait_any(pid);
        if (!stopped_by(st, SIGCHLD) || W_EVENT(st) != 0) return st;
        ptrace(PTRACE_CONT, pid, 0, SIGCHLD);
    }
}

int test_main(void) {
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    i64 pid = fork_untraced();
    if (pid == 0) {
        char c;
        read(p[0], &c, 1);
        i64 g = fork();
        if (g == 0) exit_group(5);
        int code = wait_exit(g);
        i64 v = syscall5(SYS_clone, 0x4000 | 0x100 | SIGCHLD, 0, 0, 0, 0); /* CLONE_VFORK | CLONE_VM */
        if (v == 0) syscall1(SYS_exit, 6);
        int vcode = wait_exit(v);
        exit_group(code + 10 * (vcode == 6));
    }
    expect("seize", ptrace(PTRACE_SEIZE, pid, 0, PTRACE_O_TRACEFORK | PTRACE_O_TRACEVFORK | PTRACE_O_TRACEVFORKDONE), 0);
    write(p[1], "g", 1);
    int st = wait_any(pid);
    check("fork_event_stop", stopped_by(st, SIGTRAP) && W_EVENT(st) == PTRACE_EVENT_FORK, st);
    u64 g = 0;
    expect("fork_geteventmsg", ptrace(PTRACE_GETEVENTMSG, pid, 0, (u64)&g), 0);
    check("fork_eventmsg_pid", (i64)g > 0 && (i64)g != pid, 0);
    st = wait_any((i64)g);
    check("new_child_starts_stopped", st != -1 && W_STOPPED(st) && W_EVENT(st) == PTRACE_EVENT_STOP, st);
    check("new_child_stop_sig", st != -1 && W_STOPSIG(st) == SIGTRAP, st);
    expect("cont_new_child", ptrace(PTRACE_CONT, (i64)g, 0, 0), 0);
    expect("cont_parent", ptrace(PTRACE_CONT, pid, 0, 0), 0);
    st = wait_any((i64)g);
    check("tracer_sees_child_exit", st != -1 && W_EXITED(st) && W_EXITCODE(st) == 5, st);

    st = wait_skip_chld(pid);
    check("vfork_event_stop", stopped_by(st, SIGTRAP) && W_EVENT(st) == PTRACE_EVENT_VFORK, st);
    u64 v = 0;
    ptrace(PTRACE_GETEVENTMSG, pid, 0, (u64)&v);
    check("vfork_eventmsg_pid", (i64)v > 0 && v != g, 0);
    st = wait_any((i64)v);
    check("vfork_child_starts_stopped", st != -1 && W_STOPPED(st) && W_EVENT(st) == PTRACE_EVENT_STOP, st);
    ptrace(PTRACE_CONT, (i64)v, 0, 0);
    ptrace(PTRACE_CONT, pid, 0, 0);
    st = wait_any((i64)v);
    check("vfork_child_exit", st != -1 && W_EXITED(st) && W_EXITCODE(st) == 6, st);
    st = wait_skip_chld(pid);
    check("vfork_done_stop", stopped_by(st, SIGTRAP) && W_EVENT(st) == 5, st);
    u64 vd = 0;
    ptrace(PTRACE_GETEVENTMSG, pid, 0, (u64)&vd);
    check("vfork_done_eventmsg_pid", vd == v, 0);
    ptrace(PTRACE_CONT, pid, 0, 0);
    st = wait_skip_chld(pid);
    check("parent_reaped_both", st != -1 && W_EXITED(st) && W_EXITCODE(st) == 15, st);
    return finish();
}
