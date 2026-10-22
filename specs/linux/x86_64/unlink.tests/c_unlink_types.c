/* unlink removes any kind of non-directory: a FIFO, a bound UNIX socket's
 * file (the socket itself is unaffected), and an empty file. */
#include "a_sys.h"

#define D "/tmp/c_unlink_types"
#define SYS_socket 41
#define SYS_bind 49
#define SYS_getsockname 51
#define SYS_mknodat 259
#define AF_UNIX 1
#define SOCK_STREAM 1

static i64 u(const char *p) { return syscall1(SYS_unlink, p); }

/* sys/un.h: struct sockaddr_un */
struct sun {
    u16 family;
    char path[108];
};

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("mkfifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    expect("fifo_type", a_lstat_path(D "/fifo", &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFIFO);
    expect("unlink_fifo", u(D "/fifo"), 0);
    expect("fifo_gone", a_lstat_path(D "/fifo", &st), -ENOENT);

    i64 s = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    expect("socket_ok", s >= 0, 1);
    struct sun addr;
    memset(&addr, 0, sizeof addr);
    addr.family = AF_UNIX;
    str_cpy(addr.path, D "/sock");
    expect("bind_ok", syscall3(SYS_bind, s, &addr, sizeof addr), 0);
    expect("sock_type", a_lstat_path(D "/sock", &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFSOCK);
    expect("unlink_sock", u(D "/sock"), 0);
    expect("sock_gone", a_lstat_path(D "/sock", &st), -ENOENT);
    struct sun got;
    i32 len = sizeof got;
    memset(&got, 0, sizeof got);
    expect("getsockname_after", syscall3(SYS_getsockname, s, &got, &len), 0);
    expect("name_still_bound", str_eq(got.path, D "/sock"), 1);
    expect("rebind_same_path", syscall3(SYS_bind, syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0), &addr, sizeof addr),
           0);
    expect("unlink_sock_again", u(D "/sock"), 0);

    expect("empty_file", a_make_file(D "/empty", "", 0), 0);
    expect("unlink_mode_0_file", u(D "/empty"), 0);
    expect("cleanup_dir", syscall1(SYS_rmdir, D), 0);
    return finish();
}
