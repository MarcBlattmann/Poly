/* poll on pipes: the number of entries with nonzero revents is returned;
 * an empty pipe's read end is not ready (timeout 0 returns 0 at once), a
 * write end with space reports POLLOUT, data gives POLLIN (each only
 * when asked for, as are POLLRDNORM and POLLWRNORM); a closed write end gives POLLHUP on the read end
 * (with POLLIN while data remains); a closed read end gives POLLERR on
 * the write end; POLLHUP and POLLERR are reported even when not asked
 * for; a negative fd is skipped (revents 0); a closed descriptor gives
 * POLLNVAL and counts; revents is always overwritten. */
#include "poly_ipc.h"

int test_main(void) {
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    struct k_pollfd f[2] = { { p[0], POLLIN, 0x7777 }, { p[1], POLLOUT, 0x7777 } };
    i64 r = syscall3(SYS_poll, f, 2, 0);
    check("one_ready", r == 1, r);
    check("empty_read_end_not_ready", f[0].revents == 0, f[0].revents);
    check("write_end_pollout_only_as_asked", f[1].revents == POLLOUT, f[1].revents);

    f[0].events = POLLIN | POLLOUT;
    syscall3(SYS_poll, f, 1, 0);
    check("read_end_never_writable", f[0].revents == 0, f[0].revents);

    write(p[1], "ab", 2);
    f[0].events = POLLIN;
    r = syscall3(SYS_poll, f, 2, 0);
    check("both_ready", r == 2, r);
    check("data_pollin", f[0].revents == POLLIN, f[0].revents);
    f[0].events = POLLIN | POLLRDNORM;
    syscall3(SYS_poll, f, 1, 0);
    check("rdnorm_when_asked", f[0].revents == (POLLIN | POLLRDNORM), f[0].revents);
    f[0].events = POLLRDNORM;
    syscall3(SYS_poll, f, 1, 0);
    check("only_requested_bits", f[0].revents == POLLRDNORM, f[0].revents);
    f[0].events = POLLPRI;
    r = syscall3(SYS_poll, f, 1, 0);
    check("unrequested_pollin_not_reported", r == 0 && f[0].revents == 0, f[0].revents);

    /* Writer gone, data left: POLLIN and POLLHUP; drained: POLLHUP only. */
    close(p[1]);
    f[0].events = POLLIN;
    syscall3(SYS_poll, f, 1, 0);
    check("hup_with_data", f[0].revents == (POLLIN | POLLHUP), f[0].revents);
    char buf[4];
    read(p[0], buf, 2);
    syscall3(SYS_poll, f, 1, 0);
    check("hup_drained", f[0].revents == POLLHUP, f[0].revents);
    f[0].events = 0;
    r = syscall3(SYS_poll, f, 1, 0);
    check("hup_reported_without_asking", r == 1 && f[0].revents == POLLHUP, f[0].revents);
    close(p[0]);

    /* Reader gone: POLLERR on the write end (plus POLLOUT). */
    syscall2(SYS_pipe2, p, 0);
    close(p[0]);
    struct k_pollfd w = { p[1], 0, 0 };
    r = syscall3(SYS_poll, &w, 1, 0);
    check("err_without_asking", r == 1 && w.revents == POLLERR, w.revents);
    w.events = POLLOUT;
    syscall3(SYS_poll, &w, 1, 0);
    check("err_with_pollout", w.revents == (POLLOUT | POLLERR), w.revents);

    /* Negative fd skipped; closed fd is POLLNVAL. */
    struct k_pollfd g[3] = { { -1, POLLIN, 0x55 }, { 90, POLLIN, 0 }, { p[1], POLLOUT, 0 } };
    r = syscall3(SYS_poll, g, 3, 0);
    check("nval_and_ready_counted", r == 2, r);
    check("negative_fd_revents_zero", g[0].revents == 0, g[0].revents);
    check("closed_fd_pollnval", g[1].revents == POLLNVAL, g[1].revents);
    g[1].events = 0;
    syscall3(SYS_poll, g, 3, 0);
    check("pollnval_without_asking", g[1].revents == POLLNVAL, g[1].revents);
    struct k_pollfd neg = { -5, POLLIN, 0 };
    r = syscall3(SYS_poll, &neg, 1, 0);
    check("only_negative_fds_zero", r == 0, r);
    close(p[1]);

    /* Duplicate entries for the same fd are each reported. */
    syscall2(SYS_pipe2, p, 0);
    struct k_pollfd d[2] = { { p[1], POLLOUT, 0 }, { p[1], POLLOUT, 0 } };
    r = syscall3(SYS_poll, d, 2, 0);
    check("duplicates_counted_twice", r == 2, r);
    return finish();
}
