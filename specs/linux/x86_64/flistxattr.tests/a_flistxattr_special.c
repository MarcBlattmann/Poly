/* Descriptors that are not plain files: both pipe ends, an eventfd and an
 * epoll descriptor list nothing (0), also with size 0 and with an unmapped
 * buffer. An opened FIFO and /dev/null list nothing (they are filesystem
 * nodes without names). A memfd is a regular file: empty until fsetxattr
 * gives it a user.* name, which it then lists. */
#include "a_sys.h"

#define SYS_mknodat 259
#define SYS_eventfd2 290
#define SYS_epoll_create1 291
#define SYS_memfd_create 319
#define SYS_fsetxattr 190
#define SYS_flistxattr 196
#define D "/tmp/a_flistxattr_special"

static char buf[64];

static i64 flist(i64 fd) { return syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    int p[2];
    expect("setup_pipe", syscall2(SYS_pipe2, p, 0), 0);
    expect("pipe_read_end", flist(p[0]), 0);
    expect("pipe_write_end", flist(p[1]), 0);
    expect("pipe_size0", syscall3(SYS_flistxattr, p[0], 0, 0), 0);
    expect("pipe_bad_buffer", syscall3(SYS_flistxattr, p[1], (void *)8, 100), 0);
    i64 ev = syscall2(SYS_eventfd2, 0, 0);
    check("setup_eventfd", ev >= 0, ev);
    expect("eventfd", flist(ev), 0);
    i64 ep = syscall1(SYS_epoll_create1, 0);
    check("setup_epoll", ep >= 0, ep);
    expect("epoll", flist(ep), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0644, 0), 0);
    i64 ff = a_open(D "/p", O_RDWR, 0);
    check("setup_open_fifo", ff >= 0, ff);
    expect("fifo", flist(ff), 0);
    i64 dn = a_open("/dev/null", O_RDWR, 0);
    check("setup_dev_null", dn >= 0, dn);
    expect("dev_null", flist(dn), 0);
    i64 mf = syscall2(SYS_memfd_create, "m", 0);
    check("setup_memfd", mf >= 0, mf);
    expect("memfd_empty", flist(mf), 0);
    expect("setup_set_memfd", syscall5(SYS_fsetxattr, mf, "user.m", "x", 1, 0), 0);
    expect("memfd_one", flist(mf), 7);
    check("memfd_bytes", mem_eq(buf, "user.m\0", 7), 0);
    a_close(mf);
    a_close(dn);
    a_close(ff);
    a_close(ep);
    a_close(ev);
    a_close(p[0]);
    a_close(p[1]);
    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
