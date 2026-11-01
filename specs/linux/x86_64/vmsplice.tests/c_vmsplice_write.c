/* vmsplice(2) on a pipe's write end gathers the iovec's bytes into the
 * pipe, in order, and returns how many. Zero-length segments (even with a
 * NULL base) are skipped; a total of 0 (nr_segs 0, or only empty
 * segments) returns 0. Read-only memory can be the source. SPLICE_F_GIFT
 * and the other defined flags are accepted. The iovec array itself is
 * only read.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_vmsplice 278
/* linux/splice.h */
#define SPLICE_F_MOVE 1
#define SPLICE_F_MORE 4
#define SPLICE_F_GIFT 8

static i64 vmsplice(i64 fd, const struct k_iovec *iov, u64 n, u64 flags) {
    return syscall4(SYS_vmsplice, fd, iov, n, flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}

static char hello[] = "hello ", world[] = "world";

int test_main(void) {
    int p[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    char buf[64];

    struct k_iovec v[2] = { { hello, 6 }, { world, 5 } };
    expect("gather", vmsplice(p[1], v, 2, 0), 11);
    expect("read", read(p[0], buf, 64), 11);
    check("data", mem_eq(buf, "hello world", 11), 0);

    expect("nr_0", vmsplice(p[1], v, 0, 0), 0);
    expect("nr_0_null", vmsplice(p[1], 0, 0, 0), 0);
    struct k_iovec empty[2] = { { 0, 0 }, { 0, 0 } };
    expect("only_empty_segments", vmsplice(p[1], empty, 2, 0), 0);
    expect("still_empty", avail(p[0]), 0);
    struct k_iovec mixed[3] = { { 0, 0 }, { world, 5 }, { hello, 0 } };
    expect("empty_segments_skipped", vmsplice(p[1], mixed, 3, 0), 5);
    expect("read_2", read(p[0], buf, 64), 5);
    check("data_2", mem_eq(buf, "world", 5), 0);

    expect("gift", vmsplice(p[1], v, 2, SPLICE_F_GIFT), 11);
    expect("move_more", vmsplice(p[1], v, 1, SPLICE_F_MOVE | SPLICE_F_MORE), 6);
    expect("read_3", read(p[0], buf, 64), 17);
    check("data_3", mem_eq(buf, "hello worldhello ", 17), 0);

    /* A read-only source page, and a read-only iovec array. */
    i64 page = syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", page > 0, page);
    memcpy((void *)page, "rodata", 6);
    struct k_iovec *rv = (struct k_iovec *)(page + 64);
    rv[0].base = (void *)page;
    rv[0].len = 6;
    expect("mprotect", syscall3(SYS_mprotect, page, 4096, PROT_READ), 0);
    expect("readonly_source", vmsplice(p[1], rv, 1, 0), 6);
    expect("read_4", read(p[0], buf, 64), 6);
    check("data_4", mem_eq(buf, "rodata", 6), 0);

    /* After the pipe's reader is done, more vmsplices queue up in order. */
    struct k_iovec one = { hello, 6 }, two = { world, 5 };
    expect("queue_1", vmsplice(p[1], &one, 1, 0), 6);
    expect("queue_2", vmsplice(p[1], &two, 1, 0), 5);
    expect("queued", avail(p[0]), 11);
    expect("read_5", read(p[0], buf, 64), 11);
    check("data_5", mem_eq(buf, "hello world", 11), 0);

    close(p[0]);
    close(p[1]);
    return finish();
}
