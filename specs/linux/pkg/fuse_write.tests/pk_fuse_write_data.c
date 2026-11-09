/* Writing data through an open file. Without a writeback cache a write
 * goes to the server at once: FUSE_WRITE (16) with the file's node id, the
 * fh of the open, the file offset, the byte count and the bytes; a write of
 * more than the negotiated max_write is split into requests of at most
 * that size at consecutive offsets; write_flags is 0 for plain writes.
 * pwrite uses its own offset and leaves the file position alone. O_APPEND
 * writes start at the end of the file as the kernel knows it (the size the
 * server reported). The WRITE reply's count is the write's result (a short
 * reply: that many bytes; the caller sees a short write). The server's
 * error (EIO, ENOSPC) is the write's error. The file's size seen by stat
 * after a write is the new end. FUSE_OPEN carries the open flags (O_WRONLY
 * or O_RDWR in the low two bits, O_APPEND kept) and close sends FLUSH then
 * RELEASE with the same fh; fsync sends FSYNC (20) with fsync_flags 0 and
 * fdatasync with 1.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_wdata"

static u8 data[20000];
static u8 back[20000];

int test_main(void) {
    expect("setup", fz_setup(P), 0);
    fz->max_write = 8192;
    expect("open", fz_open() >= 0, 1);
    expect("mount", fz_mount(0), 0);
    expect("start", fz_start() > 0, 1);

    /* overwrite part of an existing file */
    int mark = fz_mark();
    i64 fd = a_open(P "/hello", O_RDWR, 0);
    check("open_rdwr", fd >= 0, fd);
    const struct fz_rec *op = fz_nth_since(mark, FZ_OPEN, 0);
    check("open_sent", op != 0, 0);
    if (op) expect("open_access", op->a[0] & 3, O_RDWR);
    expect("pwrite_middle", syscall4(SYS_pwrite64, fd, "XYZ", 3, 6), 3);
    const struct fz_rec *w = fz_nth_since(mark, FZ_WRITE, 0);
    check("write_sent", w != 0, 0);
    if (w) {
        expect("write_node", (i64)w->nodeid, 2);
        expect("write_offset", (i64)w->a[0], 6);
        expect("write_size", (i64)w->a[1], 3);
        expect("write_flags_zero", (i64)w->a[2], 0);
        check("write_fh_nonzero", w->a[3] != 0, (i64)w->a[3]);
    }
    expect("position_unchanged_by_pwrite", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 0);
    expect("write_at_position", a_write(fd, "AB", 2), 2);
    w = fz_last(FZ_WRITE);
    if (w) expect("position_write_offset", (i64)w->a[0], 0);
    expect("position_after_write", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 2);
    expect("fsync", syscall1(74, fd), 0);
    const struct fz_rec *fs = fz_last(FZ_FSYNC);
    check("fsync_sent", fs != 0, 0);
    if (fs) expect("fsync_flags_zero", (i64)fs->a[1], 0);
    expect("fdatasync", syscall1(75, fd), 0);
    fs = fz_last(FZ_FSYNC);
    if (fs) expect("fdatasync_flag_one", (i64)fs->a[1], 1);
    expect("close", a_close(fd), 0);
    check("flush_after_writes", fz_wait_count(FZ_FLUSH, 1, 2000) >= 1, fz_count(FZ_FLUSH));
    check("release_sent", fz_wait_count(FZ_RELEASE, 1, 2000) >= 1, fz_count(FZ_RELEASE));
    char b[32];
    fd = a_open(P "/hello", O_RDONLY, 0);
    expect("reread", a_read(fd, b, sizeof b), 16);
    check("reread_content", mem_eq(b, "ABllo XYZm fuse\n", 16), 0);
    a_close(fd);

    /* grow beyond the end; size follows */
    fd = a_open(P "/hello", O_WRONLY, 0);
    expect("pwrite_past_end", syscall4(SYS_pwrite64, fd, "tail", 4, 100), 4);
    struct a_stat st;
    expect("fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("size_after_growth", st.st_size, 104);
    a_close(fd);

    /* a large write is split at max_write (8192) */
    for (int i = 0; i < 20000; i++) data[i] = (u8)(i * 31 + 7);
    i64 cfd = a_open(P "/large", O_RDWR | O_CREAT | O_EXCL, 0644);
    check("create_large", cfd >= 0, cfd);
    mark = fz_mark();
    expect("write_20000", a_write(cfd, data, 20000), 20000);
    u64 next = 0;
    int contig = 1, over = 0, sums = 1, cnt = 0;
    for (int i = mark; i < fz->n; i++) {
        const struct fz_rec *r = &fz->rec[i];
        if (r->op != FZ_WRITE) continue;
        cnt++;
        if (r->a[0] != next) contig = 0;
        if (r->a[1] > 8192) over++;
        if (r->sum != fz_sum(data + r->a[0], r->a[1], 0)) sums = 0;
        next += r->a[1];
    }
    check("split_into_requests", cnt >= 3, cnt);
    expect("split_contiguous", contig, 1);
    expect("split_none_over_max_write", over, 0);
    expect("split_covers_all", (i64)next, 20000);
    expect("split_data_intact", sums, 1);
    expect("lseek_start", syscall3(SYS_lseek, cfd, 0, SEEK_SET), 0);
    expect("read_back_large", a_read(cfd, back, sizeof back), 20000);
    expect("read_back_large_content", mem_eq(back, data, 20000), 1);
    a_close(cfd);

    /* O_APPEND */
    cfd = a_open(P "/dir/inner", O_WRONLY | O_APPEND, 0);
    check("open_append", cfd >= 0, cfd);
    mark = fz_mark();
    expect("append_write", a_write(cfd, "XY", 2), 2);
    const struct fz_rec *aw = fz_nth_since(mark, FZ_WRITE, 0);
    check("append_write_sent", aw != 0, 0);
    if (aw) expect("append_offset_is_end", (i64)aw->a[0], 6);
    expect("append_write_again", a_write(cfd, "Z", 1), 1);
    aw = fz_last(FZ_WRITE);
    if (aw) expect("append_second_offset", (i64)aw->a[0], 8);
    a_close(cfd);
    cfd = a_open(P "/dir/inner", O_RDONLY, 0);
    expect("append_result_size", a_read(cfd, b, sizeof b), 9);
    check("append_result", mem_eq(b, "inner\nXYZ", 9), 0);
    a_close(cfd);

    /* short WRITE reply */
    fz->write_short = 4;
    cfd = a_open(P "/dir/inner", O_WRONLY, 0);
    expect("short_write_result", a_write(cfd, "0123456789", 10), 4);
    fz->write_short = 0;
    a_close(cfd);

    /* errors */
    fz->fail_op = FZ_WRITE;
    fz->fail_err = EIO;
    cfd = a_open(P "/dir/inner", O_WRONLY, 0);
    expect("write_eio", a_write(cfd, "x", 1), -EIO);
    fz->fail_err = ENOSPC;
    expect("write_enospc", a_write(cfd, "x", 1), -ENOSPC);
    fz->fail_op = 0;
    a_close(cfd);
    fz->fail_op = FZ_OPEN;
    fz->fail_err = EACCES;
    expect("open_for_write_eacces", a_open(P "/hello", O_WRONLY, 0), -EACCES);
    fz->fail_op = 0;
    fz_cleanup();
    return finish();
}
