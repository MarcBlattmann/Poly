/* syncfs(2) on files of a separately mounted tmpfs, before and after it
 * became read-only, and on a descriptor that outlived a lazy unmount:
 * always 0.
 * poly-diff: needs=root (mounts tmpfs instances)
 */
#include "a_sys.h"

#define D "/tmp/lb_syncfs_mounts"
#define SYS_syncfs 306
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32
#define MNT_DETACH 2

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D, "tmpfs", 0, "mode=0755"), 0);
    expect("setup_f", a_make_file(D "/f", "abc", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    i64 dir = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("mounted_file", syscall1(SYS_syncfs, fd), 0);
    expect("mount_root_dir", syscall1(SYS_syncfs, dir), 0);
    expect("remount_ro", syscall5(SYS_mount, "none", D, "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("read_only", syscall1(SYS_syncfs, fd), 0);
    expect("lazy_umount", syscall2(SYS_umount2, D, MNT_DETACH), 0);
    expect("after_detach", syscall1(SYS_syncfs, fd), 0);
    expect("dir_after_detach", syscall1(SYS_syncfs, dir), 0);
    a_close(dir);
    a_close(fd);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
