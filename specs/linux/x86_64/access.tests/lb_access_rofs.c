/* access(2) with W_OK on a read-only mount: EROFS for regular files,
 * directories and symbolic links to them, before the permission bits are
 * looked at; R_OK, X_OK and F_OK answer as usual; a device node is
 * writable (the device is not on the file system).
 * poly-diff: needs=root (mounts a tmpfs and remounts it read-only)
 */
#include "a_sys.h"

#define D "/tmp/lb_access_rofs"
#define SYS_mount 165
#define SYS_umount2 166
#define SYS_mknodat 259
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 acc(const char *p, i64 mode) { return syscall2(SYS_access, p, mode); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("mount_tmpfs", syscall5(SYS_mount, "none", D, "tmpfs", 0, "mode=0755"), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_ro_file", a_make_file(D "/ro", "x", 0444), 0);
    expect("setup_exe", a_make_file(D "/exe", "x", 0755), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("setup_null", syscall4(SYS_mknodat, AT_FDCWD, D "/null", S_IFCHR | 0666, (1 << 8) | 3), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0666, 0), 0);
    expect("rw_file_w", acc(D "/f", W_OK), 0);

    expect("remount_ro", syscall5(SYS_mount, "none", D, "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("ro_file_w_erofs", acc(D "/f", W_OK), -EROFS);
    expect("ro_file_rw_erofs", acc(D "/f", R_OK | W_OK), -EROFS);
    expect("ro_file_r", acc(D "/f", R_OK), 0);
    expect("ro_file_fok", acc(D "/f", F_OK), 0);
    expect("ro_mode444_w_erofs", acc(D "/ro", W_OK), -EROFS);
    expect("ro_exe_x", acc(D "/exe", X_OK), 0);
    expect("ro_exe_wx_erofs", acc(D "/exe", W_OK | X_OK), -EROFS);
    expect("ro_file_noexec_wx", acc(D "/f", W_OK | X_OK), -EROFS);
    expect("ro_dir_w_erofs", acc(D "/d", W_OK), -EROFS);
    expect("ro_mountpoint_w_erofs", acc(D, W_OK), -EROFS);
    expect("ro_link_w_erofs", acc(D "/l", W_OK), -EROFS);
    expect("ro_chardev_w", acc(D "/null", W_OK), 0);
    expect("ro_fifo_w", acc(D "/fifo", W_OK), 0);
    expect("ro_missing_enoent", acc(D "/nope", W_OK), -ENOENT);

    expect("remount_rw", syscall5(SYS_mount, "none", D, "tmpfs", MS_REMOUNT, 0), 0);
    expect("rw_again", acc(D "/f", W_OK), 0);
    expect("umount", syscall2(SYS_umount2, D, 0), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
