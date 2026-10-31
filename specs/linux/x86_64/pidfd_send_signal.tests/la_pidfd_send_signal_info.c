/* pidfd_send_signal with a siginfo (as rt_sigqueueinfo): the info's
 * si_signo must equal sig (else EINVAL). To another process, si_code
 * must be negative and not SI_TKILL (-6): si_code SI_QUEUE (-1) is
 * delivered with the given si_value, si_pid and si_uid as given;
 * si_code SI_USER (0), SI_KERNEL (0x80) or SI_TKILL is EPERM. To the
 * caller itself any si_code is allowed and delivered as given.
 * poly-diff: syscalls=off (two processes run at once)
 */
#include "a_sys.h"

#define SYS_pidfd_send_signal 424
#define SYS_pidfd_open 434
#define SYS_rt_sigtimedwait 128
#define SI_QUEUE (-1)
#define SI_TKILL (-6)
#define SI_KERNEL 0x80

/* siginfo_t for SI_QUEUE: signo, errno, code, pad, pid, uid, value. */
struct k_info {
    i32 signo, err, code, pad;
    i32 pid;
    u32 uid;
    u64 value;
    u64 rest[12];
};

static i64 pss(i64 fd, i64 sig, void *info) { return syscall4(SYS_pidfd_send_signal, fd, sig, info, 0); }

static void fill(struct k_info *i, int signo, int code, u64 value) {
    memset(i, 0, sizeof *i);
    i->signo = signo;
    i->code = code;
    i->pid = 4321;
    i->uid = 77;
    i->value = value;
}

int test_main(void) {
    a_block_sigchld();
    u64 block = A_SIGBIT(SIGUSR1) | A_SIGBIT(SIGUSR2);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &block, 0, 8);
    volatile i32 *sh = a_shared();
    struct k_info in;

    i64 pid = syscall0(SYS_fork);
    if (pid == 0) {
        struct k_info got;
        u64 set = A_SIGBIT(SIGUSR1);
        memset(&got, 0, sizeof got);
        i64 s = syscall4(SYS_rt_sigtimedwait, &set, &got, 0, 8);
        sh[1] = (i32)s;
        sh[2] = got.code;
        sh[3] = got.pid;
        sh[4] = (i32)got.uid;
        sh[5] = (i32)got.value;
        syscall1(SYS_exit, 0);
    }
    i64 fd = syscall2(SYS_pidfd_open, pid, 0);
    fill(&in, SIGUSR2, SI_QUEUE, 1);
    expect("signo_mismatch_einval", pss(fd, SIGUSR1, &in), -EINVAL);
    fill(&in, SIGUSR1, 0, 1);
    expect("other_si_user_eperm", pss(fd, SIGUSR1, &in), -EPERM);
    fill(&in, SIGUSR1, SI_KERNEL, 1);
    expect("other_si_kernel_eperm", pss(fd, SIGUSR1, &in), -EPERM);
    fill(&in, SIGUSR1, SI_TKILL, 1);
    expect("other_si_tkill_eperm", pss(fd, SIGUSR1, &in), -EPERM);
    fill(&in, SIGUSR1, SI_QUEUE, 0x5eed);
    expect("other_si_queue_ok", pss(fd, SIGUSR1, &in), 0);
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child_got_sigusr1", sh[1], SIGUSR1);
    expect("child_si_code", sh[2], SI_QUEUE);
    expect("child_si_pid_as_given", sh[3], 4321);
    expect("child_si_uid_as_given", sh[4], 77);
    expect("child_si_value", sh[5], 0x5eed);

    /* To oneself any code goes. */
    i64 self = syscall2(SYS_pidfd_open, syscall0(SYS_getpid), 0);
    fill(&in, SIGUSR2, SI_KERNEL, 9);
    expect("self_si_kernel_ok", pss(self, SIGUSR2, &in), 0);
    struct k_info got;
    u64 set = A_SIGBIT(SIGUSR2);
    memset(&got, 0, sizeof got);
    expect("self_received", syscall4(SYS_rt_sigtimedwait, &set, &got, 0, 8), SIGUSR2);
    expect("self_code_as_given", got.code, SI_KERNEL);
    expect("self_pid_as_given", got.pid, 4321);
    return finish();
}
