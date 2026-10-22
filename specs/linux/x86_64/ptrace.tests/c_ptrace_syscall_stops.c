/* PTRACE_SYSCALL resumes the tracee until its next syscall entry or
 * exit. With PTRACE_O_TRACESYSGOOD the stop's signal is SIGTRAP|0x80
 * (siginfo si_code 0x85), without it plain SIGTRAP (si_code 5). At the
 * entry orig_rax is the number and rax is -ENOSYS; at the exit rax is the
 * result. PTRACE_GET_SYSCALL_INFO returns how many bytes it has (80 at an
 * entry, 33 at an exit, 24 elsewhere) and copies at most addr bytes: op
 * 1/2/0, arch AUDIT_ARCH_X86_64, rip and rsp, then the number and six
 * arguments, or the result and whether it is an error. Setting orig_rax to
 * -1 at the entry skips the call (the tracee gets -ENOSYS), another number
 * runs that call instead, and rax set at the exit is what the tracee gets.
 * PTRACE_CONT runs on without syscall stops.
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
static volatile i64 got[5];

static int tracee(void) {
    got[0] = syscall6(SYS_getpid, 11, 22, 33, 44, 55, 66);
    got[1] = syscall0(SYS_getpid);
    got[2] = syscall0(SYS_getpid);
    got[3] = syscall0(SYS_getpid);
    got[4] = syscall0(SYS_getpid);
    raise_sig(SIGSTOP);
    return 9;
}

static int body(void) {
    i64 me = getpid();
    i64 c = traced_child(tracee);
    check("traced_child", c > 0, c);
    struct k_regs r;
    struct k_syscall_info si;
    struct k_siginfo k;
    expect("sysgood", ptrace(PTRACE_SETOPTIONS, c, 0, PTRACE_O_TRACESYSGOOD), 0);

    /* got[0]: entry and exit as they are. */
    expect("syscall_1", ptrace(PTRACE_SYSCALL, c, 0, 0), 0);
    expect("entry_status", wait_task(c), STOPPED(SIGTRAP | 0x80));
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    expect("entry_orig_rax", (i64)r.orig_rax, SYS_getpid);
    expect("entry_rax_enosys", (i64)r.rax, -ENOSYS);
    expect("entry_rdi", (i64)r.rdi, 11);
    memset(&k, 0, sizeof k);
    expect("entry_getsiginfo", ptrace(PTRACE_GETSIGINFO, c, 0, (u64)&k), 0);
    expect("entry_si_signo", k.si_signo, SIGTRAP);
    expect("entry_si_code", k.si_code, SIGTRAP | 0x80);
    memset(&si, 0xee, sizeof si);
    expect("entry_info_size", ptrace(PTRACE_GET_SYSCALL_INFO, c, sizeof si, (u64)&si), 80);
    expect("entry_info_op", si.op, PTRACE_SYSCALL_INFO_ENTRY);
    check("entry_info_arch", si.arch == AUDIT_ARCH_X86_64, si.arch);
    check("entry_info_ip", si.instruction_pointer == r.rip, 0);
    check("entry_info_sp", si.stack_pointer == r.rsp, 0);
    expect("entry_info_nr", (i64)si.u.entry.nr, SYS_getpid);
    check("entry_info_args", si.u.entry.args[0] == 11 && si.u.entry.args[1] == 22 && si.u.entry.args[2] == 33 &&
                                 si.u.entry.args[3] == 44 && si.u.entry.args[4] == 55 && si.u.entry.args[5] == 66,
          0);
    memset(&si, 0xee, sizeof si);
    expect("entry_info_short", ptrace(PTRACE_GET_SYSCALL_INFO, c, 1, (u64)&si), 80);
    check("entry_info_short_copied_1", si.op == PTRACE_SYSCALL_INFO_ENTRY && si.pad[0] == 0xee, si.pad[0]);
    expect("entry_info_zero", ptrace(PTRACE_GET_SYSCALL_INFO, c, 0, 8), 80);

    expect("syscall_2", ptrace(PTRACE_SYSCALL, c, 0, 0), 0);
    expect("exit_status", wait_task(c), STOPPED(SIGTRAP | 0x80));
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    expect("exit_orig_rax", (i64)r.orig_rax, SYS_getpid);
    check("exit_rax_result", r.rax == (u64)c, (i64)r.rax);
    memset(&si, 0xee, sizeof si);
    expect("exit_info_size", ptrace(PTRACE_GET_SYSCALL_INFO, c, sizeof si, (u64)&si), 33);
    expect("exit_info_op", si.op, PTRACE_SYSCALL_INFO_EXIT);
    check("exit_info_rval", si.u.exit.rval == c, si.u.exit.rval);
    expect("exit_info_is_error", si.u.exit.is_error, 0);

    /* got[1]: skipped. */
    ptrace(PTRACE_SYSCALL, c, 0, 0);
    expect("entry_2", wait_task(c), STOPPED(SIGTRAP | 0x80));
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    r.orig_rax = (u64)-1;
    expect("set_orig_rax_minus1", ptrace(PTRACE_SETREGS, c, 0, (u64)&r), 0);
    ptrace(PTRACE_SYSCALL, c, 0, 0);
    expect("exit_2", wait_task(c), STOPPED(SIGTRAP | 0x80));
    memset(&si, 0xee, sizeof si);
    ptrace(PTRACE_GET_SYSCALL_INFO, c, sizeof si, (u64)&si);
    expect("skipped_rval", si.u.exit.rval, -ENOSYS);
    expect("skipped_is_error", si.u.exit.is_error, 1);

    /* got[2]: replaced by getppid. */
    ptrace(PTRACE_SYSCALL, c, 0, 0);
    expect("entry_3", wait_task(c), STOPPED(SIGTRAP | 0x80));
    expect("pokeusr_orig_rax", ptrace(PTRACE_POKEUSR, c, USER_REG(orig_rax), SYS_getppid), 0);
    ptrace(PTRACE_SYSCALL, c, 0, 0);
    expect("exit_3", wait_task(c), STOPPED(SIGTRAP | 0x80));
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    expect("replaced_orig_rax", (i64)r.orig_rax, SYS_getppid);

    /* got[3]: result replaced at the exit. */
    ptrace(PTRACE_SYSCALL, c, 0, 0);
    expect("entry_4", wait_task(c), STOPPED(SIGTRAP | 0x80));
    ptrace(PTRACE_SYSCALL, c, 0, 0);
    expect("exit_4", wait_task(c), STOPPED(SIGTRAP | 0x80));
    expect("pokeusr_rax", ptrace(PTRACE_POKEUSR, c, USER_REG(rax), 4242), 0);

    /* got[4]: no stops with PTRACE_CONT. */
    expect("cont", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("signal_stop", wait_task(c), STOPPED(SIGSTOP));
    memset(&si, 0xee, sizeof si);
    expect("signal_stop_info_size", ptrace(PTRACE_GET_SYSCALL_INFO, c, sizeof si, (u64)&si), 24);
    expect("signal_stop_info_op", si.op, PTRACE_SYSCALL_INFO_NONE);
    check("got0", peek(c, &got[0]) == c, peek(c, &got[0]));
    expect("got1_skipped", peek(c, &got[1]), -ENOSYS);
    check("got2_ppid", peek(c, &got[2]) == me, peek(c, &got[2]));
    expect("got3_replaced", peek(c, &got[3]), 4242);
    check("got4", peek(c, &got[4]) == c, peek(c, &got[4]));

    /* Without TRACESYSGOOD: the next call is exit_group. */
    expect("options_0", ptrace(PTRACE_SETOPTIONS, c, 0, 0), 0);
    ptrace(PTRACE_SYSCALL, c, 0, 0);
    expect("plain_sigtrap", wait_task(c), STOPPED(SIGTRAP));
    memset(&k, 0, sizeof k);
    ptrace(PTRACE_GETSIGINFO, c, 0, (u64)&k);
    expect("plain_si_code", k.si_code, SIGTRAP);
    ptrace(PTRACE_GETREGS, c, 0, (u64)&r);
    expect("exit_group_nr", (i64)r.orig_rax, SYS_exit_group);
    expect("exit_group_arg", (i64)r.rdi, 9);
    ptrace(PTRACE_SYSCALL, c, 0, 0);
    expect("exit", wait_task(c), EXITED(9));
    return finish();
}

int test_main(void) { return run_untraced(); }
