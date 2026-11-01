/* vmsplice(2) on a FIFO: a FIFO opened O_WRONLY is a write end, O_RDONLY
 * a read end. A FIFO opened O_RDWR counts as a write end: vmsplice puts
 * the iovec's bytes into it (it never reads from it). splice(2) moves
 * the FIFO's bytes into a pipe like from any pipe.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_vmsplice 278
#define SYS_splice 275
#define SYS_mknodat 259
/* linux/stat.h */
#define S_IFIFO 010000
/* linux/splice.h */
#define SPLICE_F_NONBLOCK 2

#define F "/tmp/c_vmsplice_fifo"

static i64 vmsplice(i64 fd, const struct k_iovec *iov, u64 n, u64 flags) {
    return syscall4(SYS_vmsplice, fd, iov, n, flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}
static char msg[] = "abc";

int test_main(void) {
    syscall1(SYS_unlink, F);
    expect("mkfifo", syscall4(SYS_mknodat, AT_FDCWD, F, S_IFIFO | 0600, 0), 0);
    i64 rw = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR, 0);
    check("open_rdwr", rw >= 0, rw);
    struct k_iovec v = { msg, 3 };
    char a[8] = "1234567";
    struct k_iovec r = { a, 8 };

    expect("rdwr_writes", vmsplice(rw, &v, 1, 0), 3);
    expect("avail_3", avail(rw), 3);
    expect("rdwr_writes_again", vmsplice(rw, &r, 1, SPLICE_F_NONBLOCK), 8);
    expect("avail_11", avail(rw), 11);

    i64 rd = syscall4(SYS_openat, AT_FDCWD, F, O_RDONLY | O_NONBLOCK, 0);
    i64 wr = syscall4(SYS_openat, AT_FDCWD, F, O_WRONLY, 0);
    check("open_ends", rd >= 0 && wr >= 0, rd);
    struct k_iovec r3 = { a, 3 };
    expect("rdonly_reads", vmsplice(rd, &r3, 1, 0), 3);
    check("rdonly_data", mem_eq(a, "abc", 3), 0);
    expect("wronly_writes", vmsplice(wr, &v, 1, 0), 3);
    expect("avail_11_b", avail(rd), 11);

    int p[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    expect("splice_fifo_to_pipe", syscall6(SYS_splice, rd, 0, p[1], 0, 100, 0), 11);
    expect("pipe_avail", avail(p[0]), 11);

    close(p[0]);
    close(p[1]);
    close(rd);
    close(wr);
    close(rw);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
