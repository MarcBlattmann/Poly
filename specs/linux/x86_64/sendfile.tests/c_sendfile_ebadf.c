/* EBADF: either descriptor closed or negative, in_fd not open for reading
 * (O_WRONLY, O_PATH), out_fd not open for writing (O_RDONLY, O_PATH, a
 * directory). Also with count 0. Nothing is moved.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40

#define F "/tmp/c_sendfile_ebadf_in"
#define G "/tmp/c_sendfile_ebadf_out"

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }

int test_main(void) {
    i64 off = 0;
    struct a_stat st;
    expect("setup_in", a_make_file(F, "0123456789", 0644), 0);
    i64 in = a_open(F, O_RDONLY, 0);
    i64 out = a_open(G, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    expect("closed_in", sendfile(out, 40, 0, 1), -EBADF);
    expect("closed_out", sendfile(41, in, 0, 1), -EBADF);
    expect("closed_both", sendfile(41, 40, 0, 1), -EBADF);
    expect("closed_in_count_0", sendfile(out, 40, 0, 0), -EBADF);
    expect("closed_out_count_0", sendfile(41, in, &off, 0), -EBADF);
    expect("negative_in", sendfile(out, -1, 0, 1), -EBADF);
    expect("negative_out", sendfile(-1, in, 0, 1), -EBADF);
    i64 wo = a_open(F, O_WRONLY, 0);
    expect("in_write_only", sendfile(out, wo, 0, 1), -EBADF);
    expect("in_write_only_offset", sendfile(out, wo, &off, 1), -EBADF);
    expect("out_read_only", sendfile(in, in, 0, 1), -EBADF);
    expect("both_wrong_mode", sendfile(in, wo, 0, 1), -EBADF);
    i64 pa = a_open(F, O_PATH, 0);
    expect("in_o_path", sendfile(out, pa, 0, 1), -EBADF);
    expect("out_o_path", sendfile(pa, in, 0, 1), -EBADF);
    i64 d = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("out_directory", sendfile(d, in, 0, 1), -EBADF);
    expect("in_offset_unmoved", syscall3(SYS_lseek, in, 0, SEEK_CUR), 0);
    expect("offset_unchanged", off, 0);
    expect("out_stat", a_stat_path(G, &st), 0);
    expect("out_still_empty", st.st_size, 0);
    a_close(d);
    a_close(pa);
    a_close(wo);
    a_close(in);
    a_close(out);
    expect("cleanup_in", a_unlink(F), 0);
    expect("cleanup_out", a_unlink(G), 0);
    return finish();
}
