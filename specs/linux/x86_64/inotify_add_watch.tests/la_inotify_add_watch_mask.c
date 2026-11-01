/* The mask of a watch: only events in the mask are queued. Adding a
 * watch for an inode already watched replaces its mask; with
 * IN_MASK_ADD the new bits are added to the old ones. A mask holding
 * only bits that are never generated for it (IN_IGNORED, IN_ISDIR,
 * IN_UNMOUNT) is accepted and reports nothing. IN_ONESHOT: the watch
 * reports one event, then IN_IGNORED, and is gone (inotify_rm_watch on
 * it is EINVAL). Events on a watched file have no name (len 0).
 */
#include "a_sys.h"

#define SYS_ioctl 16
#define SYS_inotify_add_watch 254
#define SYS_inotify_rm_watch 255
#define SYS_inotify_init1 294
/* asm-generic/ioctls.h */
#define FIONREAD 0x541B
/* linux/inotify.h */
#define IN_MODIFY 0x2
#define IN_ATTRIB 0x4
#define IN_CREATE 0x100
#define IN_DELETE 0x200
#define IN_UNMOUNT 0x2000
#define IN_IGNORED 0x8000
#define IN_MASK_ADD 0x20000000
#define IN_ISDIR 0x40000000
#define IN_ONESHOT 0x80000000

#define DIR "/tmp/la_inotify_add_watch_mask"

static i64 add(i64 fd, const char *path, u64 mask) { return syscall3(SYS_inotify_add_watch, fd, path, mask); }
static char buf[1024] __attribute__((aligned(8)));

/* Create and remove DIR/f; return the bytes queued and drain them. */
static i64 churn(i64 fd) {
    a_make_file(DIR "/f", "", 0644);
    a_unlink(DIR "/f");
    i32 q = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &q);
    while (a_read(fd, buf, sizeof buf) > 0) {
    }
    return q;
}

int test_main(void) {
    a_unlink(DIR "/f");
    a_unlink(DIR "/g");
    a_rmdir(DIR);
    a_mkdir(DIR, 0755);
    i64 fd = syscall1(SYS_inotify_init1, O_NONBLOCK);

    expect("watch_create", add(fd, DIR, IN_CREATE), 1);
    expect("create_only_one_event", churn(fd), 32);
    expect("replace_with_delete", add(fd, DIR, IN_DELETE), 1);
    expect("delete_only_one_event", churn(fd), 32);
    expect("mask_add_create", add(fd, DIR, IN_CREATE | IN_MASK_ADD), 1);
    expect("both_events", churn(fd), 64);
    expect("replace_with_ignored_bit", add(fd, DIR, IN_IGNORED), 1);
    expect("ignored_bit_reports_nothing", churn(fd), 0);
    expect("replace_with_isdir_bit", add(fd, DIR, IN_ISDIR), 1);
    expect("isdir_bit_reports_nothing", churn(fd), 0);
    expect("replace_with_unmount_bit", add(fd, DIR, IN_UNMOUNT), 1);
    expect("unmount_bit_reports_nothing", churn(fd), 0);

    /* One-shot. */
    expect("oneshot", add(fd, DIR, IN_CREATE | IN_DELETE | IN_ONESHOT), 1);
    a_make_file(DIR "/f", "", 0644);
    a_unlink(DIR "/f");
    i64 n = a_read(fd, buf, sizeof buf);
    expect("oneshot_bytes", n, 32 + 16);
    expect("oneshot_first_mask", *(u32 *)(buf + 4), IN_CREATE);
    expect("oneshot_ignored_wd", *(i32 *)(buf + 32), 1);
    expect("oneshot_ignored_mask", *(u32 *)(buf + 36), IN_IGNORED);
    expect("oneshot_ignored_len", *(u32 *)(buf + 44), 0);
    expect("oneshot_watch_gone", syscall2(SYS_inotify_rm_watch, fd, 1), -EINVAL);
    expect("oneshot_nothing_more", churn(fd), 0);

    /* A watched file: events without a name. */
    a_make_file(DIR "/g", "", 0644);
    expect("watch_file", add(fd, DIR "/g", IN_MODIFY | IN_ATTRIB), 2);
    syscall4(SYS_fchmodat, AT_FDCWD, DIR "/g", 0600, 0);
    i64 g = a_open(DIR "/g", O_WRONLY, 0);
    a_write(g, "z", 1);
    a_close(g);
    n = a_read(fd, buf, sizeof buf);
    expect("file_two_events", n, 32);
    expect("file_attrib", *(u32 *)(buf + 4), IN_ATTRIB);
    expect("file_attrib_len_0", *(u32 *)(buf + 12), 0);
    expect("file_modify", *(u32 *)(buf + 20), IN_MODIFY);
    expect("file_wd", *(i32 *)(buf + 16), 2);

    a_unlink(DIR "/g");
    a_rmdir(DIR);
    return finish();
}
