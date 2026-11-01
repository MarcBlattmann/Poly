/* fallocate(2) errors and their order: EBADF (not open), EINVAL (offset
 * < 0, len <= 0), EBADF (not open for writing: O_RDONLY, O_PATH),
 * ESPIPE (pipe), ENODEV (character device, socket), EFBIG (offset + len
 * beyond the largest file size, or beyond RLIMIT_FSIZE with SIGXFSZ). */
#include "a_sys.h"

#define D "/tmp/lb_fallocate_errors"
#define SYS_fallocate 285
#define SYS_socket 41
#define SIGXFSZ 25

static i64 falloc(i64 fd, i64 mode, i64 off, i64 len) { return syscall4(SYS_fallocate, fd, mode, off, len); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "abc", 0644), 0);
    i64 rw = a_open(D "/f", O_RDWR, 0);
    i64 ro = a_open(D "/f", O_RDONLY, 0);
    i64 op = a_open(D "/f", O_PATH, 0);
    i64 dir = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i32 pp[2];
    syscall2(SYS_pipe2, pp, 0);
    i64 null = a_open("/dev/null", O_WRONLY, 0);
    i64 sock = syscall3(SYS_socket, 1, 1, 0);
    check("opened", rw >= 0 && ro >= 0 && op >= 0 && dir >= 0 && null >= 0 && sock >= 0, 0);

    expect("bad_fd", falloc(-1, 0, 0, 10), -EBADF);
    expect("unopened_fd", falloc(900, 0, 0, 10), -EBADF);
    expect("negative_offset", falloc(rw, 0, -1, 10), -EINVAL);
    expect("zero_len", falloc(rw, 0, 0, 0), -EINVAL);
    expect("negative_len", falloc(rw, 0, 0, -5), -EINVAL);
    expect("rdonly", falloc(ro, 0, 0, 10), -EBADF);
    expect("opath", falloc(op, 0, 0, 10), -EBADF);
    expect("dir_rdonly", falloc(dir, 0, 0, 10), -EBADF);
    expect("pipe_write_end", falloc(pp[1], 0, 0, 10), -ESPIPE);
    expect("pipe_read_end", falloc(pp[0], 0, 0, 10), -EBADF);
    expect("devnull", falloc(null, 0, 0, 10), -ENODEV);
    expect("socket", falloc(sock, 0, 0, 10), -ENODEV);
    expect("beyond_max", falloc(rw, 0, 0x7fffffffffffff00L, 0x1000), -EFBIG);
    expect("size_unchanged", syscall2(SYS_fstat, rw, &st) == 0 ? st.st_size : -1, 3);
    /* order: a bad descriptor first, then offset/len, then the access mode */
    expect("bad_fd_vs_negative", falloc(-1, 0, -1, 0), -EBADF);
    expect("negative_vs_rdonly", falloc(ro, 0, -1, 10), -EINVAL);
    expect("zero_len_vs_pipe", falloc(pp[1], 0, 0, 0), -EINVAL);
    expect("zero_len_vs_opath", falloc(op, 0, 0, 0), -EBADF);
    /* bits outside every known mode */
    expect("unknown_mode_bit", falloc(rw, 0x40000000, 0, 10), -EOPNOTSUPP);
    expect("unknown_mode_vs_negative", falloc(rw, 0x40000000, -1, 10), -EINVAL);
    /* PUNCH_HOLE needs KEEP_SIZE */
    expect("punch_without_keep", falloc(rw, 2, 0, 10), -EOPNOTSUPP);

    /* RLIMIT_FSIZE: EFBIG and SIGXFSZ (ignored here) */
    struct a_sigaction ign = {SIG_IGN, 0, 0, 0};
    expect("ignore_sigxfsz", syscall4(SYS_rt_sigaction, SIGXFSZ, &ign, 0, 8), 0);
    /* the limit applies to the test's output too: collect, lift, check */
    struct a_rlimit lim = {4096, RLIM_INFINITY}, unlimited = {RLIM_INFINITY, RLIM_INFINITY};
    i64 set = syscall2(SYS_setrlimit, RLIMIT_FSIZE, &lim);
    i64 within = falloc(rw, 0, 0, 4096);
    i64 past = falloc(rw, 0, 0, 4097);
    i64 unset = syscall2(SYS_setrlimit, RLIMIT_FSIZE, &unlimited);
    expect("setrlimit", set, 0);
    expect("within_limit", within, 0);
    expect("past_limit", past, -EFBIG);
    expect("unset_rlimit", unset, 0);
    expect("size_at_limit", syscall2(SYS_fstat, rw, &st) == 0 ? st.st_size : -1, 4096);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
