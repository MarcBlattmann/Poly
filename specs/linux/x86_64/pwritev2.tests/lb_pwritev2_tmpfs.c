/* pwritev2(2) on a tmpfs file: RWF_NOWAIT is EOPNOTSUPP and writes
 * nothing; RWF_DSYNC / RWF_SYNC / RWF_APPEND work; RWF_NOAPPEND writes at
 * the given offset (or the position, for -1) on an O_APPEND descriptor,
 * which stays O_APPEND; on a read-only mount the file cannot be opened for
 * writing, and a read-only descriptor is EBADF.
 * poly-diff: needs=root (mounts a tmpfs, to be independent of what /tmp is)
 */
#include "a_sys.h"

#define D "/tmp/lb_pwritev2_tmpfs"
#define SYS_pwritev2 328
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 pwritev2_(i64 fd, struct a_iovec *iov, i64 cnt, i64 lo, i64 flags) {
    return syscall6(SYS_pwritev2, fd, (i64)iov, cnt, lo, 0, flags);
}

int test_main(void) {
    struct a_stat st;
    struct a_iovec v[1] = {{"data", 4}};
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D, "tmpfs", 0, 0), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT, 0644);
    expect("nowait_eopnotsupp", pwritev2_(fd, v, 1, 0, 8), -EOPNOTSUPP);
    expect("nothing_written", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 0);
    expect("dsync", pwritev2_(fd, v, 1, 0, 2), 4);
    expect("sync", pwritev2_(fd, v, 1, 4, 4), 4);
    expect("append", pwritev2_(fd, v, 1, 0, 0x10), 4);
    expect("size_12", syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 12);
    char back[16];
    struct a_iovec xy[1] = {{"XY", 2}};
    i64 ap = a_open(D "/f", O_WRONLY | O_APPEND, 0);
    expect("noappend_on_o_append", pwritev2_(ap, xy, 1, 4, 0x20), 2);
    expect("noappend_read", syscall4(SYS_pread64, fd, back, 16, 0), 12);
    expect("noappend_at_offset", mem_eq(back, "dataXYtadata", 12), 1);
    expect("noappend_minus1", pwritev2_(ap, xy, 1, -1, 0x20), 2);
    expect("noappend_minus1_at_pos_0", syscall4(SYS_pread64, fd, back, 2, 0) == 2 && mem_eq(back, "XY", 2), 1);
    expect("noappend_minus1_pos_2", syscall3(SYS_lseek, ap, 0, SEEK_CUR), 2);
    expect("still_o_append", syscall2(SYS_fcntl, ap, F_GETFL) & O_APPEND, O_APPEND);
    expect("plain_write_appends", pwritev2_(ap, xy, 1, 0, 0) == 2 && syscall2(SYS_fstat, fd, &st) == 0 ? st.st_size : -1, 14);
    a_close(ap);
    a_close(fd);
    expect("remount_ro", syscall5(SYS_mount, "none", D, "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("open_for_write_erofs", a_open(D "/f", O_WRONLY, 0), -EROFS);
    i64 ro = a_open(D "/f", O_RDONLY, 0);
    expect("rdonly_ebadf", pwritev2_(ro, v, 1, 0, 0), -EBADF);
    a_close(ro);
    expect("umount", syscall2(SYS_umount2, D, 0), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
