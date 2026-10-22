/* Invalid arguments on a stopped tracee: EIO for an unknown request, a
 * signal number above 64 (or negative) for PTRACE_CONT or PTRACE_DETACH,
 * a PTRACE_PEEKUSR/POKEUSR offset that is not 8-aligned or outside
 * struct user (and for POKEUSR outside the registers and debug
 * registers), an invalid segment selector or non-canonical base in
 * PTRACE_POKEUSR/PTRACE_SETREGS, PTRACE_INTERRUPT and PTRACE_LISTEN on a
 * tracee not attached with PTRACE_SEIZE, PTRACE_SEIZE with a non-zero
 * addr or an unknown option bit. EINVAL for an unknown option bit in
 * PTRACE_SETOPTIONS, a sigset size other than 8, an unknown or
 * misaligned regset. EFAULT when the data pointer is bad. A failed
 * request leaves the tracee stopped and unchanged.
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
static volatile i64 word = 42;
static int tracee(void) { return word == 42 ? 5 : 6; }

static int body(void) {
    i64 c = traced_child(tracee);
    check("traced_child", c > 0, c);
    u64 w = 0;
    struct k_regs r;
    expect("unknown_request", ptrace(0x9999, c, 0, 0), -EIO);
    expect("cont_sig_65", ptrace(PTRACE_CONT, c, 0, 65), -EIO);
    expect("cont_sig_negative", ptrace(PTRACE_CONT, c, 0, (u64)-1), -EIO);
    expect("syscall_sig_65", ptrace(PTRACE_SYSCALL, c, 0, 65), -EIO);
    expect("detach_sig_99", ptrace(PTRACE_DETACH, c, 0, 99), -EIO);
    expect("peekusr_unaligned", ptrace(PTRACE_PEEKUSR, c, 3, (u64)&w), -EIO);
    expect("peekusr_past_end", ptrace(PTRACE_PEEKUSR, c, USER_SIZE, (u64)&w), -EIO);
    expect("peekusr_last_ok", ptrace(PTRACE_PEEKUSR, c, USER_SIZE - 8, (u64)&w), 0);
    expect("peekusr_negative", ptrace(PTRACE_PEEKUSR, c, (u64)-8, (u64)&w), -EIO);
    expect("pokeusr_unaligned", ptrace(PTRACE_POKEUSR, c, 4, 0), -EIO);
    expect("pokeusr_after_regs", ptrace(PTRACE_POKEUSR, c, sizeof(struct k_regs), 0), -EIO);
    expect("pokeusr_last_word", ptrace(PTRACE_POKEUSR, c, USER_SIZE - 8, 0), -EIO);
    expect("pokeusr_bad_cs", ptrace(PTRACE_POKEUSR, c, USER_REG(cs), 0x1234), -EIO);
    expect("pokeusr_noncanonical_fs_base", ptrace(PTRACE_POKEUSR, c, USER_REG(fs_base), 0x8000000000000000UL), -EIO);
    expect("getregs", ptrace(PTRACE_GETREGS, c, 0, (u64)&r), 0);
    struct k_regs bad = r;
    bad.cs = 0x1234;
    expect("setregs_bad_cs", ptrace(PTRACE_SETREGS, c, 0, (u64)&bad), -EIO);
    bad = r;
    bad.fs_base = 0x8000000000000000UL;
    expect("setregs_noncanonical_fs_base", ptrace(PTRACE_SETREGS, c, 0, (u64)&bad), -EIO);
    expect("interrupt_not_seized", ptrace(PTRACE_INTERRUPT, c, 0, 0), -EIO);
    expect("listen_not_seized", ptrace(PTRACE_LISTEN, c, 0, 0), -EIO);
    expect("peek_null", ptrace(PTRACE_PEEKDATA, c, 0, (u64)&w), -EIO);
    expect("poke_null", ptrace(PTRACE_POKEDATA, c, 0, 1), -EIO);

    expect("setoptions_unknown_bit", ptrace(PTRACE_SETOPTIONS, c, 0, 1UL << 30), -EINVAL);
    expect("setoptions_bit31", ptrace(PTRACE_SETOPTIONS, c, 0, 1UL << 31), -EINVAL);
    expect("setoptions_bit32", ptrace(PTRACE_SETOPTIONS, c, 0, 1UL << 32), -EINVAL);
    expect("setoptions_valid", ptrace(PTRACE_SETOPTIONS, c, 0, PTRACE_O_TRACESYSGOOD | PTRACE_O_TRACEEXIT), 0);
    expect("setoptions_zero", ptrace(PTRACE_SETOPTIONS, c, 0, 0), 0);
    expect("getsigmask_size4", ptrace(PTRACE_GETSIGMASK, c, 4, (u64)&w), -EINVAL);
    expect("setsigmask_size16", ptrace(PTRACE_SETSIGMASK, c, 16, (u64)&w), -EINVAL);
    u64 buf[64];
    struct k_iovec v = { buf, sizeof buf };
    expect("getregset_unknown", ptrace(PTRACE_GETREGSET, c, 0x999, (u64)&v), -EINVAL);
    v.len = 12;
    expect("getregset_len12", ptrace(PTRACE_GETREGSET, c, NT_PRSTATUS, (u64)&v), -EINVAL);
    v.len = 220;
    expect("getregset_len220", ptrace(PTRACE_GETREGSET, c, NT_PRSTATUS, (u64)&v), -EINVAL);

    expect("getregs_efault", ptrace(PTRACE_GETREGS, c, 0, 8), -EFAULT);
    expect("setregs_efault", ptrace(PTRACE_SETREGS, c, 0, 8), -EFAULT);
    expect("peek_out_efault", ptrace(PTRACE_PEEKDATA, c, (u64)&word, 8), -EFAULT);
    expect("peekusr_out_efault", ptrace(PTRACE_PEEKUSR, c, 0, 8), -EFAULT);
    expect("getsiginfo_efault", ptrace(PTRACE_GETSIGINFO, c, 0, 8), -EFAULT);
    expect("setsiginfo_efault", ptrace(PTRACE_SETSIGINFO, c, 0, 8), -EFAULT);
    expect("geteventmsg_efault", ptrace(PTRACE_GETEVENTMSG, c, 0, 8), -EFAULT);
    expect("getregset_iov_efault", ptrace(PTRACE_GETREGSET, c, NT_PRSTATUS, 8), -EFAULT);
    v.len = sizeof buf;
    v.base = (void *)8;
    expect("getregset_buf_efault", ptrace(PTRACE_GETREGSET, c, NT_PRSTATUS, (u64)&v), -EFAULT);
    expect("getfpregs_efault", ptrace(PTRACE_GETFPREGS, c, 0, 8), -EFAULT);
    expect("syscall_info_efault", ptrace(PTRACE_GET_SYSCALL_INFO, c, sizeof(struct k_syscall_info), 8), -EFAULT);
    expect("getsigmask_efault", ptrace(PTRACE_GETSIGMASK, c, 8, 8), -EFAULT);

    struct k_regs after;
    ptrace(PTRACE_GETREGS, c, 0, (u64)&after);
    check("regs_unchanged", mem_eq(&r, &after, sizeof r), 0);
    expect("word_unchanged", peek(c, &word), 42);
    expect("still_stopped", poll_task(c), NOTHING);
    expect("cont", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("exit", wait_task(c), EXITED(5));

    /* PTRACE_SEIZE arguments. */
    c = fork();
    if (c == 0)
        for (;;) syscall0(SYS_pause);
    expect("seize_addr_nonzero", ptrace(PTRACE_SEIZE, c, 1, 0), -EIO);
    expect("seize_unknown_option", ptrace(PTRACE_SEIZE, c, 0, 1UL << 30), -EIO);
    expect("seize_failed_not_traced", ptrace(PTRACE_INTERRUPT, c, 0, 0), -ESRCH);
    expect("seize_ok", ptrace(PTRACE_SEIZE, c, 0, PTRACE_O_TRACESYSGOOD), 0);
    expect("kill_req", ptrace(PTRACE_KILL, c, 0, 0), 0);
    expect("killed", wait_task(c), SIGKILL);
    return finish();
}

int test_main(void) { return run_untraced(); }
