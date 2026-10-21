/* umount2: a mount with an open file in it is EBUSY (and stays mounted);
 * MNT_DETACH detaches it anyway (the path shows the directory below at
 * once, the open file still works); a path that is not a mount point is
 * EINVAL; a missing path ENOENT; unknown flags EINVAL; MNT_EXPIRE with
 * MNT_DETACH is EINVAL; with UMOUNT_NOFOLLOW a symlink to the mount point
 * is not followed, and the link itself is not a mount point (EINVAL). */
#include "poly_ipc.h"

#define D "/tmp/b_umount2"
#define L "/tmp/b_umount2_link"

int test_main(void) {
    syscall2(SYS_umount2, D, MNT_DETACH);
    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    syscall5(SYS_mount, "x", D, "tmpfs", 0, 0);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, D "/open", O_CREAT | O_RDWR, 0644);
    i64 r = syscall2(SYS_umount2, D, 0);
    check("open_file_ebusy", r == -EBUSY, r);
    i64 again = syscall4(SYS_openat, AT_FDCWD, D "/open", O_RDONLY, 0);
    check("still_mounted_after_ebusy", again >= 0, again);
    close((int)again);

    r = syscall2(SYS_umount2, D, 0x10);
    check("unknown_flag_einval", r == -EINVAL, r);
    r = syscall2(SYS_umount2, D, MNT_EXPIRE | MNT_DETACH);
    check("expire_with_detach_einval", r == -EINVAL, r);
    syscall3(SYS_symlinkat, D, AT_FDCWD, L);
    r = syscall2(SYS_umount2, L, UMOUNT_NOFOLLOW);
    check("nofollow_symlink_einval", r == -EINVAL, r);

    r = syscall2(SYS_umount2, D, MNT_DETACH);
    check("detach_busy_ok", r == 0, r);
    i64 gone = syscall4(SYS_openat, AT_FDCWD, D "/open", O_RDONLY, 0);
    check("path_shows_directory_below", gone == -ENOENT, gone);
    r = write((int)fd, "still", 5);
    check("open_file_still_works", r == 5, r);
    close((int)fd);

    r = syscall2(SYS_umount2, D, 0);
    check("not_a_mount_point_einval", r == -EINVAL, r);
    r = syscall2(SYS_umount2, "/tmp/b_umount2_missing", 0);
    check("missing_enoent", r == -ENOENT, r);
    r = syscall2(SYS_umount2, 8, 0);
    check("bad_pointer_efault", r == -EFAULT, r);
    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
