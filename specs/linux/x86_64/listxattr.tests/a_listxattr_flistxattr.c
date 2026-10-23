/* flistxattr (196): lists the attributes of an open file or directory,
 * whatever the descriptor's access mode. An unlinked but open file and a
 * removed but open directory keep their list. O_PATH, closed, never opened
 * and negative descriptors, and AT_FDCWD (-100), are EBADF; the descriptor
 * is checked before the buffer. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_fsetxattr 190
#define SYS_flistxattr 196
#define EBADF 9
#define D "/tmp/a_listxattr_flistxattr"
#define BAD ((void *)8)

static char buf[256];

static i64 flist(i64 fd) { return syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.one", "1", 1, 0), 0);
    i64 rd = a_open(D "/f", O_RDONLY, 0);
    i64 wr = a_open(D "/f", O_WRONLY, 0);
    i64 rw = a_open(D "/f", O_RDWR, 0);
    i64 op = a_open(D "/f", O_PATH, 0);
    i64 dir = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("setup_open", rd >= 0 && wr >= 0 && rw >= 0 && op >= 0 && dir >= 0, rd);
    expect("rdonly", flist(rd), 9);
    check("rdonly_bytes", mem_eq(buf, "user.one\0", 9), 0);
    expect("wronly", flist(wr), 9);
    expect("rdwr", flist(rw), 9);
    expect("size0", syscall3(SYS_flistxattr, rd, 0, 0), 9);
    expect("small", syscall3(SYS_flistxattr, rd, buf, 8), -34);
    expect("bad_buffer", syscall3(SYS_flistxattr, rd, BAD, 100), -EFAULT);
    expect("dir_empty", flist(dir), 0);
    expect("setup_fset_dir", syscall5(SYS_fsetxattr, dir, "user.d", "", 0, 0), 0);
    expect("dir_after_set", flist(dir), 7);
    check("dir_bytes", mem_eq(buf, "user.d\0", 7), 0);
    expect("o_path", flist(op), -EBADF);
    expect("o_path_size0", syscall3(SYS_flistxattr, op, 0, 0), -EBADF);
    expect("never_opened", flist(999), -EBADF);
    expect("never_opened_bad_buffer", syscall3(SYS_flistxattr, 999, BAD, 100), -EBADF);
    expect("negative", flist(-1), -EBADF);
    expect("at_fdcwd", flist(-100), -EBADF);
    expect("setup_unlink", a_unlink(D "/f"), 0);
    expect("unlinked_open", flist(rd), 9);
    expect("setup_fset_unlinked", syscall5(SYS_fsetxattr, rw, "user.two", "", 0, 0), 0);
    expect("unlinked_two", flist(wr), 18);
    expect("setup_close", a_close(rd), 0);
    expect("closed", flist(rd), -EBADF);
    a_close(wr);
    a_close(rw);
    a_close(op);
    expect("setup_rmdir", a_rmdir(D), 0);
    expect("removed_dir", flist(dir), 7);
    a_close(dir);
    return finish();
}
