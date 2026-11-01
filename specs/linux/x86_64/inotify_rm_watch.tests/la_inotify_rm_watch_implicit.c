/* Watches removed by the kernel: when the watched inode goes away
 * (rmdir of a watched directory, unlink of the last name of a watched
 * file that is not open), IN_IGNORED is queued for its wd and the wd is
 * gone: inotify_rm_watch on it is then EINVAL. A watched file that is
 * unlinked while still open stays watched until the last descriptor is
 * closed (IN_ATTRIB for the link count change on unlink; IN_DELETE_SELF
 * and IN_IGNORED come at the close).
 */
#include "a_sys.h"

#define SYS_inotify_add_watch 254
#define SYS_inotify_rm_watch 255
#define SYS_inotify_init1 294
/* linux/inotify.h */
#define IN_ATTRIB 0x4
#define IN_DELETE_SELF 0x400
#define IN_IGNORED 0x8000

#define DIR "/tmp/la_inotify_rm_watch_implicit"

static char buf[1024] __attribute__((aligned(8)));

int test_main(void) {
    a_unlink(DIR "/f");
    a_unlink(DIR "/g");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
    a_mkdir(DIR, 0755);
    a_mkdir(DIR "/sub", 0755);
    a_make_file(DIR "/f", "", 0644);
    a_make_file(DIR "/g", "", 0644);
    i64 fd = syscall1(SYS_inotify_init1, O_NONBLOCK);
    expect("watch_sub", syscall3(SYS_inotify_add_watch, fd, DIR "/sub", IN_DELETE_SELF), 1);
    expect("watch_f", syscall3(SYS_inotify_add_watch, fd, DIR "/f", IN_DELETE_SELF), 2);
    expect("watch_g", syscall3(SYS_inotify_add_watch, fd, DIR "/g", IN_DELETE_SELF | IN_ATTRIB), 3);

    a_rmdir(DIR "/sub");
    i64 n = a_read(fd, buf, sizeof buf);
    expect("rmdir_two_events", n, 32);
    expect("rmdir_delete_self", *(u32 *)(buf + 4), IN_DELETE_SELF);
    expect("rmdir_ignored", *(u32 *)(buf + 20), IN_IGNORED);
    expect("rmdir_ignored_wd", *(i32 *)(buf + 16), 1);
    expect("rmdir_wd_gone", syscall2(SYS_inotify_rm_watch, fd, 1), -EINVAL);

    a_unlink(DIR "/f");
    n = a_read(fd, buf, sizeof buf);
    expect("unlink_two_events", n, 32);
    expect("unlink_delete_self", *(u32 *)(buf + 4), IN_DELETE_SELF);
    expect("unlink_ignored", *(u32 *)(buf + 20), IN_IGNORED);
    expect("unlink_wd_gone", syscall2(SYS_inotify_rm_watch, fd, 2), -EINVAL);

    i64 g = a_open(DIR "/g", O_RDONLY, 0);
    a_unlink(DIR "/g");
    n = a_read(fd, buf, sizeof buf);
    expect("open_unlink_attrib_only", n, 16);
    expect("open_unlink_attrib", *(u32 *)(buf + 4), IN_ATTRIB);
    a_close(g);
    n = a_read(fd, buf, sizeof buf);
    expect("close_two_events", n, 32);
    expect("close_delete_self", *(u32 *)(buf + 4), IN_DELETE_SELF);
    expect("close_ignored", *(u32 *)(buf + 20), IN_IGNORED);
    expect("close_wd_gone", syscall2(SYS_inotify_rm_watch, fd, 3), -EINVAL);
    a_rmdir(DIR);
    return finish();
}
