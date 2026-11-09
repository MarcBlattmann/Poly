/*
 * Helpers for the seccomp tests of the package-format batch (pk_seccomp_*):
 * classic BPF instruction macros, struct sock_filter / sock_fprog, and
 * pk_try(), which installs a program in a forked child, makes one system
 * call there and reports what happened. Include after a_sys.h.
 *
 * Constants and layouts are ABI facts from the public UAPI headers:
 * linux/bpf_common.h, linux/filter.h, linux/seccomp.h, linux/prctl.h,
 * linux/audit.h. Written for Poly; no code is copied from anywhere.
 */
#ifndef P_SECCOMP_H
#define P_SECCOMP_H

#include "a_sys.h"

#define SYS_rt_sigreturn 15
#define SYS_ioctl 16
#define SYS_prctl 157
#define SYS_seccomp 317

/* linux/bpf_common.h */
#define BPF_LD 0x00
#define BPF_LDX 0x01
#define BPF_ST 0x02
#define BPF_STX 0x03
#define BPF_ALU 0x04
#define BPF_JMP 0x05
#define BPF_RET 0x06
#define BPF_MISC 0x07
#define BPF_W 0x00
#define BPF_H 0x08
#define BPF_B 0x10
#define BPF_IMM 0x00
#define BPF_ABS 0x20
#define BPF_IND 0x40
#define BPF_MEM 0x60
#define BPF_LEN 0x80
#define BPF_MSH 0xa0
#define BPF_ADD 0x00
#define BPF_SUB 0x10
#define BPF_MUL 0x20
#define BPF_DIV 0x30
#define BPF_OR 0x40
#define BPF_AND 0x50
#define BPF_LSH 0x60
#define BPF_RSH 0x70
#define BPF_NEG 0x80
#define BPF_MOD 0x90
#define BPF_XOR 0xa0
#define BPF_JA 0x00
#define BPF_JEQ 0x10
#define BPF_JGT 0x20
#define BPF_JGE 0x30
#define BPF_JSET 0x40
#define BPF_K 0x00
#define BPF_X 0x08
#define BPF_A 0x10
#define BPF_TAX 0x00
#define BPF_TXA 0x80

/* linux/seccomp.h */
#define SECCOMP_SET_MODE_STRICT 0
#define SECCOMP_SET_MODE_FILTER 1
#define SECCOMP_RET_KILL_PROCESS 0x80000000U
#define SECCOMP_RET_KILL_THREAD 0x00000000U
#define SECCOMP_RET_TRAP 0x00030000U
#define SECCOMP_RET_ERRNO 0x00050000U
#define SECCOMP_RET_USER_NOTIF 0x7fc00000U
#define SECCOMP_RET_TRACE 0x7ff00000U
#define SECCOMP_RET_LOG 0x7ffc0000U
#define SECCOMP_RET_ALLOW 0x7fff0000U
#define SECCOMP_FILTER_FLAG_TSYNC 1
#define SECCOMP_FILTER_FLAG_LOG 2
#define SECCOMP_FILTER_FLAG_NEW_LISTENER 8
/* linux/audit.h */
#define AUDIT_ARCH_X86_64 0xc000003eU
/* linux/prctl.h */
#define PR_GET_DUMPABLE 3
#define PR_SET_DUMPABLE 4
#define PR_GET_SECCOMP 21
#define PR_SET_SECCOMP 22
#define PR_CAPBSET_READ 23
#define PR_CAPBSET_DROP 24
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39
#define SIGSYS 31

/* offsets in struct seccomp_data (linux/seccomp.h) */
#define SD_NR 0
#define SD_ARCH 4
#define SD_IP_LO 8
#define SD_IP_HI 12
#define SD_ARG_LO(i) (16 + 8 * (i))
#define SD_ARG_HI(i) (20 + 8 * (i))

struct pk_insn {
    u16 code;
    u8 jt, jf;
    u32 k;
};
struct pk_prog {
    u16 len;
    struct pk_insn *filter;
};

#define PK_STMT(c, kk) {(c), 0, 0, (kk)}
#define PK_JUMP(c, kk, t, f) {(c), (t), (f), (kk)}
#define PK_N(a) ((u16)(sizeof(a) / sizeof((a)[0])))
#define PK_LDABS(o) PK_STMT(BPF_LD | BPF_W | BPF_ABS, (o))
#define PK_RET(k) PK_STMT(BPF_RET | BPF_K, (k))
#define PK_ALLOW PK_RET(SECCOMP_RET_ALLOW)
#define PK_ERR(e) PK_RET(SECCOMP_RET_ERRNO | (e))
#define PK_JEQ(k, t, f) PK_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (k), (t), (f))

