/* Any descriptor of a regular file or directory reads its attributes,
 * whatever the access mode: O_RDONLY, O_WRONLY, O_RDWR, O_DIRECTORY,
 * O_APPEND, a dup'd descriptor, a descriptor inherited by a forked child,
 * and one opened through a symbolic link (which reads the target). */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fgetxattr_accmode"

static char buf[64];

static i64 fget(i64 fd, const char *name) { return syscall4(SYS_fgetxattr, fd, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "abc", 3, 0), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D, "user.d", "dir", 3, 0), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("rdonly", fget(fd, "user.a"), 3);
    check("rdonly_value", mem_eq(buf, "abc", 3), 0);
    i64 d2 = syscall1(SYS_dup, fd);
    check("setup_dup", d2 >= 0, d2);
    a_close(fd);
    expect("dup_after_close_of_original", fget(d2, "user.a"), 3);
    a_close(d2);
    fd = a_open(D "/f", O_WRONLY, 0);
    expect("wronly", fget(fd, "user.a"), 3);
    a_close(fd);
    fd = a_open(D "/f", O_RDWR | O_APPEND, 0);
    expect("rdwr_append", fget(fd, "user.a"), 3);
    long pid = a_fork_sync();
    if (pid == 0) {
        i64 r = syscall4(SYS_fgetxattr, fd, "user.a", buf, sizeof buf);
        syscall1(SYS_exit, r == 3 && mem_eq(buf, "abc", 3) ? 0 : 1);
    }
    check("setup_fork", pid > 0, pid);
    int status = -1;
    expect("wait_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_read_inherited", status, 0);
    a_close(fd);
    fd = a_open(D "/l", O_RDONLY, 0);
    expect("through_symlink_reads_target", fget(fd, "user.a"), 3);
    a_close(fd);
    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("directory", fget(fd, "user.d"), 3);
    check("directory_value", mem_eq(buf, "dir", 3), 0);
    expect("directory_missing", fget(fd, "user.a"), -ENODATA);
    a_close(fd);
    fd = a_open(D, O_RDONLY, 0);
    expect("directory_plain_rdonly", fget(fd, "user.d"), 3);
    a_close(fd);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
