/* signalfd4: a descriptor that reads blocked pending signals of its mask
 * as 128-byte signalfd_siginfo records (signal number, code, sender pid
 * and uid; the value for a queued signal); reading takes the signal (it is
 * no longer pending, the handler never runs); several pending signals come
 * out as several records in one read, lowest first; a buffer under 128
 * bytes is EINVAL; nothing pending is EAGAIN (SFD_NONBLOCK). Passing an
 * existing signalfd replaces its mask; SIGKILL and SIGSTOP in the mask are
 * ignored. Flags other than SFD_CLOEXEC|SFD_NONBLOCK are EINVAL, as is a
 * sigsetsize other than 8; an fd that is not a signalfd is EINVAL.
 * poly-diff: syscalls=names (the records read carry the sender's pid)
 */
#include "poly_ipc.h"

static volatile int hits;
static void on_sig(int sig) {
    (void)sig;
    hits++;
}

int test_main(void) {
    i64 pid = getpid();
    sig_handler(SIGUSR1, on_sig, 0);
    sig_handler(SIGUSR2, on_sig, 0);
    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2) | SIGBIT(SIGRTMIN), 0);
    u64 m = SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2);
    i64 fd = syscall4(SYS_signalfd4, -1, &m, 8, SFD_NONBLOCK | SFD_CLOEXEC);
    check("returns_fd", fd == 3, fd);
    check("cloexec", syscall2(SYS_fcntl, fd, F_GETFD) == FD_CLOEXEC, 0);
    check("rdwr_nonblock", syscall2(SYS_fcntl, fd, F_GETFL) == (O_RDWR | O_NONBLOCK), syscall2(SYS_fcntl, fd, F_GETFL));

    struct k_signalfd_siginfo si[3];
    i64 r = read((int)fd, si, sizeof si);
    check("nothing_pending_eagain", r == -EAGAIN, r);

    raise_sig(SIGUSR2);
    raise_sig(SIGUSR1);
    memset(si, 0, sizeof si);
    r = read((int)fd, si, sizeof si);
    check("two_records", r == 2 * 128, r);
    check("lowest_first", si[0].ssi_signo == SIGUSR1 && si[1].ssi_signo == SIGUSR2, si[0].ssi_signo);
    check("code_user", si[0].ssi_code == SI_USER, si[0].ssi_code);
    check("sender_pid", si[0].ssi_pid == (unsigned)pid, 0);
    check("sender_uid", si[0].ssi_uid == (unsigned)syscall0(SYS_getuid), si[0].ssi_uid);
    check("taken_not_pending", sig_pending() == 0, (i64)sig_pending());
    check("handler_never_ran", hits == 0, hits);

    raise_sig(SIGUSR1);
    r = read((int)fd, si, 100);
    check("small_buffer_einval", r == -EINVAL, r);
    check("small_buffer_left_pending", sig_pending() == SIGBIT(SIGUSR1), (i64)sig_pending());
    r = read((int)fd, si, 200);
    check("buffer_of_one_and_a_half_reads_one", r == 128 && si[0].ssi_signo == SIGUSR1, r);

    /* A signal not in the mask is not read, stays pending. */
    raise_sig(SIGRTMIN);
    r = read((int)fd, si, sizeof si);
    check("outside_mask_not_read", r == -EAGAIN, r);
    check("outside_mask_pending", sig_pending() == SIGBIT(SIGRTMIN), (i64)sig_pending());

    /* Replace the mask: now SIGRTMIN, with a queued value. */
    u64 m2 = SIGBIT(SIGRTMIN) | SIGBIT(SIGKILL) | SIGBIT(SIGSTOP);
    r = syscall4(SYS_signalfd4, fd, &m2, 8, 0);
    check("replace_mask_same_fd", r == fd, r);
    struct k_siginfo q;
    memset(&q, 0, sizeof q);
    q.si_signo = SIGRTMIN;
    q.si_code = SI_QUEUE;
    q.u.kill.pid = (int)pid;
    q.u.kill.value = 0xdead;
    syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN, &q);
    r = read((int)fd, si, sizeof si);
    check("rt_records_queued_twice", r == 2 * 128, r);
    check("first_from_kill", si[0].ssi_code == SI_USER, si[0].ssi_code);
    check("second_queued_value", si[1].ssi_code == SI_QUEUE && si[1].ssi_ptr == 0xdead, (i64)si[1].ssi_ptr);
    check("queued_int_field", si[1].ssi_int == 0xdead, si[1].ssi_int);

    r = syscall4(SYS_signalfd4, -1, &m, 8, 0x1);
    check("unknown_flag_einval", r == -EINVAL, r);
    r = syscall4(SYS_signalfd4, -1, &m, 4, 0);
    check("sigsetsize_4_einval", r == -EINVAL, r);
    r = syscall4(SYS_signalfd4, -1, 8, 8, 0);
    check("bad_mask_efault", r == -EFAULT, r);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    r = syscall4(SYS_signalfd4, p[0], &m, 8, 0);
    check("not_a_signalfd_einval", r == -EINVAL, r);
    r = syscall4(SYS_signalfd4, 90, &m, 8, 0);
    check("closed_fd_ebadf", r == -EBADF, r);
    r = write((int)fd, si, 128);
    check("write_einval", r == -EINVAL, r);
    return finish();
}
