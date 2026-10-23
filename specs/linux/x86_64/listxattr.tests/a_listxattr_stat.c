/* listxattr family: listing changes nothing. Mode, size, link count,
 * atime, mtime and ctime are the same after successful, size-0 and ERANGE
 * calls through all three entry points; the contents are unchanged. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_listxattr 194
#define SYS_llistxattr 195
#define SYS_flistxattr 196
#define D "/tmp/a_listxattr_stat"
#define F D "/f"

static char buf[256];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "abc", 0640), 0);
    expect("setup_set", syscall5(SYS_setxattr, F, "user.s", "1", 1, 0), 0);
    i64 fd = a_open(F, O_WRONLY, 0);
    check("setup_open", fd >= 0, fd);
    struct a_stat a, b, c, d;
    expect("setup_stat_before", a_stat_path(F, &a), 0);
    expect("setup_stat_dir_before", a_stat_path(D, &c), 0);
    expect("list", syscall3(SYS_listxattr, F, buf, sizeof buf), 7);
    expect("list_size0", syscall3(SYS_listxattr, F, 0, 0), 7);
    expect("list_erange", syscall3(SYS_listxattr, F, buf, 1), -34);
    expect("llist", syscall3(SYS_llistxattr, F, buf, sizeof buf), 7);
    expect("flist", syscall3(SYS_flistxattr, fd, buf, sizeof buf), 7);
    expect("list_dir", syscall3(SYS_listxattr, D, buf, sizeof buf), 0);
    expect("setup_stat_after", a_stat_path(F, &b), 0);
    expect("setup_stat_dir_after", a_stat_path(D, &d), 0);
    check("mode_same", a.st_mode == b.st_mode, b.st_mode);
    check("size_same", a.st_size == b.st_size, b.st_size);
    check("nlink_same", a.st_nlink == b.st_nlink, b.st_nlink);
    check("atime_same", a.st_atime_sec == b.st_atime_sec && a.st_atime_nsec == b.st_atime_nsec, 0);
    check("mtime_same", a.st_mtime_sec == b.st_mtime_sec && a.st_mtime_nsec == b.st_mtime_nsec, 0);
    check("ctime_same", a.st_ctime_sec == b.st_ctime_sec && a.st_ctime_nsec == b.st_ctime_nsec, 0);
    check("dir_atime_same", c.st_atime_sec == d.st_atime_sec && c.st_atime_nsec == d.st_atime_nsec, 0);
    check("dir_ctime_same", c.st_ctime_sec == d.st_ctime_sec && c.st_ctime_nsec == d.st_ctime_nsec, 0);
    a_close(fd);
    char text[8];
    i64 rd = a_open(F, O_RDONLY, 0);
    expect("contents_length", a_read(rd, text, sizeof text), 3);
    check("contents_same", mem_eq(text, "abc", 3), 0);
    a_close(rd);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
