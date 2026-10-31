/* copy_file_range(2) between two tmpfs instances (different file
 * systems) is EXDEV; within one mounted tmpfs it copies; a destination on
 * a read-only mount cannot be opened for writing, and a read-only source
 * works.
 * poly-diff: needs=root (mounts tmpfs instances)
 */
#include "a_sys.h"

#define D "/tmp/lb_cfr_xdev"
#define SYS_copy_file_range 326
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 cfr(i64 in, i64 *oin, i64 out, i64 *oout, i64 len) {
    return syscall6(SYS_copy_file_range, in, (i64)oin, out, (i64)oout, len, 0);
}

int test_main(void) {
    char c[16];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_mkdir(D "/a", 0755), 0);
    expect("setup_b", a_mkdir(D "/b", 0755), 0);
    expect("mount_a", syscall5(SYS_mount, "none", D "/a", "tmpfs", 0, 0), 0);
    expect("mount_b", syscall5(SYS_mount, "none", D "/b", "tmpfs", 0, 0), 0);
    expect("setup_src", a_make_file(D "/a/src", "crossdata", 0644), 0);
    i64 in = a_open(D "/a/src", O_RDONLY, 0);
    i64 same = a_open(D "/a/dst", O_RDWR | O_CREAT, 0644);
    i64 other = a_open(D "/b/dst", O_RDWR | O_CREAT, 0644);
    check("opened", in >= 0 && same >= 0 && other >= 0, other);
    i64 o = 0;
    expect("cross_fs_exdev", cfr(in, &o, other, 0, 9), -EXDEV);
    expect("cross_fs_offset_kept", o, 0);
    expect("same_fs_copies", cfr(in, &o, same, 0, 9), 9);
    expect("same_fs_data", syscall4(SYS_pread64, same, c, 16, 0) == 9 && mem_eq(c, "crossdata", 9), 1);
    a_close(same);
    a_close(other);
    a_close(in);
    /* a read-only source file system is fine */
    expect("remount_a_ro", syscall5(SYS_mount, "none", D "/a", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    in = a_open(D "/a/src", O_RDONLY, 0);
    i64 dst = a_open(D "/a/dst", O_RDONLY, 0);
    expect("ro_source_ro_dest_ebadf", cfr(in, 0, dst, 0, 4), -EBADF);
    expect("ro_dest_open_for_write_erofs", a_open(D "/a/dst", O_WRONLY, 0), -EROFS);
    a_close(dst);
    a_close(in);
    expect("umount_a", syscall2(SYS_umount2, D "/a", 0), 0);
    expect("umount_b", syscall2(SYS_umount2, D "/b", 0), 0);
    expect("cleanup_a", a_rmdir(D "/a"), 0);
    expect("cleanup_b", a_rmdir(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
