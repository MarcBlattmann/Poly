/* bind for AF_UNIX: a path name creates a socket file (EADDRINUSE if the
 * path exists, even a stale socket file; ENOENT for a missing directory);
 * an abstract name (leading NUL) uses no file and must be unique
 * (EADDRINUSE); binding a bound socket again is EINVAL (EADDRINUSE when
 * the new name is taken, its own path included); a wrong family is
 * EINVAL; a length below 2 is EINVAL; a path that does not fit sun_path is
 * EINVAL; a bad pointer is EFAULT; a non-socket fd is ENOTSOCK. */
#include "poly_ipc.h"

#define SYS_newfstatat 262
#define SYS_unlinkat 263
#define P "/tmp/b_bind_unix.sock"

struct k_stat {
    u64 dev, ino, nlink;
    unsigned mode, uid, gid, pad0;
    u64 rdev;
    i64 size, blksize, blocks;
    u64 times[6];
    i64 reserved[3];
};

static struct k_sockaddr_un path_addr(const char *p, int *len) {
    struct k_sockaddr_un a;
    memset(&a, 0, sizeof a);
    a.sun_family = AF_UNIX;
    int n = 0;
    while (p[n]) {
        a.sun_path[n] = p[n];
        n++;
    }
    *len = 2 + n + 1;
    return a;
}

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, P, 0);
    int s = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    int len;
    struct k_sockaddr_un a = path_addr(P, &len);
    i64 r = syscall3(SYS_bind, s, &a, len);
    check("bind_path_ok", r == 0, r);
    struct k_stat st;
    r = syscall4(SYS_newfstatat, AT_FDCWD, P, &st, 0);
    check("socket_file_created", r == 0 && (st.mode & 0170000) == 0140000, st.mode);
    r = syscall3(SYS_bind, s, &a, len);
    check("rebind_same_path_eaddrinuse", r == -EADDRINUSE, r);
    struct k_sockaddr_un other = { AF_UNIX, { 0, 'b', '_', 'r', 'e' } };
    r = syscall3(SYS_bind, s, &other, 2 + 5);
    check("rebind_free_name_einval", r == -EINVAL, r);
    int t = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    r = syscall3(SYS_bind, t, &a, len);
    check("path_in_use_eaddrinuse", r == -EADDRINUSE, r);
    close(s);
    r = syscall3(SYS_bind, t, &a, len);
    check("stale_file_still_eaddrinuse", r == -EADDRINUSE, r);
    syscall3(SYS_unlinkat, AT_FDCWD, P, 0);
    r = syscall3(SYS_bind, t, &a, len);
    check("after_unlink_ok", r == 0, r);
    close(t);
    syscall3(SYS_unlinkat, AT_FDCWD, P, 0);

    int u = (int)syscall3(SYS_socket, AF_UNIX, SOCK_DGRAM, 0);
    struct k_sockaddr_un m = path_addr("/tmp/b_bind_missing_dir/x", &len);
    r = syscall3(SYS_bind, u, &m, len);
    check("missing_dir_enoent", r == -ENOENT, r);

    struct k_sockaddr_un ab = { AF_UNIX, { 0, 'b', '_', 'b', 'i', 'n', 'd' } };
    r = syscall3(SYS_bind, u, &ab, 2 + 7);
    check("abstract_ok", r == 0, r);
    int v = (int)syscall3(SYS_socket, AF_UNIX, SOCK_DGRAM, 0);
    r = syscall3(SYS_bind, v, &ab, 2 + 7);
    check("abstract_in_use_eaddrinuse", r == -EADDRINUSE, r);
    r = syscall3(SYS_bind, v, &ab, 2 + 6);
    check("shorter_abstract_is_other_name", r == 0, r);
    close(u);
    int w = (int)syscall3(SYS_socket, AF_UNIX, SOCK_DGRAM, 0);
    r = syscall3(SYS_bind, w, &ab, 2 + 7);
    check("abstract_freed_on_close", r == 0, r);

    int x = (int)syscall3(SYS_socket, AF_UNIX, SOCK_DGRAM, 0);
    struct k_sockaddr_in in = in_loopback(0);
    r = syscall3(SYS_bind, x, &in, sizeof in);
    check("wrong_family_einval", r == -EINVAL, r);
    r = syscall3(SYS_bind, x, &ab, 1);
    check("length_1_einval", r == -EINVAL, r);
    r = syscall3(SYS_bind, x, &ab, sizeof(struct k_sockaddr_un) + 1);
    check("length_over_struct_einval", r == -EINVAL, r);
    r = syscall3(SYS_bind, x, 8, 10);
    check("bad_pointer_efault", r == -EFAULT, r);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    r = syscall3(SYS_bind, p[0], &ab, 2 + 7);
    check("not_socket_enotsock", r == -ENOTSOCK, r);
    r = syscall3(SYS_bind, 90, &ab, 2 + 7);
    check("closed_fd_ebadf", r == -EBADF, r);
    return finish();
}
