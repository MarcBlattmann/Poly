#include "poly_mtest.h"
/* sigaction, BSD 46, the siginfo a SA_SIGINFO handler receives (104 bytes: si_signo 0, si_errno 4, si_code 8,
 * si_pid 12, si_uid 16, si_status 20, si_addr 24, si_value 32). A signal sent with kill by another
 * process: si_signo the number, si_pid the sender's pid (si_code varies between runs: 0 or 1), si_uid the sender's user. SIGCHLD
 * for a child that exits with 5: si_code CLD_EXITED (1), si_pid the child, si_status 5; for a
 * child killed by SIGKILL: CLD_KILLED (2), si_status 9. Faults, each in a forked child whose handler reports in its exit
 * status: unmapped address 8: SIGSEGV with si_code SEGV_ACCERR (2) and si_addr 8; a write to a
 * PROT_NONE page or to a read-only page: SIGBUS (10) with si_code BUS_ADRALN (1) and si_addr the
 * address; a jump to an odd address: SIGBUS 1; an undefined instruction: SIGILL (4) with ILL_ILLTRP (2); a
 * breakpoint instruction: SIGTRAP (5) with si_code 0. */
#define SIGBUS 10
struct si { i32 signo, err, code, pid; u32 uid; i32 status; void *addr; i64 val; i64 band; i64 pad[7]; };
static volatile i64 r_signo, r_code, r_pid, r_uid, r_status, r_runs, r_errno;
static void h(int sig, void *info, void *uc) {
    (void)sig; (void)uc;
    struct si *s = info;
    r_signo = s->signo; r_code = s->code; r_pid = s->pid; r_uid = s->uid; r_status = s->status; r_errno = s->err;
    r_runs++;
}
static char *volatile expect_addr;
static void spin_until_pid(i64 pid) { for (long i = 0; i < 400000000L && r_pid != pid; i++) {} }
static void hf(int sig, void *info, void *uc) {
    (void)uc;
    struct si *s = info;
    int ok = (s->signo == sig) && (s->addr == expect_addr || expect_addr == (char *)-1);
    poly_exit(sig | (s->code << 5) | (ok ? 128 : 0));
}
static int fault_child(int kind) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        poly_sigaction(SIGSEGV, (void *)hf, 0, SA_SIGINFO, 0);
        poly_sigaction(SIGBUS, (void *)hf, 0, SA_SIGINFO, 0);
        poly_sigaction(4, (void *)hf, 0, SA_SIGINFO, 0);
        poly_sigaction(5, (void *)hf, 0, SA_SIGINFO, 0);
        if (kind == 0) { expect_addr = (char *)8; *(volatile int *)8 = 1; }
        if (kind == 1) {
            char *p = (char *)bsd6(SYS_mmap, 0, 16384, PROT_NONE, MAP_ANON | MAP_PRIVATE, -1, 0).ret;
            expect_addr = p;
            *(volatile char *)p = 1;
        }
        if (kind == 2) {
            char *p = (char *)bsd6(SYS_mmap, 0, 16384, PROT_READ, MAP_ANON | MAP_PRIVATE, -1, 0).ret;
            expect_addr = p + 16;
            *(volatile char *)(p + 16) = 1;
        }
        if (kind == 3) {
            void *p = (void *)bsd6(SYS_mmap, 0, 16384, 3, MAP_ANON | MAP_PRIVATE, -1, 0).ret;
            expect_addr = (char *)-1;
            ((void (*)(void))((char *)p + 1))();
        }
        if (kind == 4) {
            expect_addr = (char *)-1;
#if defined(POLY_ARM64)
            __asm__ volatile(".inst 0x00000000");
#else
            __asm__ volatile("ud2");
#endif
        }
        if (kind == 5) {
            expect_addr = (char *)-1;
#if defined(POLY_ARM64)
            __asm__ volatile("brk #0");
#else
            __asm__ volatile("int3");
#endif
        }
        poly_exit(1);
    }
    return poly_wait(f.ret);
}
int test_main(void) {
    poly_sigaction(SIGUSR1, (void *)h, 0, SA_SIGINFO, 0);
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        bsd2(SYS_kill, bsd0(SYS_getppid).ret, SIGUSR1);
        poly_exit(0);
    }
    poly_wait(f.ret);
    check("a signal sent by a child: the handler ran once", r_runs == 1, r_runs);
    check("si_signo SIGUSR1, si_errno 0", r_signo == SIGUSR1 && r_errno == 0, r_signo);
    check("si_pid is the sender, si_uid the user", r_pid == f.ret && r_uid == bsd0(SYS_getuid).ret, r_pid);
    poly_sigaction(SIGCHLD, (void *)h, 0, SA_SIGINFO, 0);
    r_runs = 0;
    f = bsd0(SYS_fork);
    if (f.ret2 == 1) poly_exit(5);
    spin_until_pid(f.ret);
    check("SIGCHLD for an exit: si_code CLD_EXITED, si_pid the child, si_status 5", r_runs >= 1 && r_signo == SIGCHLD && r_code == 1 && r_pid == f.ret && r_status == 5, r_code);
    poly_sigaction(SIGCHLD, SIG_DFL, 0, 0, 0);
    bsd4(SYS_wait4, f.ret, 0, 0, 0);
    poly_sigaction(SIGCHLD, (void *)h, 0, SA_SIGINFO, 0);
    r_runs = 0;
    f = bsd0(SYS_fork);
    if (f.ret2 == 1) { for (;;) bsd0(SYS_getpid); }
    for (volatile int i = 0; i < 1000000; i++) {}
    bsd2(SYS_kill, f.ret, SIGKILL);
    spin_until_pid(f.ret);
    check("SIGCHLD for a kill: CLD_KILLED, si_status 9", r_runs >= 1 && r_code == 2 && r_pid == f.ret && r_status == 9, r_code);
    poly_sigaction(SIGCHLD, SIG_DFL, 0, 0, 0);
    bsd4(SYS_wait4, f.ret, 0, 0, 0);
    int st = fault_child(0);
    check("unmapped address 8: SIGSEGV, SEGV_ACCERR, si_addr 8", W_EXITED(st) && W_EXITSTATUS(st) == (SIGSEGV | (2 << 5) | 128), W_EXITSTATUS(st));
    st = fault_child(1);
    check("write to a PROT_NONE page: SIGBUS, code 1, si_addr the address", W_EXITED(st) && W_EXITSTATUS(st) == (SIGBUS | (1 << 5) | 128), W_EXITSTATUS(st));
    st = fault_child(2);
    check("write to a read-only page: SIGBUS, code 1, si_addr the address", W_EXITED(st) && W_EXITSTATUS(st) == (SIGBUS | (1 << 5) | 128), W_EXITSTATUS(st));
    st = fault_child(4);
    check("an undefined instruction: SIGILL, code ILL_ILLTRP", W_EXITED(st) && (W_EXITSTATUS(st) & 127) == (4 | (2 << 5)), W_EXITSTATUS(st));
    st = fault_child(5);
    check("a breakpoint instruction: SIGTRAP, code 0", W_EXITED(st) && (W_EXITSTATUS(st) & 127) == 5, W_EXITSTATUS(st));
#if defined(POLY_ARM64)
    st = fault_child(3);
    check("a jump to an odd address: SIGBUS, code 1", W_EXITED(st) && (W_EXITSTATUS(st) & 127) == (SIGBUS | (1 << 5)), W_EXITSTATUS(st));
#endif
    return finish();
}
