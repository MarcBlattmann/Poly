/* syncfs(2) commits the data of the file system that contains the file
 * open on fd and returns 0; any access mode works (read-only, write-only,
 * a directory); nothing visible changes. */
#include "a_sys.h"

#define D "/tmp/lb_syncfs_basic"
#define SYS_syncfs 306

static A_UNUSED i64 syncfs_(i64 fd) { return syscall1(SYS_syncfs, fd); }

int test_main(void) {
    struct a_stat st, st2;
    char c[8];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT, 0644);
    expect("write", a_write(fd, "fsdata", 6), 6);
    expect("stat0", syscall2(SYS_fstat, fd, &st), 0);
    expect("rdwr_fd", syscall1(SYS_syncfs, fd), 0);
    expect("stat1", syscall2(SYS_fstat, fd, &st2), 0);
    expect("stat_identical", mem_eq(&st, &st2, sizeof st), 1);
    expect("offset_kept", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 6);
    expect("contents", syscall4(SYS_pread64, fd, c, 8, 0) == 6 && mem_eq(c, "fsdata", 6), 1);
    i64 ro = a_open(D "/f", O_RDONLY, 0);
    expect("rdonly_fd", syscall1(SYS_syncfs, ro), 0);
    i64 wo = a_open(D "/f", O_WRONLY, 0);
    expect("wronly_fd", syscall1(SYS_syncfs, wo), 0);
    i64 dir = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("dir_fd", syscall1(SYS_syncfs, dir), 0);
    i64 root = a_open("/", O_RDONLY | O_DIRECTORY, 0);
    expect("root_fd", syscall1(SYS_syncfs, root), 0);
    expect("unlink", a_unlink(D "/f"), 0);
    expect("unlinked_file", syscall1(SYS_syncfs, fd), 0);
    expect("upper_half_of_fd_ignored", syscall1(SYS_syncfs, (i64)0x100000000L | fd), 0);
    a_close(root);
    a_close(dir);
    a_close(wo);
    a_close(ro);
    a_close(fd);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
