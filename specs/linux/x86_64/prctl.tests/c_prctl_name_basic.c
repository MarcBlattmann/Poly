/* PR_SET_NAME / PR_GET_NAME: the calling thread's name is at most 15
 * bytes; a longer name is cut to its first 15 bytes. PR_GET_NAME always
 * writes exactly 16 bytes, the name padded with NUL bytes, and nothing
 * past them. An empty name is allowed. Arguments 3 to 5 are ignored by
 * both. /proc/self/comm shows the same name followed by a newline.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_SET_NAME 15
#define PR_GET_NAME 16

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static char buf[32];

static void fill(void) {
    for (int i = 0; i < 32; i++) buf[i] = 'Z';
}

static int all_nul(int from, int to) {
    for (int i = from; i < to; i++)
        if (buf[i]) return 0;
    return 1;
}

int test_main(void) {
    expect("set short name", pr(PR_SET_NAME, (i64) "short", 0, 0, 0), 0);
    fill();
    expect("get name", pr(PR_GET_NAME, (i64)buf, 0, 0, 0), 0);
    check("name is short", str_eq(buf, "short"), buf[0]);
    check("rest of 16 bytes is NUL", all_nul(5, 16), buf[5]);
    check("byte 16 untouched", buf[16] == 'Z', buf[16]);

    expect("set 26-byte name", pr(PR_SET_NAME, (i64) "abcdefghijklmnopqrstuvwxyz", 0, 0, 0), 0);
    fill();
    pr(PR_GET_NAME, (i64)buf, 0, 0, 0);
    check("cut to 15 bytes", str_eq(buf, "abcdefghijklmno"), str_len(buf));
    check("byte 15 is NUL", buf[15] == 0, buf[15]);
    check("byte 16 untouched after long", buf[16] == 'Z', buf[16]);

    expect("set exactly 15 bytes", pr(PR_SET_NAME, (i64) "123456789012345", 0, 0, 0), 0);
    fill();
    pr(PR_GET_NAME, (i64)buf, 0, 0, 0);
    check("15-byte name kept", str_eq(buf, "123456789012345"), str_len(buf));

    expect("set 16 bytes", pr(PR_SET_NAME, (i64) "1234567890123456", 0, 0, 0), 0);
    fill();
    pr(PR_GET_NAME, (i64)buf, 0, 0, 0);
    check("16-byte name cut", str_eq(buf, "123456789012345"), str_len(buf));

    expect("set empty name", pr(PR_SET_NAME, (i64) "", 0, 0, 0), 0);
    fill();
    expect("get empty name", pr(PR_GET_NAME, (i64)buf, 0, 0, 0), 0);
    check("empty name: 16 NUL bytes", all_nul(0, 16), buf[0]);

    expect("set with extra args", pr(PR_SET_NAME, (i64) "extra", -1, -1, -1), 0);
    fill();
    expect("get with extra args", pr(PR_GET_NAME, (i64)buf, -1, -1, -1), 0);
    check("extra args ignored", str_eq(buf, "extra"), buf[0]);

    pr(PR_SET_NAME, (i64) "a name, spaces!", 0, 0, 0);
    fill();
    pr(PR_GET_NAME, (i64)buf, 0, 0, 0);
    check("any bytes allowed", str_eq(buf, "a name, spaces!"), str_len(buf));

    pr(PR_SET_NAME, (i64) "viaproc", 0, 0, 0);
    char comm[64] = {0};
    i64 fd = a_open("/proc/self/comm", O_RDONLY, 0);
    i64 n = fd >= 0 ? a_read(fd, comm, sizeof comm - 1) : fd;
    if (fd >= 0) a_close(fd);
    expect("/proc/self/comm length", n, 8);
    check("/proc/self/comm text", str_eq(comm, "viaproc\n"), comm[0]);
    return finish();
}
