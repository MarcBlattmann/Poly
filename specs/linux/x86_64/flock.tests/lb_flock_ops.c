/* flock(2) operations: LOCK_SH 1, LOCK_EX 2, LOCK_UN 8, optionally with
 * LOCK_NB 4. Exactly one of SH/EX/UN is required (0, 3, LOCK_NB alone and
 * unknown bits are EINVAL, checked before the descriptor); LOCK_MAND (32)
 * makes the call a no-op that returns 0; the operation is a 32-bit value.
 * Any open descriptor except O_PATH can be locked: files of any access
 * mode, directories, pipes, sockets, devices. Unlocking an unlocked file
 * and relocking with the same lock succeed. */
#include "a_sys.h"

#define D "/tmp/lb_flock_ops"
#define SYS_flock 73
#define SYS_socket 41
#define LOCK_SH 1
#define LOCK_EX 2
#define LOCK_NB 4
#define LOCK_UN 8
#define LOCK_MAND 32

static i64 flock_(i64 fd, i64 op) { return syscall2(SYS_flock, fd, op); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    i64 rw = a_open(D "/f", O_RDWR, 0);
    i64 ro = a_open(D "/f", O_RDONLY, 0);
    i64 wo = a_open(D "/f", O_WRONLY, 0);
    i64 op = a_open(D "/f", O_PATH, 0);
    i64 dir = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i32 pp[2];
    syscall2(SYS_pipe2, pp, 0);
    i64 sock = syscall3(SYS_socket, 1, 1, 0);
    i64 null = a_open("/dev/null", O_RDONLY, 0);

    expect("op_0", flock_(rw, 0), -EINVAL);
    expect("op_sh_ex", flock_(rw, LOCK_SH | LOCK_EX), -EINVAL);
    expect("op_nb_alone", flock_(rw, LOCK_NB), -EINVAL);
    expect("op_16", flock_(rw, 16), -EINVAL);
    expect("op_64", flock_(rw, 64), -EINVAL);
    expect("op_ex_64", flock_(rw, LOCK_EX | 64), -EINVAL);
    expect("op_sh_un", flock_(rw, LOCK_SH | LOCK_UN), -EINVAL);
    expect("op_einval_before_ebadf", flock_(-1, 0), -EINVAL);
    expect("mand_noop", flock_(rw, LOCK_MAND), 0);
    expect("mand_with_ex_noop", flock_(rw, LOCK_MAND | LOCK_EX), 0);
    expect("mand_on_bad_fd_noop", flock_(-1, LOCK_MAND), 0);
    expect("upper_half_ignored", flock_(rw, (i64)0x500000000L | LOCK_EX), 0);
    expect("unlock", flock_(rw, LOCK_UN), 0);
    expect("unlock_nb", flock_(rw, LOCK_UN | LOCK_NB), 0);
    expect("unlock_unlocked", flock_(ro, LOCK_UN), 0);

    expect("bad_fd", flock_(-1, LOCK_EX), -EBADF);
    expect("unopened_fd", flock_(600, LOCK_SH), -EBADF);
    expect("opath", flock_(op, LOCK_EX), -EBADF);
    expect("rdonly_ex", flock_(ro, LOCK_EX), 0);
    expect("rdonly_ex_again", flock_(ro, LOCK_EX), 0);
    expect("rdonly_un", flock_(ro, LOCK_UN), 0);
    expect("wronly_sh", flock_(wo, LOCK_SH), 0);
    expect("wronly_sh_again", flock_(wo, LOCK_SH | LOCK_NB), 0);
    expect("wronly_un", flock_(wo, LOCK_UN), 0);
    expect("dir_ex", flock_(dir, LOCK_EX | LOCK_NB), 0);
    expect("pipe_read_ex", flock_(pp[0], LOCK_EX | LOCK_NB), 0);
    expect("pipe_write_ex_conflicts", flock_(pp[1], LOCK_EX | LOCK_NB), -EAGAIN);
    expect("socket_ex", flock_(sock, LOCK_EX), 0);
    expect("devnull_sh", flock_(null, LOCK_SH), 0);
    /* conversions on one open file description */
    expect("sh", flock_(rw, LOCK_SH), 0);
    expect("sh_to_ex", flock_(rw, LOCK_EX | LOCK_NB), 0);
    expect("ex_to_sh", flock_(rw, LOCK_SH | LOCK_NB), 0);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
