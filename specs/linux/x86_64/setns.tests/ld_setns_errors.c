/* setns(fd, nstype) argument errors, the same for every caller (they are
 * checked before privileges): a descriptor that is not open, -1, or one
 * opened with O_PATH is EBADF, checked first (also with a bad nstype); a
 * descriptor that is neither a namespace file nor a pidfd (a regular
 * file, a directory, a pipe) is EINVAL; for a namespace file nstype must
 * be 0 or that namespace's CLONE_NEW* flag (only the low 32 bits count),
 * anything else (another type, two types, bits that are no type, -1) is
 * EINVAL. Joining one's own user namespace is EINVAL. The /proc/self/ns
 * files are regular files of mode 0444.
 */
#include "a_sys.h"

#define SYS_setns 308
/* linux/sched.h */
#define CLONE_NEWNS 0x20000
#define CLONE_NEWUTS 0x04000000
#define CLONE_NEWIPC 0x08000000
#define CLONE_NEWUSER 0x10000000
#define CLONE_NEWNET 0x40000000

int test_main(void) {
    a_block_sigchld();
    i64 uts = a_open("/proc/self/ns/uts", O_RDONLY, 0);
    expect("open_uts", uts, 3);
    struct a_stat st;
    syscall2(SYS_fstat, uts, &st);
    expect("ns_file_mode", st.st_mode, S_IFREG | 0444);

    expect("fd_minus1_ebadf", syscall2(SYS_setns, -1, 0), -EBADF);
    expect("fd_closed_ebadf", syscall2(SYS_setns, 50, 0), -EBADF);
    expect("fd_closed_bad_type_ebadf", syscall2(SYS_setns, 50, 1), -EBADF);
    i64 path = a_open("/proc/self/ns/uts", O_PATH, 0);
    expect("o_path_ebadf", syscall2(SYS_setns, path, 0), -EBADF);

    i64 reg = a_open("/proc/self/status", O_RDONLY, 0);
    expect("regular_file_einval", syscall2(SYS_setns, reg, 0), -EINVAL);
    expect("regular_file_newuts_einval", syscall2(SYS_setns, reg, CLONE_NEWUTS), -EINVAL);
    i64 dir = a_open("/proc/self/ns", O_RDONLY | O_DIRECTORY, 0);
    expect("ns_directory_einval", syscall2(SYS_setns, dir, 0), -EINVAL);
    i32 p[2];
    syscall2(SYS_pipe2, p, 0);
    expect("pipe_einval", syscall2(SYS_setns, p[0], 0), -EINVAL);

    expect("uts_as_ipc_einval", syscall2(SYS_setns, uts, CLONE_NEWIPC), -EINVAL);
    expect("uts_as_net_einval", syscall2(SYS_setns, uts, CLONE_NEWNET), -EINVAL);
    expect("uts_as_mnt_einval", syscall2(SYS_setns, uts, CLONE_NEWNS), -EINVAL);
    expect("uts_two_types_einval", syscall2(SYS_setns, uts, CLONE_NEWUTS | CLONE_NEWIPC), -EINVAL);
    expect("uts_bit0_einval", syscall2(SYS_setns, uts, 1), -EINVAL);
    expect("uts_vm_einval", syscall2(SYS_setns, uts, CLONE_VM), -EINVAL);
    expect("uts_minus1_einval", syscall2(SYS_setns, uts, -1), -EINVAL);
    expect("uts_ipc_high_bits_einval", syscall2(SYS_setns, uts, (1L << 32) | CLONE_NEWIPC), -EINVAL);

    i64 user = a_open("/proc/self/ns/user", O_RDONLY, 0);
    expect("own_userns_einval", syscall2(SYS_setns, user, 0), -EINVAL);
    expect("own_userns_typed_einval", syscall2(SYS_setns, user, CLONE_NEWUSER), -EINVAL);
    expect("user_as_uts_einval", syscall2(SYS_setns, user, CLONE_NEWUTS), -EINVAL);
    return finish();
}
