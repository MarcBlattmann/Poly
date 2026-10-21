/* stat: the type bits and device numbers of other kinds of files:
 * character devices, FIFOs, sockets, and links to them. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_socket 41
#define SYS_bind 49
#define SYS_mknodat 259
/* linux/socket.h, linux/net.h */
#define AF_UNIX 1
#define SOCK_STREAM 1
/* linux/un.h */
struct un_addr {
    u16 family;
    char path[108];
};

#define D "/tmp/a_stat_kinds"

static i64 st(const char *p, struct a_stat *s) { return syscall2(SYS_stat, p, s); }

int test_main(void) {
    struct a_stat s;
    a_unlink(D "/fifo");
    a_unlink(D "/sock");
    a_unlink(D "/l_null");
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);

    expect("devnull", st("/dev/null", &s), 0);
    expect("devnull_mode", s.st_mode, S_IFCHR | 0666);
    expect("devnull_rdev_1_3", s.st_rdev, (1 << 8) | 3);
    expect("devnull_size_0", s.st_size, 0);
    expect("devzero", st("/dev/zero", &s), 0);
    expect("devzero_mode", s.st_mode, S_IFCHR | 0666);
    expect("devzero_rdev_1_5", s.st_rdev, (1 << 8) | 5);

    expect("symlink_to_devnull", syscall3(SYS_symlinkat, "/dev/null", AT_FDCWD, D "/l_null"), 0);
    expect("link_to_devnull", st(D "/l_null", &s), 0);
    expect("link_to_devnull_is_chr", s.st_mode & S_IFMT, S_IFCHR);
    expect("link_to_devnull_rdev", s.st_rdev, (1 << 8) | 3);

    expect("mkfifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0640, 0), 0);
    expect("fifo", st(D "/fifo", &s), 0);
    expect("fifo_mode", s.st_mode, S_IFIFO | 0640);
    expect("fifo_size_0", s.st_size, 0);
    expect("fifo_rdev_0", s.st_rdev, 0);
    expect("fifo_nlink_1", s.st_nlink, 1);
    expect("fifo_owner", s.st_uid, syscall0(SYS_geteuid));

    i64 fd = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    check("socket", fd >= 0, fd);
    struct un_addr addr;
    memset(&addr, 0, sizeof addr);
    addr.family = AF_UNIX;
    str_cpy(addr.path, D "/sock");
    expect("bind", syscall3(SYS_bind, fd, &addr, sizeof addr), 0);
    expect("sock", st(D "/sock", &s), 0);
    expect("sock_type", s.st_mode & S_IFMT, S_IFSOCK);
    expect("sock_perm_umask", s.st_mode & 07777, 0755);
    expect("sock_size_0", s.st_size, 0);
    a_close(fd);
    expect("sock_stays_after_close", st(D "/sock", &s), 0);

    expect("cleanup_fifo", a_unlink(D "/fifo"), 0);
    expect("cleanup_sock", a_unlink(D "/sock"), 0);
    expect("cleanup_link", a_unlink(D "/l_null"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
