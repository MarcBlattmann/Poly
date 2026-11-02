/* PTRACE_SEIZE: addr must be 0 (else EIO) and data holds options (an
 * unknown bit is EIO too, while PTRACE_SETOPTIONS says EINVAL); seizing a tracee twice is EPERM. PTRACE_INTERRUPT
 * stops a tracee blocked in read(2); PTRACE_CONT resumes it and the read
 * goes on without EINTR. A signal to a seized tracee gives a
 * signal-delivery stop; PTRACE_CONT with 0 suppresses it.
 * PTRACE_O_TRACEEXIT stops the tracee at exit (PTRACE_EVENT_EXIT) with the
 * wait status as event message. PTRACE_INTERRUPT on a tracee attached with
 * PTRACE_ATTACH is EIO.
 * poly-diff: syscalls=off (the traced children are not followed by the reference's tracer)
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

int test_main(void) {
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    i64 pid = fork_untraced();
    if (pid == 0) {
        char c = 0;
        i64 n = read(p[0], &c, 1);
        exit_group(n == 1 && c == 'g' ? 42 : 1);
    }
    expect("seize_addr_eio", ptrace(PTRACE_SEIZE, pid, 1, 0), -EIO);
    expect("seize_bad_option_eio", ptrace(PTRACE_SEIZE, pid, 0, 0x40000000), -EIO);
    expect("seize", ptrace(PTRACE_SEIZE, pid, 0, PTRACE_O_TRACEEXIT), 0);
    expect("seize_again_eperm", ptrace(PTRACE_SEIZE, pid, 0, 0), -EPERM);
    expect("attach_again_eperm", ptrace(PTRACE_ATTACH, pid, 0, 0), -EPERM);
    sleep_ms(20);
    expect("interrupt", ptrace(PTRACE_INTERRUPT, pid, 0, 0), 0);
    int st = wait_any(pid);
    check("interrupt_stop", stopped_by(st, SIGTRAP) && W_EVENT(st) == PTRACE_EVENT_STOP, st);
    struct k_siginfo si;
    memset(&si, 0, sizeof si);
    expect("interrupt_getsiginfo", ptrace(PTRACE_GETSIGINFO, pid, 0, (u64)&si), 0);
    expect("interrupt_siginfo_signo", si.si_signo, SIGTRAP);
    expect("interrupt_siginfo_code", si.si_code, SIGTRAP | (PTRACE_EVENT_STOP << 8));
    expect("cont", ptrace(PTRACE_CONT, pid, 0, 0), 0);

    syscall2(SYS_kill, pid, SIGUSR1);
    st = wait_any(pid);
    check("signal_delivery_stop", stopped_by(st, SIGUSR1) && W_EVENT(st) == 0, st);
    memset(&si, 0, sizeof si);
    ptrace(PTRACE_GETSIGINFO, pid, 0, (u64)&si);
    expect("usr1_siginfo_signo", si.si_signo, SIGUSR1);
    expect("usr1_siginfo_code_si_user", si.si_code, SI_USER);
    check("usr1_siginfo_pid_is_sender", si.u.kill.pid == getpid(), 0);
    expect("cont_suppress", ptrace(PTRACE_CONT, pid, 0, 0), 0);

    write(p[1], "g", 1);
    st = wait_any(pid);
    check("exit_event_stop", stopped_by(st, SIGTRAP) && W_EVENT(st) == PTRACE_EVENT_EXIT, st);
    u64 msg = 0;
    expect("exit_geteventmsg", ptrace(PTRACE_GETEVENTMSG, pid, 0, (u64)&msg), 0);
    expect("exit_eventmsg_status", (i64)msg, 42 << 8);
    expect("cont_to_exit", ptrace(PTRACE_CONT, pid, 0, 0), 0);
    st = wait_any(pid);
    check("child_exit_42", st != -1 && W_EXITED(st) && W_EXITCODE(st) == 42, st);

    /* PTRACE_ATTACH: SIGSTOP stop; PTRACE_INTERRUPT is only for seized tracees. */
    i64 q = fork_untraced();
    if (q == 0) {
        char c;
        read(p[0], &c, 1);
        exit_group(3);
    }
    expect("attach", ptrace(PTRACE_ATTACH, q, 0, 0), 0);
    st = wait_any(q);
    check("attach_sigstop", stopped_by(st, SIGSTOP), st);
    expect("interrupt_attached_eio", ptrace(PTRACE_INTERRUPT, q, 0, 0), -EIO);
    expect("detach", ptrace(PTRACE_DETACH, q, 0, 0), 0);
    write(p[1], "g", 1);
    st = wait_any(q);
    check("attached_child_exit_3", st != -1 && W_EXITED(st) && W_EXITCODE(st) == 3, st);
    return finish();
}
