/* Permission: watching an inode needs read permission on it (and search
 * permission on the directories of the path). For a caller without
 * CAP_DAC_OVERRIDE / CAP_DAC_READ_SEARCH, a file of mode 0200 or 0000 is
 * EACCES, mode 0400 works, and a path through a directory of mode 0600
 * (no x) is EACCES. Root passes these checks. The checks come after the
 * mask and descriptor checks.
 */
#include "a_sys.h"

#define SYS_inotify_add_watch 254
#define SYS_inotify_init1 294
/* linux/inotify.h */
#define IN_MODIFY 0x2

#define DIR "/tmp/la_inotify_add_watch_access"

static i64 add(i64 fd, const char *path, u64 mask) { return syscall3(SYS_inotify_add_watch, fd, path, mask); }

int test_main(void) {
    a_unlink(DIR "/w");
    a_unlink(DIR "/n");
    a_unlink(DIR "/r");
    a_unlink(DIR "/closed/f");
    a_rmdir(DIR "/closed");
    a_rmdir(DIR);
    a_mkdir(DIR, 0755);
    a_make_file(DIR "/w", "", 0200);
    a_make_file(DIR "/n", "", 0000);
    a_make_file(DIR "/r", "", 0400);
    a_mkdir(DIR "/closed", 0755);
    a_make_file(DIR "/closed/f", "", 0644);
    syscall4(SYS_fchmodat, AT_FDCWD, DIR "/closed", 0600, 0);

    int root = syscall0(SYS_geteuid) == 0;
    i64 fd = syscall1(SYS_inotify_init1, 0);
    expect("readable_ok", add(fd, DIR "/r", IN_MODIFY), 1);
    expect("write_only_file", add(fd, DIR "/w", IN_MODIFY), root ? 2 : -EACCES);
    expect("mode_0_file", add(fd, DIR "/n", IN_MODIFY), root ? 3 : -EACCES);
    expect("unsearchable_dir", add(fd, DIR "/closed/f", IN_MODIFY), root ? 4 : -EACCES);
    expect("mask_checked_before_access", add(fd, DIR "/n", 0), -EINVAL);
    expect("fd_checked_before_access", add(999, DIR "/n", IN_MODIFY), -EBADF);

    syscall4(SYS_fchmodat, AT_FDCWD, DIR "/closed", 0755, 0);
    a_unlink(DIR "/w");
    a_unlink(DIR "/n");
    a_unlink(DIR "/r");
    a_unlink(DIR "/closed/f");
    a_rmdir(DIR "/closed");
    a_rmdir(DIR);
    return finish();
}
