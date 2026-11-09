/* Size limits that are negotiated at mount time. (a) The mount option
 * max_read=4096: reading the 300000-byte file /big in 64 KiB user reads
 * issues FUSE_READ requests of at most 4096 bytes each, and the data are
 * right. (b) max_write in the INIT reply (8192): writing 30000 bytes to a
 * new file issues FUSE_WRITE requests of at most 8192 bytes, contiguous,
 * covering exactly the bytes written, with the written data intact (the
 * server sums them). (c) FUSE_MAX_PAGES with max_pages 2 in the reply:
 * reads and writes are at most 2 pages (8192 bytes) per request. (d) With
 * no limit given (max_write 65536, no max_pages) reads stay within 131072
 * bytes and writes within 65536.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_ilim"

static u8 buf[300000];
static u8 data[30000];

static void check_reads(const char *tag, u32 limit, int mark) {
    char name[64];
    char *p = str_cpy(name, tag);
    int n = 0, bad = 0, over = 0;
    u64 covered = 0;
    for (int i = mark; i < fz->n; i++) {
        if (fz->rec[i].op != FZ_READ) continue;
        n++;
        if (fz->rec[i].a[1] > limit) over++;
        covered += fz->rec[i].a[1];
        (void)bad;
    }
    str_cpy(p, "_read_requests_seen");
    check(name, n > 0, n);
    str_cpy(p, "_no_read_over_limit");
    expect(name, over, 0);
    (void)covered;
}

static void read_big(const char *tag) {
    char name[64];
    char *p = str_cpy(name, tag);
    i64 fd = a_open(P "/big", O_RDONLY, 0);
    check("open_big", fd >= 0, fd);
    u64 total = 0;
    int ok = 1;
    for (;;) {
        i64 n = a_read(fd, buf, 65536);
        if (n <= 0) break;
        for (i64 i = 0; i < n; i++)
            if (buf[i] != fz_bigbyte(total + (u64)i)) ok = 0;
        total += (u64)n;
    }
    a_close(fd);
    str_cpy(p, "_big_total");
    expect(name, (i64)total, 300000);
    str_cpy(p, "_big_content");
    expect(name, ok, 1);
}

static void write_file(const char *tag, u32 limit, int mark_write, const char *file) {
    char name[64];
    char *p = str_cpy(name, tag);
    for (int i = 0; i < 30000; i++) data[i] = (u8)(i * 13 + 5);
    i64 fd = a_open(file, O_WRONLY | O_CREAT | O_EXCL, 0644);
    check("create", fd >= 0, fd);
    i64 n = a_write(fd, data, sizeof data);
    str_cpy(p, "_write_all");
    expect(name, n, 30000);
    a_close(fd);
    u64 next = 0;
    int contig = 1, over = 0, sums = 1, cnt = 0;
    for (int i = mark_write; i < fz->n; i++) {
        const struct fz_rec *r = &fz->rec[i];
        if (r->op != FZ_WRITE) continue;
        cnt++;
        if (r->a[0] != next) contig = 0;
        next += r->a[1];
        if (r->a[1] > limit) over++;
        if (r->a[0] + r->a[1] <= 30000 && r->sum != fz_sum(data + r->a[0], r->a[1], 0)) sums = 0;
    }
    str_cpy(p, "_write_requests_seen");
    check(name, cnt >= 1, cnt);
    str_cpy(p, "_writes_contiguous");
    expect(name, contig, 1);
    str_cpy(p, "_writes_cover_all");
    expect(name, (i64)next, 30000);
    str_cpy(p, "_no_write_over_limit");
    expect(name, over, 0);
    str_cpy(p, "_write_data");
    expect(name, sums, 1);
}

int test_main(void) {
    expect("setup", fz_setup(P), 0);

    /* (a) max_read */
    expect("open_a", fz_open() >= 0, 1);
    expect("mount_a", fz_mount("max_read=4096"), 0);
    expect("start_a", fz_start() > 0, 1);
    int m = fz_mark();
    read_big("a");
    check_reads("a", 4096, m);
    syscall2(SYS_umount2, P, 0);
    fz_wait();

    /* (b) max_write */
    fz->max_write = 8192;
    expect("open_b", fz_open() >= 0, 1);
    expect("mount_b", fz_mount(0), 0);
    expect("start_b", fz_start() > 0, 1);
    m = fz_mark();
    write_file("b", 8192, m, P "/w1");
    syscall2(SYS_umount2, P, 0);
    fz_wait();

    /* (c) max_pages */
    fz->max_write = 65536;
    fz->init_flags = FZ_MAX_PAGES;
    fz->max_pages = 2;
    expect("open_c", fz_open() >= 0, 1);
    expect("mount_c", fz_mount(0), 0);
    expect("start_c", fz_start() > 0, 1);
    m = fz_mark();
    read_big("c");
    check_reads("c", 8192, m);
    m = fz_mark();
    write_file("c", 8192, m, P "/w2");
    syscall2(SYS_umount2, P, 0);
    fz_wait();

    /* (d) defaults */
    fz->init_flags = 0;
    fz->max_pages = 0;
    expect("open_d", fz_open() >= 0, 1);
    expect("mount_d", fz_mount(0), 0);
    expect("start_d", fz_start() > 0, 1);
    m = fz_mark();
    read_big("d");
    check_reads("d", 131072, m);
    m = fz_mark();
    write_file("d", 65536, m, P "/w3");
    syscall2(SYS_umount2, P, 0);
    fz_wait();
    fz_cleanup();
    return finish();
}
