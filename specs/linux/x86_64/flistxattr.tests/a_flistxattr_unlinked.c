/* Descriptors of files without a name: an unlinked open file keeps its
 * list and can still be given names; a removed open directory keeps its
 * list; an O_TMPFILE file lists nothing until fsetxattr gives it a name. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_flistxattr 196
#define D "/tmp/a_flistxattr_unlinked"

static char buf[64];

static i64 flist(i64 fd) { return syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    i64 dd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open", fd >= 0 && dd >= 0, fd);
    expect("setup_set_f", syscall5(SYS_fsetxattr, fd, "user.u", "1", 1, 0), 0);
    expect("setup_set_dir", syscall5(SYS_fsetxattr, dd, "user.dir", "", 0, 0), 0);
    expect("setup_unlink", a_unlink(D "/f"), 0);
    expect("unlinked", flist(fd), 7);
    check("unlinked_bytes", mem_eq(buf, "user.u\0", 7), 0);
    expect("setup_set_unlinked", syscall5(SYS_fsetxattr, fd, "user.v", "", 0, 0), 0);
    expect("unlinked_two", flist(fd), 14);
    expect("unlinked_size0", syscall3(SYS_flistxattr, fd, 0, 0), 14);
    expect("setup_rmdir_sub", a_rmdir(D "/sub"), 0);
    expect("removed_dir", flist(dd), 9);
    check("removed_dir_bytes", mem_eq(buf, "user.dir\0", 9), 0);
    i64 tf = a_open(D, O_TMPFILE | O_RDWR, 0600);
    check("setup_tmpfile", tf >= 0, tf);
    expect("tmpfile_empty", flist(tf), 0);
    expect("setup_set_tmpfile", syscall5(SYS_fsetxattr, tf, "user.t", "", 0, 0), 0);
    expect("tmpfile_one", flist(tf), 7);
    check("tmpfile_bytes", mem_eq(buf, "user.t\0", 7), 0);
    a_close(tf);
    a_close(dd);
    a_close(fd);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
