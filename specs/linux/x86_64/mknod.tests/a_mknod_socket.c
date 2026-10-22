/* mknod: S_IFSOCK makes a socket node that no socket is bound to: opening
 * it fails with ENXIO (O_PATH works), connecting to it is refused. */
#include "a_sys.h"

#define SYS_mknod 133
#define SYS_socket 41
#define SYS_connect 42
#define SYS_bind 49
#define AF_UNIX 1
#define SOCK_STREAM 1
#define EADDRINUSE 98
#define ECONNREFUSED 111
#define D "/tmp/a_mknod_socket"

static i64 mk(const char *p, i64 mode, i64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

/* linux/un.h: struct sockaddr_un */
struct a_sockaddr_un {
    u16 family;
    char path[108];
};

int test_main(void) {
    struct a_stat st;
    syscall1(SYS_umask, 022);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);

    expect("mksock", mk(D "/s", S_IFSOCK | 0777, 42), 0);
    expect("stat", a_lstat_path(D "/s", &st), 0);
    expect("mode", st.st_mode, S_IFSOCK | 0755);
    expect("rdev_ignored", (i64)st.st_rdev, 0);
    expect("size", st.st_size, 0);
    expect("nlink", st.st_nlink, 1);

    expect("open_rdonly_enxio", a_open(D "/s", O_RDONLY, 0), -ENXIO);
    expect("open_wronly_enxio", a_open(D "/s", O_WRONLY, 0), -ENXIO);
    i64 pfd = a_open(D "/s", O_PATH, 0);
    check("open_opath", pfd >= 0, pfd);
    struct a_stat fst;
    expect("fstat_opath", syscall2(SYS_fstat, pfd, &fst), 0);
    expect("fstat_opath_sock", fst.st_mode & S_IFMT, S_IFSOCK);
    a_close(pfd);

    struct a_sockaddr_un sa;
    memset(&sa, 0, sizeof sa);
    sa.family = AF_UNIX;
    str_cpy(sa.path, D "/s");
    i64 sk = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    check("socket", sk >= 0, sk);
    expect("connect_refused", syscall3(SYS_connect, sk, &sa, sizeof sa), -ECONNREFUSED);
    a_close(sk);
    sk = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    expect("bind_over_node_eaddrinuse", syscall3(SYS_bind, sk, &sa, sizeof sa), -EADDRINUSE);
    a_close(sk);

    expect("again_eexist", mk(D "/s", S_IFSOCK | 0644, 0), -EEXIST);
    expect("fifo_over_sock_eexist", mk(D "/s", S_IFIFO | 0644, 0), -EEXIST);

    expect("cleanup_s", a_unlink(D "/s"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
