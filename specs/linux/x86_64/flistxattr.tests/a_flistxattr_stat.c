/* flistxattr changes nothing: mode, size, link count, atime, mtime and
 * ctime of the file (fstat through the descriptor) and of its directory
 * are the same after successful, size-0, ERANGE and EFAULT calls; the
 * contents and the file offset are unchanged. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_flistxattr 196
#define D "/tmp/a_flistxattr_stat"
#define F D "/f"

static char buf[64];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "abc", 0640), 0);
    expect("setup_set", syscall5(SYS_setxattr, F, "user.s", "1", 1, 0), 0);
    i64 fd = a_open(F, O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("setup_seek", syscall3(SYS_lseek, fd, 2, SEEK_SET), 2);
    struct a_stat a, b, c, d;
    expect("setup_fstat_before", syscall2(SYS_fstat, fd, &a), 0);
    expect("setup_stat_dir_before", a_stat_path(D, &c), 0);
    expect("list", syscall3(SYS_flistxattr, fd, buf, sizeof buf), 7);
    expect("list_size0", syscall3(SYS_flistxattr, fd, 0, 0), 7);
    expect("list_erange", syscall3(SYS_flistxattr, fd, buf, 1), -ERANGE);
    expect("list_efault", syscall3(SYS_flistxattr, fd, (void *)8, 64), -EFAULT);
    expect("setup_fstat_after", syscall2(SYS_fstat, fd, &b), 0);
    expect("setup_stat_dir_after", a_stat_path(D, &d), 0);
    check("mode_same", a.st_mode == b.st_mode, b.st_mode);
    check("size_same", a.st_size == b.st_size, b.st_size);
    check("nlink_same", a.st_nlink == b.st_nlink, b.st_nlink);
    check("atime_same", a.st_atime_sec == b.st_atime_sec && a.st_atime_nsec == b.st_atime_nsec, 0);
    check("mtime_same", a.st_mtime_sec == b.st_mtime_sec && a.st_mtime_nsec == b.st_mtime_nsec, 0);
    check("ctime_same", a.st_ctime_sec == b.st_ctime_sec && a.st_ctime_nsec == b.st_ctime_nsec, 0);
    check("dir_mtime_same", c.st_mtime_sec == d.st_mtime_sec && c.st_mtime_nsec == d.st_mtime_nsec, 0);
    check("dir_ctime_same", c.st_ctime_sec == d.st_ctime_sec && c.st_ctime_nsec == d.st_ctime_nsec, 0);
    expect("offset_same", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 2);
    char text[8];
    expect("setup_rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("contents_length", a_read(fd, text, sizeof text), 3);
    check("contents_same", mem_eq(text, "abc", 3), 0);
    a_close(fd);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
