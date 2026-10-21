/* signalfd (syscall 282): signalfd4 with flags 0: blocking, no
 * FD_CLOEXEC; -1 makes a new descriptor, an existing signalfd gets the new
 * mask; sigsetsize must be 8. A signal that is not blocked is delivered to
 * its handler as usual and never shows up in the signalfd.
 * poly-diff: syscalls=names (the records read carry the sender's pid)
 */
#include "poly_ipc.h"

static volatile int hits;
static void on_sig(int sig) {
    (void)sig;
    hits++;
}

int test_main(void) {
    u64 m = SIGBIT(SIGUSR1);
    i64 fd = syscall3(SYS_signalfd, -1, &m, 8);
    check("returns_fd", fd == 3, fd);
    check("no_cloexec", syscall2(SYS_fcntl, fd, F_GETFD) == 0, 0);
    check("blocking", syscall2(SYS_fcntl, fd, F_GETFL) == O_RDWR, 0);

    sig_handler(SIGUSR1, on_sig, 0);
    raise_sig(SIGUSR1);
    check("unblocked_signal_goes_to_handler", hits == 1, hits);
    struct k_pollfd pf = { (int)fd, POLLIN, 0 };
    i64 r = syscall3(SYS_poll, &pf, 1, 0);
    check("handled_signal_not_in_fd", r == 0, r);

    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2), 0);
    raise_sig(SIGUSR1);
    struct k_signalfd_siginfo si;
    r = read((int)fd, &si, sizeof si);
    check("blocked_signal_read", r == 128 && si.ssi_signo == SIGUSR1, r);

    m = SIGBIT(SIGUSR2);
    r = syscall3(SYS_signalfd, fd, &m, 8);
    check("update_mask", r == fd, r);
    raise_sig(SIGUSR2);
    r = read((int)fd, &si, sizeof si);
    check("new_mask_read", r == 128 && si.ssi_signo == SIGUSR2, si.ssi_signo);
    r = syscall3(SYS_signalfd, -1, &m, 16);
    check("sigsetsize_16_einval", r == -EINVAL, r);
    return finish();
}
