/* inotify_init1 takes the lowest free descriptor and fails with EMFILE
 * when every descriptor below RLIMIT_NOFILE is in use; bad flags are
 * reported first (EINVAL even when the table is full). A freed slot is
 * reused. inotify_init behaves the same.
 */
#include "a_sys.h"

#define SYS_inotify_init 253
#define SYS_inotify_init1 294

int test_main(void) {
    struct a_rlimit rl = { 6, 6 };
    expect("setrlimit", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &rl), 0);
    expect("fd3", syscall1(SYS_inotify_init1, 0), 3);
    expect("fd4", syscall1(SYS_inotify_init1, O_CLOEXEC), 4);
    expect("fd5", syscall0(SYS_inotify_init), 5);
    expect("full_emfile", syscall1(SYS_inotify_init1, 0), -EMFILE);
    expect("full_init_emfile", syscall0(SYS_inotify_init), -EMFILE);
    expect("full_bad_flags_einval", syscall1(SYS_inotify_init1, 1), -EINVAL);
    a_close(4);
    expect("slot_reused", syscall1(SYS_inotify_init1, O_NONBLOCK), 4);
    expect("reused_nonblock", syscall2(SYS_fcntl, 4, F_GETFL), O_RDONLY | O_NONBLOCK);
    expect("reused_no_cloexec", syscall2(SYS_fcntl, 4, F_GETFD), 0);
    return finish();
}
