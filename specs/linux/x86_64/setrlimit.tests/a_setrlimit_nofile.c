/* setrlimit RLIMIT_NOFILE: new descriptors must be below the soft limit
 * (EMFILE otherwise, for open, dup and pipe2); raising it again lets them
 * through. */
#include "a_sys.h"

int test_main(void) {
    struct a_rlimit old, l;
    syscall2(SYS_getrlimit, RLIMIT_NOFILE, &old);
    l.rlim_cur = 5;
    l.rlim_max = old.rlim_max;
    expect("set_soft_5", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &l), 0);
    expect("fd_3", a_open("/", O_RDONLY, 0), 3);
    expect("fd_4", a_open("/", O_RDONLY, 0), 4);
    expect("open_emfile", a_open("/", O_RDONLY, 0), -EMFILE);
    expect("dup_emfile", syscall1(SYS_dup, 0), -EMFILE);
    i32 p[2];
    expect("pipe_emfile", syscall2(SYS_pipe2, p, 0), -EMFILE);
    l.rlim_cur = 7;
    expect("raise_soft_7", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &l), 0);
    expect("pipe_now_fits", syscall2(SYS_pipe2, p, 0), 0);
    check("pipe_fds_5_6", p[0] == 5 && p[1] == 6, p[0]);
    for (int fd = 3; fd < 7; fd++) a_close(fd);
    expect("restore", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &old), 0);
    return finish();
}
