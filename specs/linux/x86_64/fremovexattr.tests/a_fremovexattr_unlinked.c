/* Attributes belong to the inode, so fremovexattr works on files that no
 * longer have a name: a regular file unlinked while open, an O_TMPFILE
 * file, and a directory removed while open. In each case removing an
 * existing user.* attribute is 0, the next removal ENODATA, and other
 * attributes stay. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define SYS_fremovexattr 199
#define ENODATA 61
#define D "/tmp/a_fremovexattr_unlinked"

static char buf[16];

static i64 frm(i64 fd, const char *n) { return syscall2(SYS_fremovexattr, fd, n); }
static i64 fset(i64 fd, const char *n) { return syscall5(SYS_fsetxattr, fd, n, "v", 1, 0); }

static void run(const char *label, i64 fd) {
    check(label, fset(fd, "user.x") == 0, 0);
    check(label, fset(fd, "user.y") == 0, 0);
    check(label, frm(fd, "user.x") == 0, 0);
    check(label, frm(fd, "user.x") == -ENODATA, 0);
    check(label, syscall4(SYS_fgetxattr, fd, "user.y", buf, sizeof buf) == 1, 0);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/u", "", 0644), 0);
    i64 uf = a_open(D "/u", O_RDONLY, 0);
    check("setup_open", uf >= 0, uf);
    expect("unlink", a_unlink(D "/u"), 0);
    run("unlinked_file", uf);
    i64 tf = a_open(D, O_TMPFILE | O_RDWR, 0600);
    check("setup_tmpfile", tf >= 0, tf);
    run("tmpfile", tf);
    expect("setup_subdir", a_mkdir(D "/rd", 0755), 0);
    i64 rd = a_open(D "/rd", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_dir", rd >= 0, rd);
    expect("rmdir", a_rmdir(D "/rd"), 0);
    run("removed_dir", rd);
    a_close(uf);
    a_close(tf);
    a_close(rd);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