/* Outcome of pk_try(). */
struct pk_result {
    i64 load;   /* result of installing the program (0 or -errno); 1000000 if the child never reported */
    i64 ret;    /* result of the call made under the program (low 32 bits, sign-extended), valid if ran */
    int ran;    /* the call returned */
    i32 status; /* wait status of the child */
};

#define PK_SEC 0   /* install with seccomp(SECCOMP_SET_MODE_FILTER, 0, &prog) */
#define PK_PRCTL 1 /* install with prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &prog) */
#define PK_RAW 4   /* install the program exactly as given (otherwise it is run only for the probed call, see below) */

/*
 * In a child (made with a_fork_sync, so its end is seen before the parent
 * goes on): set no_new_privs, install `f` (n instructions) the way `via`
 * says, then make the system call nr(a1, a2, a3, a4) and report. Nothing
 * is installed in the calling process.
 *
 * Unless PK_RAW is set the installed program is three instructions followed
 * by `f`: they allow every call but `nr`, and for `nr` fall into `f`. So the
 * calls the child itself needs (exit_group) are never affected, and `f`
 * only has to give a verdict for the probed call. (Jumps inside `f` stay
 * relative to `f`.) With PK_RAW `f` is installed as it is, which the tests
 * of program validation need.
 */
static struct pk_insn pk_buf[4100];
static A_UNUSED void pk_try_via(int via, struct pk_result *r, const struct pk_insn *f, u16 n, i64 nr, i64 a1, i64 a2,
                                i64 a3, i64 a4) {
    volatile i32 *s = a_shared();
    s[0] = 0x7fffffff;
    s[1] = 0;
    s[2] = 0;
    struct pk_prog prog = {n, (struct pk_insn *)f};
    if (!(via & PK_RAW)) {
        pk_buf[0] = (struct pk_insn)PK_LDABS(SD_NR);
        pk_buf[1] = (struct pk_insn)PK_JEQ((u32)nr, 1, 0);
        pk_buf[2] = (struct pk_insn)PK_ALLOW;
        for (u16 i = 0; i < n; i++) pk_buf[3 + i] = f[i];
        prog.len = (u16)(n + 3);
        prog.filter = pk_buf;
    }
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        i64 l = (via & 3) == PK_SEC ? syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog)
                              : syscall3(SYS_prctl, PR_SET_SECCOMP, 2, &prog);
        s[0] = (i32)l;
        if (l == 0) {
            i64 v = syscall6(nr, a1, a2, a3, a4, 0, 0);
            s[1] = (i32)v;
            s[2] = 1;
        }
        syscall1(SYS_exit_group, 0);
    }
    r->status = -1;
    syscall4(SYS_wait4, pid, &r->status, 0, 0);
    r->load = s[0] == 0x7fffffff ? 1000000 : s[0];
    r->ran = s[2];
    r->ret = s[1];
}
static A_UNUSED void pk_try(struct pk_result *r, const struct pk_insn *f, u16 n, i64 nr, i64 a1, i64 a2, i64 a3,
                            i64 a4) {
    pk_try_via(PK_SEC, r, f, n, nr, a1, a2, a3, a4);
}

/* The child was killed by signal `sig`. */
static A_UNUSED int pk_killed_by(const struct pk_result *r, int sig) {
    return A_WIFSIGNALED(r->status) && A_WTERMSIG(r->status) == sig;
}

/*
 * 0 if the program loaded and the call returned -want_errno; otherwise a
 * non-zero code that says what happened (the install error, -1000-signal
 * when the child was killed, or the call's own result).
 */
static A_UNUSED i64 pk_errno_is(const struct pk_result *r, i64 want_errno) {
    if (r->load != 0) return r->load;
    if (!r->ran) return -1000 - (A_WIFSIGNALED(r->status) ? A_WTERMSIG(r->status) : 0);
    return r->ret == -want_errno ? 0 : r->ret;
}

/*
 * Run `body` in a child made with a_fork_sync (the parent is suspended while
 * it runs, so its output comes in order). The checks the body makes count
 * in the test: the child's totals are copied back through shared memory.
 * Returns the child's wait status.
 */
static A_UNUSED i32 pk_in_child(void (*body)(void)) {
    volatile i32 *s = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        body();
        s[8] = passed;
        s[9] = failed;
        syscall1(SYS_exit_group, 0);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    if (A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0) {
        passed = s[8];
        failed = s[9];
    } else {
        check("child_ended_normally", 0, st);
    }
    return st;
}

#endif
