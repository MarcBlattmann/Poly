/* Reading a file. open sends FUSE_OPEN (opcode 14) with the file's node id
 * and the open flags (access mode in the low two bits); the reply's fh is
 * passed back in every READ (opcode 15), the FLUSH (25) that close sends
 * and the RELEASE (18) that follows when the last reference to the open
 * file goes away. Two opens get their own fh. A READ asks for at least
 * what the user asked for (the kernel may ask for more, as readahead) at
 * the file offset; the data of the reply are the data read; a read at or
 * past the end of the file returns 0. The server's errors (EIO, EACCES,
 * ENOENT) are the calls' errors. After an OPEN reply of ENOSYS the open
 * succeeds, is not asked for again, and READ and FLUSH carry fh 0.
 * (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define P "/tmp/pk_fuse_rread"

static u8 big[4096];

int test_main(void) {
    expect("setup", fz_setup(P), 0);
    expect("open", fz_open() >= 0, 1);
    expect("mount", fz_mount(0), 0);
    expect("start", fz_start() > 0, 1);

    i64 fd = a_open(P "/hello", O_RDONLY, 0);
    check("open_hello", fd >= 0, fd);
    const struct fz_rec *o = fz_last(FZ_OPEN);
    check("open_sent", o != 0, 0);
    if (o) {
        expect("open_node", (i64)o->nodeid, 2);
        expect("open_access_mode", o->a[0] & 3, O_RDONLY);
    }
    char b[64];
    memset(b, 0, sizeof b);
    expect("read_all", a_read(fd, b, sizeof b), 16);
    check("read_content", mem_eq(b, "hello from fuse\n", 16), 0);
    expect("read_at_eof", a_read(fd, b, sizeof b), 0);
    const struct fz_rec *r = fz_nth(FZ_READ, 0);
    check("read_sent", r != 0, 0);
    u64 fh = 0;
    if (r) {
        expect("read_node", (i64)r->nodeid, 2);
        expect("read_offset_zero", (i64)r->a[0], 0);
        check("read_size_covers_request", r->a[1] >= 16, (i64)r->a[1]);
        fh = r->a[3];
        check("read_fh_nonzero", fh != 0, (i64)fh);
    }
    expect("pread_middle", syscall4(SYS_pread64, fd, b, 5, 6), 5);
    check("pread_middle_content", mem_eq(b, "from ", 5), 0);
    expect("pread_past_end", syscall4(SYS_pread64, fd, b, 5, 100), 0);
    expect("lseek_set", syscall3(SYS_lseek, fd, 11, SEEK_SET), 11);
    expect("read_after_seek", a_read(fd, b, sizeof b), 5);
    check("read_after_seek_content", mem_eq(b, "fuse\n", 5), 0);
    struct a_stat st;
    expect("fstat_size", syscall2(SYS_fstat, fd, &st), 0);
    expect("fstat_size_value", st.st_size, 16);

    i64 fd2 = a_open(P "/hello", O_RDONLY, 0);
    check("open_second", fd2 >= 0, fd2);
    expect("read_second", a_read(fd2, b, sizeof b), 16);
    const struct fz_rec *r2 = fz_last(FZ_READ);
    check("second_open_has_own_fh", r2 != 0 && r2->a[3] != fh, 0);

    expect("close_first", a_close(fd), 0);
    check("flush_sent_by_close", fz_wait_count(FZ_FLUSH, 1, 2000) >= 1, fz_count(FZ_FLUSH));
    const struct fz_rec *f = fz_nth(FZ_FLUSH, 0);
    if (f) expect("flush_has_read_fh", (i64)f->a[0], (i64)fh);
    check("release_sent", fz_wait_count(FZ_RELEASE, 1, 2000) >= 1, fz_count(FZ_RELEASE));
    const struct fz_rec *rel = fz_nth(FZ_RELEASE, 0);
    if (rel) {
        expect("release_has_read_fh", (i64)rel->a[0], (i64)fh);
        expect("release_node", (i64)rel->nodeid, 2);
    }
    expect("close_second", a_close(fd2), 0);
    check("second_release", fz_wait_count(FZ_RELEASE, 2, 2000) >= 2, fz_count(FZ_RELEASE));

    /* a file larger than a page, read in pieces at an offset */
    fd = a_open(P "/big", O_RDONLY, 0);
    check("open_big", fd >= 0, fd);
    expect("pread_big", syscall4(SYS_pread64, fd, big, sizeof big, 250000), 4096);
    int ok = 1;
    for (int i = 0; i < 4096; i++)
        if (big[i] != fz_bigbyte(250000 + (u64)i)) ok = 0;
    expect("pread_big_content", ok, 1);
    expect("pread_big_tail", syscall4(SYS_pread64, fd, big, sizeof big, 299000), 1000);
    ok = 1;
    for (int i = 0; i < 1000; i++)
        if (big[i] != fz_bigbyte(299000 + (u64)i)) ok = 0;
    expect("pread_big_tail_content", ok, 1);
    a_close(fd);

    /* errors from the server */
    fz->fail_op = FZ_OPEN;
    fz->fail_err = EACCES;
    expect("open_eacces", a_open(P "/hello", O_RDONLY, 0), -EACCES);
    fz->fail_err = ENOENT;
    expect("open_enoent", a_open(P "/hello", O_RDONLY, 0), -ENOENT);
    fz->fail_err = EIO;
    expect("open_eio", a_open(P "/hello", O_RDONLY, 0), -EIO);
    fz->fail_op = FZ_READ;
    fz->fail_err = EIO;
    fd = a_open(P "/dir/inner", O_RDONLY, 0);
    check("open_for_failing_read", fd >= 0, fd);
    expect("read_eio", a_read(fd, b, sizeof b), -EIO);
    a_close(fd);
    fz->fail_op = 0;

    /* OPEN answered with ENOSYS */
    fz->fail_op = FZ_OPEN;
    fz->fail_err = ENOSYS;
    int mark = fz_mark();
    fd = a_open(P "/dir/inner", O_RDONLY, 0);
    check("open_enosys_succeeds", fd >= 0, fd);
    fz->fail_op = 0;
    expect("open_enosys_asked_once", fz_count_since(mark, FZ_OPEN), 1);
    expect("read_without_open", a_read(fd, b, sizeof b), 6);
    check("read_without_open_content", mem_eq(b, "inner\n", 6), 0);
    const struct fz_rec *r3 = fz_nth_since(mark, FZ_READ, 0);
    check("read_sent_without_open", r3 != 0, 0);
    if (r3) expect("read_fh_zero", (i64)r3->a[3], 0);
    a_close(fd);
    mark = fz_mark();
    i64 fd3 = a_open(P "/dir/inner", O_RDONLY, 0);
    check("second_open_no_request", fd3 >= 0 && fz_count_since(mark, FZ_OPEN) == 0, fd3);
    a_close(fd3);
    fz_msleep(100);
    expect("no_release_without_open", fz_count_since(mark, FZ_RELEASE), 0);
    fz_cleanup();
    return finish();
}
