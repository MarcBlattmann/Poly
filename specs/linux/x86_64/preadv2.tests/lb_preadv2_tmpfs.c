/* preadv2(2) on a tmpfs file: RWF_NOWAIT is EOPNOTSUPP (tmpfs files do
 * not support non-blocking buffered reads), the other read flags work.
 * poly-diff: needs=root (mounts a tmpfs, to be independent of what /tmp is)
 */
#include "a_sys.h"

#define D "/tmp/lb_preadv2_tmpfs"
#define SYS_preadv2 327
#define SYS_mount 165
#define SYS_umount2 166

static i64 preadv2_(i64 fd, struct a_iovec *iov, i64 cnt, i64 lo, i64 flags) {
    return syscall6(SYS_preadv2, fd, (i64)iov, cnt, lo, 0, flags);
}

int test_main(void) {
    char a[8];
    struct a_iovec v[1] = {{a, 4}};
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D, "tmpfs", 0, 0), 0);
    expect("setup_f", a_make_file(D "/f", "tmpfsdat", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("nowait_eopnotsupp", preadv2_(fd, v, 1, 0, 8), -EOPNOTSUPP);
    expect("nowait_minus1_eopnotsupp", preadv2_(fd, v, 1, -1, 8), -EOPNOTSUPP);
    expect("position_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 0);
    expect("dsync_ok", preadv2_(fd, v, 1, 0, 2) == 4 && mem_eq(a, "tmpf", 4), 1);
    expect("hipri_ok", preadv2_(fd, v, 1, 4, 1) == 4 && mem_eq(a, "sdat", 4), 1);
    a_close(fd);
    expect("umount", syscall2(SYS_umount2, D, 0), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
