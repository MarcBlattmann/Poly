/* Who may attach. A process of uid 1000 may attach to its own dumpable
 * child; not to its own child that made itself non-dumpable
 * (prctl(PR_SET_DUMPABLE, 0)): EPERM; not to a process of another uid
 * (root's): EPERM. Root (CAP_SYS_PTRACE) attaches to a non-dumpable
 * process of uid 1000 that is not its child. Changing uids makes a
 * process non-dumpable (fs.suid_dumpable 0), so the uid-1000 process sets
 * itself dumpable again before it forks.
 * poly-diff: needs=root (set*id to other users)
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

#define PR_SET_DUMPABLE 4
#define PR_GET_DUMPABLE 3

static int ready[2], go[2];

/* A child that becomes `uid` (if not 0), sets its dumpable flag, reports
 * ready and waits for the go pipe. */
static i64 target(i64 uid, int dumpable) {
    i64 pid = fork_untraced();
    if (pid == 0) {
        if (uid) {
            syscall3(SYS_setresgid, uid, uid, uid);
            syscall3(SYS_setresuid, uid, uid, uid);
        }
        syscall2(SYS_prctl, PR_SET_DUMPABLE, dumpable);
        write(ready[1], "r", 1);
        char c;
        read(go[0], &c, 1);
        exit_group(0);
    }
    return pid;
}

static void wait_ready(int n) {
    char rb[8];
    for (int got = 0; got < n;) {
        i64 r = read(ready[0], rb, n - got);
        if (r <= 0) break;
        got += (int)r;
    }
}

static int code_of(i64 r) { return r == 0 ? 0 : r == -EPERM ? 1 : 2; }

int test_main(void) {
    if (syscall0(SYS_geteuid) != 0) {
        check("needs_root", 0, syscall0(SYS_geteuid));
        return finish();
    }
    syscall2(SYS_pipe2, ready, 0);
    syscall2(SYS_pipe2, go, 0);
    i64 t_root = target(0, 1);
    i64 t_nodump = target(1000, 0);
    wait_ready(2);

    i64 u = fork_untraced();
    if (u == 0) {
        syscall3(SYS_setresgid, 1000, 1000, 1000);
        syscall3(SYS_setresuid, 1000, 1000, 1000);
        int was = (int)syscall2(SYS_prctl, PR_GET_DUMPABLE, 0);
        syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
        i64 d1 = target(0, 1);
        i64 d0 = target(0, 0);
        wait_ready(2);
        int code = code_of(ptrace(PTRACE_SEIZE, d1, 0, 0));
        code |= code_of(ptrace(PTRACE_SEIZE, d0, 0, 0)) << 2;
        code |= code_of(ptrace(PTRACE_SEIZE, t_root, 0, 0)) << 4;
        code |= (was == 0 ? 0 : 1) << 6;
        exit_group(code);
    }
    int st = wait_any(u);
    int code = st != -1 && W_EXITED(st) ? W_EXITCODE(st) : -1;
    check("setresuid_made_nondumpable", code >= 0 && ((code >> 6) & 1) == 0, code);
    check("user_attach_own_child_ok", code >= 0 && (code & 3) == 0, code);
    check("user_attach_nondumpable_child_eperm", code >= 0 && ((code >> 2) & 3) == 1, code);
    check("user_attach_root_eperm", code >= 0 && ((code >> 4) & 3) == 1, code);

    expect("root_attach_nondumpable", ptrace(PTRACE_SEIZE, t_nodump, 0, 0), 0);
    expect("root_interrupt", ptrace(PTRACE_INTERRUPT, t_nodump, 0, 0), 0);
    st = wait_any(t_nodump);
    check("root_stopped_it", stopped_by(st, SIGTRAP), st);
    expect("root_detach", ptrace(PTRACE_DETACH, t_nodump, 0, 0), 0);
    write(go[1], "gggg", 4);
    int ok = 1;
    ok &= wait_exit(t_root) == 0;
    ok &= wait_exit(t_nodump) == 0;
    check("targets_exit", ok, 0);
    return finish();
}
