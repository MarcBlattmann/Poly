/* llistxattr on other file types: a FIFO, /dev/null and a bound unix
 * socket's path have no visible names (0). The /proc/self/fd/N entries are
 * symbolic links themselves, so llistxattr lists the link (0) for a file
 * with names, a directory and a socket, while listxattr follows them (the
 * file's names; a socket lists "system.sockprotoname"). A trailing slash
 * on the fd link of a directory follows it. /proc/self/exe and
 * /proc/self/cwd list as empty links too. */
#include "a_sys.h"

#define SYS_socket 41
#define SYS_bind 49
#define SYS_mknodat 259
#define SYS_setxattr 188
#define SYS_listxattr 194
#define SYS_llistxattr 195
#define D "/tmp/a_llistxattr_special"

static char buf[256];
static char p[64];

static i64 llist(const char *path) { return syscall3(SYS_llistxattr, path, buf, sizeof buf); }

static const char *fd_path(i64 fd, int slash) {
    a_utoa(str_cpy(p, "/proc/self/fd/"), fd);
    if (slash) str_cpy(p + str_len(p), "/");
    return p;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set_file", syscall5(SYS_setxattr, D "/f", "user.ab", "x", 1, 0), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D, "user.dd", "x", 1, 0), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", 0010644, 0), 0);
    expect("fifo", llist(D "/fifo"), 0);
    expect("dev_null", llist("/dev/null"), 0);
    i64 s = syscall3(SYS_socket, 1, 1, 0);
    check("setup_socket", s >= 0, s);
    struct { u16 family; char path[108]; } addr = { 1, D "/sock" };
    expect("setup_bind", syscall3(SYS_bind, s, &addr, sizeof addr), 0);
    expect("bound_path", llist(D "/sock"), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open_file", fd >= 0, fd);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_dir", dfd >= 0, dfd);
    expect("proc_fd_file", llist(fd_path(fd, 0)), 0);
    expect("proc_fd_file_listxattr", syscall3(SYS_listxattr, fd_path(fd, 0), buf, sizeof buf), 8);
    check("proc_fd_file_bytes", mem_eq(buf, "user.ab\0", 8), 0);
    expect("proc_fd_dir", llist(fd_path(dfd, 0)), 0);
    expect("proc_fd_dir_slash", llist(fd_path(dfd, 1)), 8);
    check("proc_fd_dir_slash_bytes", mem_eq(buf, "user.dd\0", 8), 0);
    expect("proc_fd_socket", llist(fd_path(s, 0)), 0);
    expect("proc_fd_socket_listxattr", syscall3(SYS_listxattr, fd_path(s, 0), buf, sizeof buf), 21);
    check("proc_fd_socket_name", mem_eq(buf, "system.sockprotoname\0", 21), 0);
    expect("proc_self_exe", llist("/proc/self/exe"), 0);
    expect("proc_self_cwd", llist("/proc/self/cwd"), 0);
    expect("close_file", a_close(fd), 0);
    expect("close_dir", a_close(dfd), 0);
    expect("close_socket", a_close(s), 0);
    expect("cleanup_sock", a_unlink(D "/sock"), 0);
    expect("cleanup_fifo", a_unlink(D "/fifo"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
