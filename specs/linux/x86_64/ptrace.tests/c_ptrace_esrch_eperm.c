/* Who may be traced, and requests on processes that are not stopped
 * tracees of the caller. Every request but PTRACE_TRACEME, PTRACE_ATTACH,
 * PTRACE_SEIZE and PTRACE_KILL fails with ESRCH unless the pid is a
 * tracee of the caller that is in a ptrace stop: a pid that does not
 * exist, 0, -1, the caller itself, pid 1, an own child that is not
 * traced, a running tracee, a tracee that has exited (zombie) or been
 * reaped. The pid is checked before the request number (an unknown
 * request on such a pid is ESRCH too). PTRACE_ATTACH and PTRACE_SEIZE of
 * the caller itself, of a process already traced, or of a zombie fail
 * with EPERM, of a pid that does not exist with ESRCH. PTRACE_TRACEME
 * by a process that is already traced fails with EPERM. PTRACE_KILL of
 * a running tracee succeeds and kills it.
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
#define NO_PID 0x3ffffff0 /* above any pid_max */
static volatile i64 go, traceme_result = 1;

static int body(void) {
    i64 me = getpid();
    u64 w = 0;
    expect("peek_no_pid", ptrace(PTRACE_PEEKDATA, NO_PID, (u64)&go, (u64)&w), -ESRCH);
    expect("peek_pid0", ptrace(PTRACE_PEEKDATA, 0, (u64)&go, (u64)&w), -ESRCH);
    expect("peek_pid_minus1", ptrace(PTRACE_PEEKDATA, -1, (u64)&go, (u64)&w), -ESRCH);
    expect("peek_self", ptrace(PTRACE_PEEKDATA, me, (u64)&go, (u64)&w), -ESRCH);
    expect("peek_init", ptrace(PTRACE_PEEKDATA, 1, (u64)&go, (u64)&w), -ESRCH);
    expect("unknown_request_self", ptrace(0x9999, me, 0, 0), -ESRCH);
    expect("unknown_request_no_pid", ptrace(0x9999, NO_PID, 0, 0), -ESRCH);
    expect("attach_self", ptrace(PTRACE_ATTACH, me, 0, 0), -EPERM);
    expect("seize_self", ptrace(PTRACE_SEIZE, me, 0, 0), -EPERM);
    expect("attach_no_pid", ptrace(PTRACE_ATTACH, NO_PID, 0, 0), -ESRCH);
    expect("attach_pid0", ptrace(PTRACE_ATTACH, 0, 0, 0), -ESRCH);
    expect("seize_no_pid", ptrace(PTRACE_SEIZE, NO_PID, 0, 0), -ESRCH);

    i64 c = fork();
    if (c == 0) {
        for (;;) {
            if (go) {
                traceme_result = ptrace(PTRACE_TRACEME, 0, 0, 0);
                raise_sig(SIGSTOP);
                go = 0;
            }
            sleep_ms(1);
        }
    }
    expect("peek_untraced_child", ptrace(PTRACE_PEEKDATA, c, (u64)&go, (u64)&w), -ESRCH);
    expect("cont_untraced_child", ptrace(PTRACE_CONT, c, 0, 0), -ESRCH);
    expect("detach_untraced_child", ptrace(PTRACE_DETACH, c, 0, 0), -ESRCH);
    expect("getregs_untraced_child", ptrace(PTRACE_GETREGS, c, 0, (u64)&w), -ESRCH);
    expect("kill_req_untraced_child", ptrace(PTRACE_KILL, c, 0, 0), -ESRCH);
    expect("attach_child", ptrace(PTRACE_ATTACH, c, 0, 0), 0);
    expect("attach_again_eperm", ptrace(PTRACE_ATTACH, c, 0, 0), -EPERM);
    expect("seize_traced_eperm", ptrace(PTRACE_SEIZE, c, 0, 0), -EPERM);
    expect("attach_stop", wait_task(c), STOPPED(SIGSTOP));
    expect("poke_go", ptrace(PTRACE_POKEDATA, c, (u64)&go, 1), 0);
    expect("cont", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("tracee_stops_itself", wait_task(c), STOPPED(SIGSTOP));
    expect("traceme_when_traced_eperm", peek(c, &traceme_result), -EPERM);
    expect("cont_running", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("peek_running_esrch", ptrace(PTRACE_PEEKDATA, c, (u64)&go, (u64)&w), -ESRCH);
    expect("getregs_running_esrch", ptrace(PTRACE_GETREGS, c, 0, (u64)&w), -ESRCH);
    expect("cont_running_esrch", ptrace(PTRACE_CONT, c, 0, 0), -ESRCH);
    expect("detach_running_esrch", ptrace(PTRACE_DETACH, c, 0, 0), -ESRCH);
    expect("unknown_running_esrch", ptrace(0x9999, c, 0, 0), -ESRCH);
    expect("kill_req_running_ok", ptrace(PTRACE_KILL, c, 0, 0), 0);
    expect("killed", wait_task(c), SIGKILL);
    expect("peek_reaped", ptrace(PTRACE_PEEKDATA, c, (u64)&go, (u64)&w), -ESRCH);

    /* A tracee that has exited but is not reaped yet. */
    c = fork();
    if (c == 0) {
        ptrace(PTRACE_TRACEME, 0, 0, 0);
        exit_group(3);
    }
    struct k_siginfo si;
    syscall5(SYS_waitid, P_PID, c, &si, WEXITED | WNOWAIT, 0);
    expect("peek_zombie_tracee", ptrace(PTRACE_PEEKDATA, c, (u64)&go, (u64)&w), -ESRCH);
    expect("cont_zombie_tracee", ptrace(PTRACE_CONT, c, 0, 0), -ESRCH);
    expect("zombie_exit", wait_task(c), EXITED(3));
    c = fork();
    if (c == 0) exit_group(4);
    syscall5(SYS_waitid, P_PID, c, &si, WEXITED | WNOWAIT, 0);
    expect("attach_zombie_eperm", ptrace(PTRACE_ATTACH, c, 0, 0), -EPERM);
    expect("seize_zombie_eperm", ptrace(PTRACE_SEIZE, c, 0, 0), -EPERM);
    expect("zombie_reaped", wait_task(c), EXITED(4));
    return finish();
}

int test_main(void) { return run_untraced(); }
