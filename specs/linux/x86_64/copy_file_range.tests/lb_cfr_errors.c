/* copy_file_range(2) errors: EINVAL for flags != 0, for non-regular files
 * (pipes, devices) and for overlapping ranges in one file; EBADF for a
 * source not open for reading, a destination not open for writing or open
 * O_APPEND, O_PATH or closed descriptors; EISDIR for directories;
 * EOVERFLOW for negative offsets or a length that would pass the largest
 * offset; EFAULT for unreadable offset pointers. */
#include "a_sys.h"

#define D "/tmp/lb_cfr_errors"
#define SYS_copy_file_range 326

static i64 cfr(i64 in, i64 *oin, i64 out, i64 *oout, i64 len, i64 flags) {
    return syscall6(SYS_copy_file_range, in, (i64)oin, out, (i64)oout, len, flags);
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_src", a_make_file(D "/src", "0123456789", 0644), 0);
    i64 in = a_open(D "/src", O_RDONLY, 0);
    i64 out = a_open(D "/dst", O_WRONLY | O_CREAT, 0644);
    i64 rw = a_open(D "/src", O_RDWR, 0);
    i64 ap = a_open(D "/dst", O_WRONLY | O_APPEND, 0);
    i64 op = a_open(D "/src", O_PATH, 0);
    i64 dir = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i32 pp[2];
    syscall2(SYS_pipe2, pp, 0);
    i64 null = a_open("/dev/null", O_RDWR, 0);

    expect("flags_1", cfr(in, 0, out, 0, 4, 1), -EINVAL);
    expect("flags_high", cfr(in, 0, out, 0, 4, (i64)0x80000000L), -EINVAL);
    expect("src_wronly", cfr(out, 0, out, 0, 4, 0), -EBADF);
    expect("dst_rdonly", cfr(in, 0, in, 0, 4, 0), -EBADF);
    expect("dst_append", cfr(in, 0, ap, 0, 4, 0), -EBADF);
    expect("src_opath", cfr(op, 0, out, 0, 4, 0), -EBADF);
    expect("src_bad", cfr(-1, 0, out, 0, 4, 0), -EBADF);
    expect("dst_bad", cfr(in, 0, 900, 0, 4, 0), -EBADF);
    expect("src_dir", cfr(dir, 0, out, 0, 4, 0), -EISDIR);
    expect("src_pipe", cfr(pp[0], 0, out, 0, 4, 0), -EINVAL);
    expect("dst_pipe", cfr(in, 0, pp[1], 0, 4, 0), -EINVAL);
    expect("dst_devnull", cfr(in, 0, null, 0, 4, 0), -EINVAL);
    expect("src_devnull", cfr(null, 0, out, 0, 4, 0), -EINVAL);
    i64 a = 0, b = 2;
    expect("same_file_overlap", cfr(rw, &a, rw, &b, 4, 0), -EINVAL);
    a = 0;
    b = 0;
    expect("same_file_same_range", cfr(rw, &a, rw, &b, 4, 0), -EINVAL);
    a = 4;
    b = 2;
    expect("same_file_overlap_backwards", cfr(rw, &a, rw, &b, 4, 0), -EINVAL);
    a = -1;
    expect("negative_in_offset", cfr(in, &a, out, 0, 4, 0), -EOVERFLOW);
    b = -1;
    expect("negative_out_offset", cfr(in, 0, out, &b, 4, 0), -EOVERFLOW);
    expect("bad_in_pointer", cfr(in, (i64 *)1, out, 0, 4, 0), -EFAULT);
    expect("bad_out_pointer", cfr(in, 0, out, (i64 *)1, 4, 0), -EFAULT);
    expect("nothing_copied", syscall2(SYS_fstat, out, &st) == 0 ? st.st_size : -1, 0);
    /* a huge len is cut to what the source has, unless the destination
     * offset plus len would pass the largest offset */
    a = 0;
    b = 100;
    expect("huge_len_past_max_offset", cfr(in, &a, out, &b, -1, 0), -EOVERFLOW);
    a = 0;
    b = 0;
    expect("huge_len_from_0", cfr(in, &a, out, &b, -1, 0), 10);
    expect("huge_len_offsets", a == 10 && b == 10, 1);
    /* order: the descriptors first, then the flags */
    expect("bad_fd_vs_flags", cfr(-1, 0, -1, 0, 4, 1), -EBADF);
    expect("flags_vs_wrong_mode", cfr(out, 0, in, 0, 4, 1), -EINVAL);
    expect("bad_fd_vs_bad_pointer", cfr(-1, (i64 *)1, out, 0, 4, 0), -EBADF);
    expect("bad_pointer_vs_dir", cfr(dir, (i64 *)1, out, 0, 4, 0), -EFAULT);

    static i64 fds[8];
    fds[0] = in, fds[1] = out, fds[2] = rw, fds[3] = ap, fds[4] = op, fds[5] = dir, fds[6] = null;
    for (int i = 0; i < 7; i++) a_close(fds[i]);
    expect("cleanup_dst", a_unlink(D "/dst"), 0);
    expect("cleanup_src", a_unlink(D "/src"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
