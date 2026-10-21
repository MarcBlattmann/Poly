/* Where the bytes land: at out_fd's file offset, overwriting in place
 * without truncating; an offset past the end leaves a hole of zero bytes.
 * /dev/null as out_fd swallows everything and reports it as sent.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40

#define F "/tmp/c_sendfile_out_position_in"
#define G "/tmp/c_sendfile_out_position_out"

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }

int test_main(void) {
    char buf[32];
    struct a_stat st;
    expect("setup_in", a_make_file(F, "0123456789", 0644), 0);
    expect("setup_out", a_make_file(G, "abcdefghij", 0644), 0);
    i64 in = a_open(F, O_RDONLY, 0);
    i64 out = a_open(G, O_RDWR, 0);
    expect("seek_out_3", syscall3(SYS_lseek, out, 3, SEEK_SET), 3);
    i64 off = 5;
    expect("overwrite", sendfile(out, in, &off, 4), 4);
    expect("out_offset_7", syscall3(SYS_lseek, out, 0, SEEK_CUR), 7);
    expect("overwrite_stat", a_stat_path(G, &st), 0);
    expect("size_unchanged", st.st_size, 10);
    expect("overwrite_pread", syscall4(SYS_pread64, out, buf, 32, 0), 10);
    check("overwrite_contents", mem_eq(buf, "abc5678hij", 10), buf[3]);

    expect("seek_out_20", syscall3(SYS_lseek, out, 20, SEEK_SET), 20);
    off = 0;
    expect("past_end", sendfile(out, in, &off, 2), 2);
    expect("out_offset_22", syscall3(SYS_lseek, out, 0, SEEK_CUR), 22);
    expect("past_end_stat", a_stat_path(G, &st), 0);
    expect("size_22", st.st_size, 22);
    expect("hole_pread", syscall4(SYS_pread64, out, buf, 32, 8), 14);
    check("hole_contents", mem_eq(buf, "ij\0\0\0\0\0\0\0\0\0\0" "01", 14), buf[2]);

    i64 dn = a_open("/dev/null", O_WRONLY, 0);
    off = 0;
    expect("to_dev_null", sendfile(dn, in, &off, 100), 10);
    expect("to_dev_null_offset", off, 10);
    expect("to_dev_null_null", sendfile(dn, in, 0, 4), 4);
    expect("to_dev_null_in_offset", syscall3(SYS_lseek, in, 0, SEEK_CUR), 4);
    a_close(dn);
    a_close(out);
    a_close(in);
    expect("cleanup_in", a_unlink(F), 0);
    expect("cleanup_out", a_unlink(G), 0);
    return finish();
}
