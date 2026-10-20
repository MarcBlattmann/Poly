/* A pipe between two processes: the parent's blocking read waits until the
 * child writes; data written in several pieces arrives in order; the read
 * returns what is there (not necessarily the whole request); a blocked
 * writer on a full pipe resumes as the reader drains it; EOF when the
 * child exits.
 * poly-diff: syscalls=off (two processes; how the bytes split across reads varies)
 */
#include "poly_ipc.h"

static unsigned char buf[200000];

int test_main(void) {
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    i64 pid = fork();
    if (pid == 0) {
        close(p[0]);
        sleep_ms(20);
        write(p[1], "first", 5);
        /* 200000 bytes: blocks until the parent reads. */
        for (int i = 0; i < (int)sizeof buf; i++) buf[i] = (unsigned char)(i * 7);
        i64 n = write(p[1], buf, sizeof buf);
        exit_group(n == (i64)sizeof buf ? 0 : 1);
    }
    close(p[1]);
    char head[5];
    i64 got = 0;
    while (got < 5) {
        i64 r = read(p[0], head + got, 5 - got);
        if (r <= 0) break;
        got += r;
    }
    check("blocking_read_waits_for_writer", got == 5 && mem_eq(head, "first", 5), got);

    i64 total = 0, reads = 0;
    int in_order = 1;
    static unsigned char rbuf[65536];
    for (;;) {
        i64 r = read(p[0], rbuf, sizeof rbuf);
        if (r <= 0) break;
        for (i64 i = 0; i < r; i++) in_order &= rbuf[i] == (unsigned char)((total + i) * 7);
        total += r;
        reads++;
    }
    check("large_write_all_received", total == (i64)sizeof buf, total);
    check("bytes_in_order", in_order, 0);
    check("several_reads_needed", reads >= 3, reads);
    check("writer_completed", wait_exit(pid) == 0, 0);
    check("eof_after_child_exit", read(p[0], rbuf, 1) == 0, 0);
    return finish();
}
