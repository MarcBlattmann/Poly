/* inotify_init() (no arguments) is inotify_init1(0): the lowest free
 * descriptor, open read-only and blocking (F_GETFL O_RDONLY), without
 * FD_CLOEXEC, st_mode 0600 with no type bits, link
 * "anon_inode:inotify". Every call makes an independent instance: watch
 * descriptors are numbered from 1 in each instance, and an event is
 * queued in each instance that watches the object. A dup'd descriptor
 * shares the instance (one queue).
 */
#include "a_sys.h"

#define SYS_inotify_init 253
#define SYS_inotify_add_watch 254
#define SYS_ioctl 16
/* asm-generic/ioctls.h */
#define FIONREAD 0x541B
/* linux/inotify.h */
#define IN_CREATE 0x100

static i64 queued(i64 fd) {
    i32 n = -1;
    i64 r = syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return r < 0 ? r : n;
}

#define DIR "/tmp/la_inotify_init_basic"

int test_main(void) {
    a_unlink(DIR "/f");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
    a_mkdir(DIR, 0755);
    a_mkdir(DIR "/sub", 0755);

    i64 a = syscall0(SYS_inotify_init);
    expect("fd_3", a, 3);
    expect("getfl_rdonly_blocking", syscall2(SYS_fcntl, a, F_GETFL), O_RDONLY);
    expect("getfd_0", syscall2(SYS_fcntl, a, F_GETFD), 0);
    struct a_stat st;
    syscall2(SYS_fstat, a, &st);
    expect("mode_0600", st.st_mode, 0600);
    char link[64];
    i64 n = syscall4(SYS_readlinkat, AT_FDCWD, "/proc/self/fd/3", link, sizeof link - 1);
    link[n > 0 ? n : 0] = 0;
    check("link_text", str_eq(link, "anon_inode:inotify"), n);

    i64 b = syscall0(SYS_inotify_init);
    expect("second_fd_4", b, 4);
    expect("a_first_wd_1", syscall3(SYS_inotify_add_watch, a, DIR, IN_CREATE), 1);
    expect("a_second_wd_2", syscall3(SYS_inotify_add_watch, a, DIR "/sub", IN_CREATE), 2);
    expect("b_numbering_own_wd_1", syscall3(SYS_inotify_add_watch, b, DIR, IN_CREATE), 1);

    i64 d = syscall1(SYS_dup, a);
    expect("dup_fd_5", d, 5);
    a_make_file(DIR "/f", "", 0644);
    expect("a_queued_32", queued(a), 32);
    expect("b_queued_32", queued(b), 32);
    expect("dup_same_queue", queued(d), 32);
    char buf[64] __attribute__((aligned(8)));
    expect("read_via_dup", a_read(d, buf, sizeof buf), 32);
    expect("a_drained_by_dup", queued(a), 0);
    expect("b_unaffected", queued(b), 32);
    expect("b_read", a_read(b, buf, sizeof buf), 32);
    expect("b_event_wd_1", *(i32 *)buf, 1);

    a_unlink(DIR "/f");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
    return finish();
}
