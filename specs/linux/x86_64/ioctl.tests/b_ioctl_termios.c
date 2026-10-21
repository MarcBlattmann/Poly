/* Line discipline through TCGETS/TCSETS on a pty: a new slave is in
 * canonical mode with echo (ICANON, ECHO, ISIG set; OPOST|ONLCR;
 * ICRNL); in canonical mode the slave reads nothing until a newline, then
 * the whole line, and the master reads the echo (with "\n" as "\r\n");
 * output "\n" from the slave reaches the master as "\r\n"; the erase
 * character removes the previous one; after clearing ICANON and ECHO
 * (VMIN 1, VTIME 0) each byte is readable at once and nothing is echoed;
 * clearing OPOST sends "\n" unchanged.
 * poly-diff: syscalls=off (pty input is passed on asynchronously, so the
 * number of reads that collect it varies)
 */
#include "poly_ipc.h"

static i64 drain(int fd, char *buf, int cap) {
    struct k_pollfd pf = { fd, POLLIN, 0 };
    i64 got = 0;
    while (got < cap && syscall3(SYS_poll, &pf, 1, 50) == 1) {
        i64 r = read(fd, buf + got, (u64)(cap - got));
        if (r <= 0) break;
        got += r;
    }
    return got;
}

int test_main(void) {
    int m = (int)syscall4(SYS_openat, AT_FDCWD, "/dev/ptmx", O_RDWR | O_NOCTTY, 0);
    int zero = 0;
    syscall3(SYS_ioctl, m, TIOCSPTLCK, &zero);
    int s = (int)syscall3(SYS_ioctl, m, TIOCGPTPEER, O_RDWR | O_NOCTTY | O_NONBLOCK);
    struct k_termios t;
    syscall3(SYS_ioctl, s, TCGETS, &t);
    check("default_canonical_echo_isig", (t.c_lflag & (ICANON | ECHO | ISIG)) == (ICANON | ECHO | ISIG), t.c_lflag);
    check("default_opost_onlcr", (t.c_oflag & (OPOST | ONLCR)) == (OPOST | ONLCR), t.c_oflag);
    check("default_icrnl", (t.c_iflag & ICRNL) != 0, t.c_iflag);
    check("default_veof_ctrl_d", t.c_cc[VEOF] == 4, t.c_cc[VEOF]);

    char buf[32];
    write(m, "ab", 2);
    sleep_ms(10);
    i64 r = read(s, buf, sizeof buf);
    check("no_line_yet_eagain", r == -EAGAIN, r);
    write(m, "c\n", 2);
    sleep_ms(10);
    r = read(s, buf, sizeof buf);
    check("whole_line_read", r == 4 && mem_eq(buf, "abc\n", 4), r);
    r = drain(m, buf, sizeof buf);
    check("echo_with_crlf", r == 5 && mem_eq(buf, "abc\r\n", 5), r);

    char erase = (char)t.c_cc[VERASE];
    char line[4] = { 'x', 'y', erase, '\n' };
    write(m, line, 4);
    sleep_ms(10);
    r = read(s, buf, sizeof buf);
    check("erase_removes_previous", r == 2 && buf[0] == 'x' && buf[1] == '\n', r);
    drain(m, buf, sizeof buf);

    write(s, "hi\n", 3);
    r = drain(m, buf, sizeof buf);
    check("output_onlcr", r == 4 && mem_eq(buf, "hi\r\n", 4), r);

    t.c_lflag &= ~(ICANON | ECHO);
    t.c_cc[VMIN] = 1;
    t.c_cc[VTIME] = 0;
    r = syscall3(SYS_ioctl, s, TCSETS, &t);
    check("tcsets_ok", r == 0, r);
    struct k_termios back;
    syscall3(SYS_ioctl, s, TCGETS, &back);
    check("tcsets_read_back", (back.c_lflag & (ICANON | ECHO)) == 0, back.c_lflag);
    write(m, "z", 1);
    sleep_ms(10);
    r = read(s, buf, sizeof buf);
    check("raw_byte_at_once", r == 1 && buf[0] == 'z', r);
    r = drain(m, buf, sizeof buf);
    check("raw_no_echo", r == 0, r);

    t.c_oflag &= ~OPOST;
    syscall3(SYS_ioctl, s, TCSETS, &t);
    write(s, "a\n", 2);
    r = drain(m, buf, sizeof buf);
    check("no_opost_newline_unchanged", r == 2 && mem_eq(buf, "a\n", 2), r);
    return finish();
}
