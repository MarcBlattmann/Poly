/* PTRACE_SEIZE attaches without stopping the tracee (and takes the
 * options in data). PTRACE_INTERRUPT stops it: the stop is a
 * PTRACE_EVENT_STOP (status (PTRACE_EVENT_STOP<<16)|(SIGTRAP<<8)|0x7f,
 * siginfo SIGTRAP with si_code (PTRACE_EVENT_STOP<<8)|SIGTRAP);
 * interrupting a tracee that is already in that stop succeeds and makes
 * the next resume end in another such stop at once.
 * A SIGSTOP sent to it stops it first as a signal-delivery-stop
 * (SI_USER, sender's pid); delivering it (PTRACE_CONT with SIGSTOP)
 * gives a group-stop, reported as PTRACE_EVENT_STOP with SIGSTOP (si_code
 * (PTRACE_EVENT_STOP<<8)|SIGSTOP). PTRACE_LISTEN leaves it stopped but no
 * longer in a ptrace stop (requests are ESRCH, nothing to wait for);
 * SIGCONT then ends the group-stop with a PTRACE_EVENT_STOP/SIGTRAP stop,
 * and the next PTRACE_CONT reaches the SIGCONT signal-delivery-stop.
 * In an attach made with PTRACE_ATTACH, a group-stop is reported as a
 * plain SIGSTOP stop, PTRACE_GETSIGINFO fails there with EINVAL and
 * PTRACE_LISTEN with EIO.
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
static volatile i64 ticks;

static void loop(void) {
    for (;;) {
        ticks++;
        sleep_ms(1);
    }
}

static int body(void) {
    i64 me = getpid();
    struct k_siginfo k;
    i64 c = fork();
    if (c == 0) loop();
    expect("seize", ptrace(PTRACE_SEIZE, c, 0, PTRACE_O_TRACESYSGOOD), 0);
    sleep_ms(20);
    expect("not_stopped", poll_task(c), NOTHING);
    expect("peek_running_esrch", peek(c, &ticks), -ESRCH);
    expect("interrupt", ptrace(PTRACE_INTERRUPT, c, 0, 0), 0);
    expect("interrupt_stop", wait_task(c), EVENT_STOPPED(SIGTRAP, PTRACE_EVENT_STOP));
    memset(&k, 0, sizeof k);
    expect("interrupt_siginfo", ptrace(PTRACE_GETSIGINFO, c, 0, (u64)&k), 0);
    expect("interrupt_si_signo", k.si_signo, SIGTRAP);
    expect("interrupt_si_code", k.si_code, (PTRACE_EVENT_STOP << 8) | SIGTRAP);
    expect("interrupt_when_stopped", ptrace(PTRACE_INTERRUPT, c, 0, 0), 0);
    i64 t1 = peek(c, &ticks);
    expect("cont", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("second_interrupt_stops_again", wait_task(c), EVENT_STOPPED(SIGTRAP, PTRACE_EVENT_STOP));
    expect("cont_again", ptrace(PTRACE_CONT, c, 0, 0), 0);
    sleep_ms(20);
    expect("runs_again", poll_task(c), NOTHING);

    expect("send_sigstop", syscall2(SYS_kill, c, SIGSTOP), 0);
    expect("signal_delivery_stop", wait_task(c), STOPPED(SIGSTOP));
    check("ran", peek(c, &ticks) > t1, peek(c, &ticks));
    memset(&k, 0, sizeof k);
    ptrace(PTRACE_GETSIGINFO, c, 0, (u64)&k);
    expect("sigstop_si_code", k.si_code, SI_USER);
    check("sigstop_si_pid", k.u.kill.pid == me, k.u.kill.pid);
    expect("deliver_sigstop", ptrace(PTRACE_CONT, c, 0, SIGSTOP), 0);
    expect("group_stop", wait_task(c), EVENT_STOPPED(SIGSTOP, PTRACE_EVENT_STOP));
    memset(&k, 0, sizeof k);
    expect("group_stop_siginfo", ptrace(PTRACE_GETSIGINFO, c, 0, (u64)&k), 0);
    expect("group_stop_si_signo", k.si_signo, SIGSTOP);
    expect("group_stop_si_code", k.si_code, (PTRACE_EVENT_STOP << 8) | SIGSTOP);
    expect("listen", ptrace(PTRACE_LISTEN, c, 0, 0), 0);
    expect("listen_peek_esrch", peek(c, &ticks), -ESRCH);
    sleep_ms(20);
    expect("listen_nothing", poll_task(c), NOTHING);
    expect("sigcont", syscall2(SYS_kill, c, SIGCONT), 0);
    expect("after_sigcont", wait_task(c), EVENT_STOPPED(SIGTRAP, PTRACE_EVENT_STOP));
    expect("cont_after_sigcont", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("sigcont_delivery_stop", wait_task(c), STOPPED(SIGCONT));
    expect("cont_2", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("kill_req", ptrace(PTRACE_KILL, c, 0, 0), 0);
    expect("killed", wait_task(c), SIGKILL);

    /* The same with PTRACE_ATTACH. */
    c = fork();
    if (c == 0) loop();
    expect("attach", ptrace(PTRACE_ATTACH, c, 0, 0), 0);
    expect("attach_stop", wait_task(c), STOPPED(SIGSTOP));
    expect("attach_interrupt_eio", ptrace(PTRACE_INTERRUPT, c, 0, 0), -EIO);
    expect("attach_deliver_sigstop", ptrace(PTRACE_CONT, c, 0, SIGSTOP), 0);
    expect("attach_group_stop", wait_task(c), STOPPED(SIGSTOP));
    expect("attach_group_stop_siginfo_einval", ptrace(PTRACE_GETSIGINFO, c, 0, (u64)&k), -EINVAL);
    expect("attach_listen_eio", ptrace(PTRACE_LISTEN, c, 0, 0), -EIO);
    expect("attach_kill_req", ptrace(PTRACE_KILL, c, 0, 0), 0);
    expect("attach_killed", wait_task(c), SIGKILL);
    return finish();
}

int test_main(void) { return run_untraced(); }
