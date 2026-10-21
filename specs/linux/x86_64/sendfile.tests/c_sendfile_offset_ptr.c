/* sendfile with an offset pointer reads in_fd from *offset, stores the
 * offset after the last byte read back into *offset and leaves in_fd's own
 * file offset alone. At or past end of file it returns 0 and *offset is
 * unchanged; a count of 0 returns 0. out_fd's offset still advances.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40

#define F "/tmp/c_sendfile_offset_ptr_in"
#define G "/tmp/c_sendfile_offset_ptr_out"

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }
static i64 pos(i64 fd) { return syscall3(SYS_lseek, fd, 0, SEEK_CUR); }

int test_main(void) {
    char buf[32];
    i64 off;
    expect("setup_in", a_make_file(F, "abcdefghij", 0644), 0);
    i64 in = a_open(F, O_RDONLY, 0);
    i64 out = a_open(G, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("move_in_offset", a_read(in, buf, 1), 1);
    off = 6;
    expect("send_at_6", sendfile(out, in, &off, 2), 2);
    expect("offset_8", off, 8);
    expect("in_offset_untouched", pos(in), 1);
    expect("out_offset_2", pos(out), 2);
    expect("send_short", sendfile(out, in, &off, 100), 2);
    expect("offset_10", off, 10);
    expect("send_at_end", sendfile(out, in, &off, 100), 0);
    expect("offset_still_10", off, 10);
    off = 1L << 40;
    expect("send_far_past_end", sendfile(out, in, &off, 5), 0);
    expect("far_offset_unchanged", off, 1L << 40);
    off = 3;
    expect("send_count_0", sendfile(out, in, &off, 0), 0);
    expect("count_0_offset_unchanged", off, 3);
    off = 0;
    expect("send_from_0", sendfile(out, in, &off, 3), 3);
    expect("offset_3", off, 3);
    expect("in_offset_still_1", pos(in), 1);
    expect("out_offset_7", pos(out), 7);
    expect("out_pread", syscall4(SYS_pread64, out, buf, 32, 0), 7);
    check("out_contents", mem_eq(buf, "ghijabc", 7), buf[0]);
    /* A read through in_fd continues at its own offset. */
    expect("read_continues", a_read(in, buf, 2), 2);
    check("read_continues_contents", mem_eq(buf, "bc", 2), buf[0]);
    a_close(in);
    a_close(out);
    expect("cleanup_in", a_unlink(F), 0);
    expect("cleanup_out", a_unlink(G), 0);
    return finish();
}
