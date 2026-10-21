/* sendfile with a NULL offset reads from in_fd's file offset and advances
 * it by the bytes moved; the bytes go to out_fd at its file offset, which
 * advances too. Short at end of file, then 0 with nothing moved. A count
 * of 0 is 0. A large file to a regular file moves everything in one call.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40

#define F "/tmp/c_sendfile_null_offset_in"
#define G "/tmp/c_sendfile_null_offset_out"

static char big[200000];

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }
static i64 pos(i64 fd) { return syscall3(SYS_lseek, fd, 0, SEEK_CUR); }

int test_main(void) {
    char buf[32];
    struct a_stat st;
    expect("setup_in", a_make_file(F, "0123456789", 0644), 0);
    i64 in = a_open(F, O_RDONLY, 0);
    i64 out = a_open(G, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("send_4", sendfile(out, in, 0, 4), 4);
    expect("in_offset_4", pos(in), 4);
    expect("out_offset_4", pos(out), 4);
    expect("send_3", sendfile(out, in, 0, 3), 3);
    expect("in_offset_7", pos(in), 7);
    expect("out_offset_7", pos(out), 7);
    expect("send_short_at_end", sendfile(out, in, 0, 100), 3);
    expect("in_offset_10", pos(in), 10);
    expect("send_at_end_0", sendfile(out, in, 0, 100), 0);
    expect("send_count_0", sendfile(out, in, 0, 0), 0);
    expect("out_offset_10", pos(out), 10);
    expect("out_pread", syscall4(SYS_pread64, out, buf, 32, 0), 10);
    check("out_contents", mem_eq(buf, "0123456789", 10), buf[0]);
    /* Rewinding in_fd sends the same bytes again, after the earlier ones. */
    expect("rewind_in", syscall3(SYS_lseek, in, 2, SEEK_SET), 2);
    expect("send_again", sendfile(out, in, 0, 2), 2);
    expect("out_pread_again", syscall4(SYS_pread64, out, buf, 32, 0), 12);
    check("out_contents_again", mem_eq(buf, "012345678923", 12), buf[10]);
    a_close(in);
    a_close(out);

    /* 200000 bytes file to file: all of it in one call. */
    for (int i = 0; i < 200000; i++) big[i] = (char)('a' + i % 23);
    i64 w = a_open(F, O_WRONLY | O_TRUNC, 0);
    expect("setup_big", a_write(w, big, 200000), 200000);
    a_close(w);
    in = a_open(F, O_RDONLY, 0);
    out = a_open(G, O_WRONLY | O_TRUNC, 0);
    expect("send_big", sendfile(out, in, 0, 200000), 200000);
    expect("big_in_offset", pos(in), 200000);
    expect("big_out_size", a_stat_path(G, &st), 0);
    expect("big_out_size_value", st.st_size, 200000);
    a_close(out);
    out = a_open(G, O_RDONLY, 0);
    expect("big_tail_pread", syscall4(SYS_pread64, out, buf, 10, 199990), 10);
    check("big_tail_contents", mem_eq(buf, big + 199990, 10), buf[0]);
    a_close(in);
    a_close(out);
    expect("cleanup_in", a_unlink(F), 0);
    expect("cleanup_out", a_unlink(G), 0);
    return finish();
}
