/* Filter results that end the process: SECCOMP_RET_KILL_PROCESS, and
 * SECCOMP_RET_KILL_THREAD in a single-threaded process, end it as if by
 * SIGSYS (wait status: terminated by signal 31) at the matched call;
 * nothing after it runs. SECCOMP_RET_TRAP without a SIGSYS handler does
 * the same (the default action of SIGSYS). A more severe result of an
 * older filter wins over a newer, milder one (KILL_PROCESS over ERRNO).
 * Each case runs in its own child with core dumps off (PR_SET_DUMPABLE
 * 0), so the status has no core bit.
 */
#include "a_sys.h"

#define SYS_prctl 157
#define SYS_seccomp 317
/* linux/prctl.h */
#define PR_SET_DUMPABLE 4
#define PR_SET_NO_NEW_PRIVS 38
/* linux/seccomp.h, linux/filter.h, linux/bpf_common.h */
#define SECCOMP_SET_MODE_FILTER 1
#define SECCOMP_RET_KILL_PROCESS 0x80000000U
#define SECCOMP_RET_KILL_THREAD 0x00000000U
#define SECCOMP_RET_TRAP 0x00030000U
#define SECCOMP_RET_ERRNO 0x00050000U
#define SECCOMP_RET_ALLOW 0x7fff0000U
#define BPF_LD_W_ABS 0x20
#define BPF_JEQ_K 0x15
#define BPF_RET_K 0x06
#define SIGSYS 31

struct sock_filter {
    u16 code;
    u8 jt, jf;
    u32 k;
};
struct sock_fprog {
    u16 len;
    struct sock_filter *filter;
};

static struct sock_filter insns[4];
static struct sock_fprog prog = {4, insns};

static i64 install(u32 nr, u32 action) {
    insns[0] = (struct sock_filter){BPF_LD_W_ABS, 0, 0, 0};
    insns[1] = (struct sock_filter){BPF_JEQ_K, 0, 1, nr};
    insns[2] = (struct sock_filter){BPF_RET_K, 0, 0, action};
    insns[3] = (struct sock_filter){BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW};
    return syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog);
}

/* Child: filter getppid with `first` (and then `second`, if not ALLOW),
 * call it, and exit 3 if still alive. Returns the wait status. */
static i32 run(u32 first, u32 second) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall5(SYS_prctl, PR_SET_DUMPABLE, 0, 0, 0, 0);
        syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        if (install(SYS_getppid, first) != 0) syscall1(SYS_exit_group, 1);
        if (second != SECCOMP_RET_ALLOW && install(SYS_getppid, second) != 0) syscall1(SYS_exit_group, 2);
        syscall0(SYS_getppid);
        put("not reached\n");
        syscall1(SYS_exit_group, 3);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    return st;
}

int test_main(void) {
    a_block_sigchld();
    expect("kill_process_sigsys", run(SECCOMP_RET_KILL_PROCESS, SECCOMP_RET_ALLOW), SIGSYS);
    expect("kill_thread_sigsys", run(SECCOMP_RET_KILL_THREAD, SECCOMP_RET_ALLOW), SIGSYS);
    expect("trap_without_handler_sigsys", run(SECCOMP_RET_TRAP | 5, SECCOMP_RET_ALLOW), SIGSYS);
    expect("older_kill_beats_newer_errno", run(SECCOMP_RET_KILL_PROCESS, SECCOMP_RET_ERRNO | 1), SIGSYS);
    expect("newer_kill_beats_older_errno", run(SECCOMP_RET_ERRNO | 1, SECCOMP_RET_KILL_PROCESS), SIGSYS);
    expect("errno_alone_survives", run(SECCOMP_RET_ERRNO | 1, SECCOMP_RET_ALLOW), 3 << 8);
    return finish();
}
