/* Pty packet mode (ioctl_tty(2) TIOCPKT, pty(7)): TIOCPKT switches it on
 * with a nonzero int and TIOCGPKT reads it back; in packet mode every read
 * of the master starts with a status byte, 0 before data; a slave's
 * tcflush(TCIFLUSH) is a one-byte read with TIOCPKT_FLUSHREAD, tcflow
 * (TCOOFF / TCOON) are TIOCPKT_STOP and TIOCPKT_START; poll reports the
 * master readable while a status is pending; switching packet mode off
 * gives plain data again. */
#include "poly_ipc.h"

#define K_TCXONC 0x540A
#define K_TCFLSH 0x540B
#define K_TCIFLUSH 0
#define K_TCOOFF 0
#define K_TCOON 1

static int readable(int fd) {
    struct k_pollfd pf = { fd, POLLIN, 0 };
    return syscall3(SYS_poll, &pf, 1, 100) == 1 && (pf.revents & POLLIN);
}

int test_main(void) {
    int m = (int)syscall4(SYS_openat, AT_FDCWD, "/dev/ptmx", O_RDWR | O_NOCTTY, 0);
    int zero = 0;
    syscall3(SYS_ioctl, m, TIOCSPTLCK, &zero);
    int s = (int)syscall3(SYS_ioctl, m, TIOCGPTPEER, O_RDWR | O_NOCTTY | O_NONBLOCK);

    int on = 0;
    i64 r = syscall3(SYS_ioctl, m, TIOCGPKT, &on);
    check("packet_mode_off_at_first", r == 0 && on == 0, r);
    on = 1;
    r = syscall3(SYS_ioctl, m, TIOCPKT, &on);
    check("tiocpkt_enable", r == 0, r);
    on = 0;
    r = syscall3(SYS_ioctl, m, TIOCGPKT, &on);
    check("tiocgpkt_reports_on", r == 0 && on == 1, r);

    unsigned char buf[32];
    write(s, "hi", 2);
    check("data_makes_master_readable", readable(m), 0);
    r = read(m, buf, sizeof buf);
    check("packet_has_zero_status_before_data", r == 3 && buf[0] == TIOCPKT_DATA && buf[1] == 'h' && buf[2] == 'i', r);

    r = syscall3(SYS_ioctl, s, K_TCFLSH, K_TCIFLUSH);
    check("slave_tcflush", r == 0, r);
    check("flush_makes_master_readable", readable(m), 0);
    r = read(m, buf, sizeof buf);
    check("flushread_status_packet", r == 1 && buf[0] == TIOCPKT_FLUSHREAD, r);

    syscall3(SYS_ioctl, s, K_TCXONC, K_TCOOFF);
    r = read(m, buf, sizeof buf);
    check("stop_status_packet", r == 1 && buf[0] == TIOCPKT_STOP, r);
    syscall3(SYS_ioctl, s, K_TCXONC, K_TCOON);
    r = read(m, buf, sizeof buf);
    check("start_status_packet", r == 1 && buf[0] == TIOCPKT_START, r);

    on = 0;
    r = syscall3(SYS_ioctl, m, TIOCPKT, &on);
    check("tiocpkt_disable", r == 0, r);
    write(s, "x", 1);
    r = read(m, buf, sizeof buf);
    check("plain_data_after_disable", r == 1 && buf[0] == 'x', r);
    return finish();
}
