/* A count larger than a blocking pipe holds, with another process
 * reading: the call returns a positive count no larger than requested and
 * *offset advances by exactly that; the reader receives exactly those
 * bytes, in order. Repeating the call until the count is used up moves the
 * whole file.
 * poly-diff: syscalls=off (two processes; how the bytes split across calls varies)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40

#define F "/tmp/c_sendfile_blocking_pipe_in"
#define SIZE 300000

static char big[SIZE];
static char rb[8192];

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }

int test_main(void) {
    for (int i = 0; i < SIZE; i++) big[i] = (char)('a' + (i * 7) % 26);
    i64 w = a_open(F, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    expect("setup_in", a_write(w, big, SIZE), SIZE);
    a_close(w);
    i64 in = a_open(F, O_RDONLY, 0);
    int p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    i64 pid = syscall0(SYS_fork);
    if (pid == 0) {
        a_close(p[1]);
        i64 got = 0, n;
        int same = 1;
        while ((n = a_read(p[0], rb, sizeof rb)) > 0) {
            if (got + n > SIZE || !mem_eq(rb, big + got, (size_t)n)) same = 0;
            got += n;
        }
        syscall1(SYS_exit, (got == SIZE && same) ? 0 : 1);
    }
    a_close(p[0]);
    i64 off = 0;
    i64 r = sendfile(p[1], in, &off, SIZE);
    check("first_positive", r > 0 && r <= SIZE, r);
    expect("first_offset", off, r);
    int calls = 1, ok = 1;
    while (off < SIZE && calls < 10000) {
        i64 before = off;
        r = sendfile(p[1], in, &off, (u64)(SIZE - off));
        calls++;
        if (r <= 0 || off != before + r) {
            ok = 0;
            break;
        }
    }
    check("all_sent", ok && off == SIZE, off);
    expect("at_end_0", sendfile(p[1], in, &off, 10), 0);
    expect("in_offset_untouched", syscall3(SYS_lseek, in, 0, SEEK_CUR), 0);
    a_close(p[1]);
    int status = 0;
    expect("wait_reader", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("reader_got_all_in_order", status, 0);
    a_close(in);
    expect("cleanup_in", a_unlink(F), 0);
    return finish();
}
