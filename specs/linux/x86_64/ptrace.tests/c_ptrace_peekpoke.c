/* PTRACE_PEEKDATA/PEEKTEXT read one 8-byte word of the tracee's memory
 * at any byte address (the raw call stores it at *data and returns 0);
 * PTRACE_POKEDATA/POKETEXT write one. Both see the tracee's own copy
 * after fork. They work on memory the tracee cannot write or even read
 * (read-only data, PROT_READ and PROT_NONE mappings); a word that is
 * not wholly mapped (NULL, an unmapped page, a word running into one, a
 * non-canonical address) fails with EIO; a poke running into an
 * unmapped page has written the bytes before it.
 * PTRACE_PEEKUSR reads a word of struct user: the registers in
 * user_regs_struct order (the same values PTRACE_GETREGS gives), zeros
 * elsewhere apart from the debug registers (DR7 reads 0 in a process
 * that never set it); PTRACE_POKEUSR writes a register.
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
static volatile u64 words[2] = { 0x1122334455667788UL, 0x99aabbccddeeff00UL };
static volatile const u64 constant = 0x0123456789abcdefUL;
static volatile u64 seen[4];
static volatile char *read_only, *no_access;

static int tracee(void) {
    seen[0] = words[0];
    seen[1] = constant;
    seen[2] = *(volatile u64 *)read_only;
    raise_sig(SIGSTOP);
    return 0;
}

static int body(void) {
    read_only = (char *)syscall6(SYS_mmap, 0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    no_access = (char *)syscall6(SYS_mmap, 0, 4096, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *gone = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, gone + 4096, 4096);
    gone[4095] = 0x5a;
    words[0] = 0x1122334455667788UL; /* the tracee's copy is changed below, ours stays */

    i64 c = traced_child(tracee);
    check("traced_child", c > 0, c);
    u64 w = 0;
    expect("peekdata_ret", ptrace(PTRACE_PEEKDATA, c, (u64)&words[0], (u64)&w), 0);
    check("peekdata_value", w == 0x1122334455667788UL, (i64)w);
    w = 0;
    expect("peektext_ret", ptrace(PTRACE_PEEKTEXT, c, (u64)&words[0], (u64)&w), 0);
    check("peektext_value", w == 0x1122334455667788UL, (i64)w);
    w = 0;
    expect("peek_unaligned_ret", ptrace(PTRACE_PEEKDATA, c, (u64)&words[0] + 1, (u64)&w), 0);
    check("peek_unaligned_value", w == 0x0011223344556677UL, (i64)w);
    check("peek_const", peek(c, &constant) == 0x0123456789abcdefL, peek(c, &constant));
    expect("peek_read_only_map", peek(c, read_only), 0);
    expect("peek_no_access_map", peek(c, no_access), 0);
    expect("peek_last_mapped_byte_word", peek(c, gone + 4088) >> 56, 0x5a);

    expect("poke_data", ptrace(PTRACE_POKEDATA, c, (u64)&words[0], 0xfeedfacecafebeefUL), 0);
    check("poke_data_read_back", peek(c, &words[0]) == (i64)0xfeedfacecafebeefUL, peek(c, &words[0]));
    check("own_copy_unchanged", words[0] == 0x1122334455667788UL, 0);
    expect("poke_unaligned", ptrace(PTRACE_POKEDATA, c, (u64)&words[0] + 4, 0x0000000077777777UL), 0);
    check("poke_unaligned_low", peek(c, &words[0]) == (i64)0x77777777cafebeefUL, peek(c, &words[0]));
    check("poke_unaligned_high", peek(c, &words[1]) == (i64)0x99aabbcc00000000UL, peek(c, &words[1]));
    expect("poke_const", ptrace(PTRACE_POKETEXT, c, (u64)&constant, 0x4242), 0);
    expect("poke_read_only_map", ptrace(PTRACE_POKEDATA, c, (u64)read_only, 0x77), 0);
    expect("poke_no_access_map", ptrace(PTRACE_POKEDATA, c, (u64)no_access, 0x99), 0);
    expect("peek_no_access_after", peek(c, no_access), 0x99);

    expect("peek_null_eio", peek(c, 0), -EIO);
    expect("peek_unmapped_eio", peek(c, gone + 4096), -EIO);
    expect("peek_straddling_eio", peek(c, gone + 4092), -EIO);
    expect("peek_noncanonical_eio", peek(c, (void *)0x8000000000000000UL), -EIO);
    expect("poke_unmapped_eio", ptrace(PTRACE_POKEDATA, c, (u64)(gone + 4096), 1), -EIO);
    expect("poke_straddling_eio", ptrace(PTRACE_POKEDATA, c, (u64)(gone + 4092), 0x1111111111111111UL), -EIO);
    expect("straddling_poke_wrote_mapped_part", peek(c, gone + 4088) >> 56, 0x11);

    struct k_regs r;
    expect("getregs", ptrace(PTRACE_GETREGS, c, 0, (u64)&r), 0);
    int same = 1;
    for (u64 i = 0; i < sizeof r / 8; i++) {
        w = 0x5555;
        if (ptrace(PTRACE_PEEKUSR, c, i * 8, (u64)&w) != 0 || w != ((u64 *)&r)[i]) same = 0;
    }
    check("peekusr_regs_match_getregs", same, 0);
    int zero = 1;
    for (u64 off = sizeof r; off < USER_DEBUGREG(0); off += 8) {
        w = 0x5555;
        if (ptrace(PTRACE_PEEKUSR, c, off, (u64)&w) != 0 || w != 0) zero = 0;
    }
    check("peekusr_rest_zero", zero, 0);
    w = 0x5555;
    expect("peekusr_dr7", ptrace(PTRACE_PEEKUSR, c, USER_DEBUGREG(7), (u64)&w), 0);
    expect("dr7_zero", (i64)w, 0);
    u64 rbx = r.rbx;
    expect("pokeusr_rbx", ptrace(PTRACE_POKEUSR, c, USER_REG(rbx), 0xabcdef), 0);
    expect("getregs_again", ptrace(PTRACE_GETREGS, c, 0, (u64)&r), 0);
    expect("pokeusr_rbx_seen", (i64)r.rbx, 0xabcdef);
    expect("pokeusr_rbx_back", ptrace(PTRACE_POKEUSR, c, USER_REG(rbx), rbx), 0);

    expect("cont", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("stop_after_reading", wait_task(c), STOPPED(SIGSTOP));
    check("tracee_sees_poked_word", peek(c, &seen[0]) == (i64)0x77777777cafebeefUL, peek(c, &seen[0]));
    expect("tracee_sees_poked_const", peek(c, &seen[1]), 0x4242);
    expect("tracee_sees_poked_read_only", peek(c, &seen[2]), 0x77);
    expect("cont_again", ptrace(PTRACE_CONT, c, 0, 0), 0);
    expect("exit", wait_task(c), EXITED(0));
    return finish();
}

int test_main(void) { return run_untraced(); }
