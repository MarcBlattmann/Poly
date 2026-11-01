/* inotify_rm_watch errors: a descriptor that is not open is EBADF
 * (checked first, whatever wd is); an open descriptor that is not an
 * inotify instance is EINVAL; a wd the instance does not have (0, -1, a
 * number never handed out, one of another instance, one already
 * removed) is EINVAL. wd is a 32-bit int: the upper half of the
 * register is ignored.
 */
#include "a_sys.h"

#define SYS_inotify_add_watch 254
#define SYS_inotify_rm_watch 255
#define SYS_inotify_init1 294
/* linux/inotify.h */
#define IN_CREATE 0x100

#define DIR "/tmp/la_inotify_rm_watch_errors"

static i64 rm(i64 fd, i64 wd) { return syscall2(SYS_inotify_rm_watch, fd, wd); }

int test_main(void) {
    a_unlink(DIR "/f");
    a_rmdir(DIR);
    a_mkdir(DIR, 0755);
    a_make_file(DIR "/f", "", 0644);
    i64 fd = syscall1(SYS_inotify_init1, 0);
    i64 other = syscall1(SYS_inotify_init1, 0);
    i64 reg = a_open(DIR "/f", O_RDONLY, 0);
    expect("fds", fd * 100 + other * 10 + reg, 345);
    expect("watch_1", syscall3(SYS_inotify_add_watch, fd, DIR, IN_CREATE), 1);
    expect("watch_2", syscall3(SYS_inotify_add_watch, fd, DIR "/f", IN_CREATE), 2);

    expect("bad_fd_ebadf", rm(999, 1), -EBADF);
    expect("negative_fd_ebadf", rm(-1, 1), -EBADF);
    expect("bad_fd_bad_wd_ebadf", rm(999, -1), -EBADF);
    expect("not_inotify_einval", rm(reg, 1), -EINVAL);
    expect("wd_0_einval", rm(fd, 0), -EINVAL);
    expect("wd_minus1_einval", rm(fd, -1), -EINVAL);
    expect("wd_unknown_einval", rm(fd, 3), -EINVAL);
    expect("wd_of_other_instance_einval", rm(other, 1), -EINVAL);
    expect("wd_upper_bits_ignored", rm(fd, (1L << 32) | 2), 0);
    expect("wd_removed_einval", rm(fd, 2), -EINVAL);
    expect("wd_1_still_there", rm(fd, 1), 0);
    a_unlink(DIR "/f");
    a_rmdir(DIR);
    return finish();
}
