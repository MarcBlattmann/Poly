/* A unix-domain socket file cannot be opened, so for a privileged
 * caller swapoff of its path is ENXIO (6), also through a symlink. After
 * the socket file is removed the same path is ENOENT (2).
 * poly-diff: needs=root (only root gets past the CAP_SYS_ADMIN check)
 */
#include "poly_ipc.h"

#define EISDIR 21

#define SYS_swapoff 168
#define SYS_newfstatat 262

#define S "/tmp/swapoff_root_sock"
#define L "/tmp/swapoff_root_sock_link"

struct sun {
    unsigned short family;
    char path[108];
};

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, S, 0);
    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    struct sun a;
    memset(&a, 0, sizeof a);
    a.family = AF_UNIX;
    memcpy(a.path, S, sizeof S);
    i64 fd = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    check("socket", fd >= 0, fd);
    i64 r = syscall3(SYS_bind, fd, &a, sizeof a);
    check("bind", r == 0, r);
    syscall3(SYS_symlinkat, S, AT_FDCWD, L);

    r = syscall1(SYS_swapoff, S);
    check("socket_enxio", r == -ENXIO, r);
    r = syscall1(SYS_swapoff, L);
    check("symlink_socket_enxio", r == -ENXIO, r);
    syscall1(SYS_close, fd);
    r = syscall1(SYS_swapoff, S);
    check("closed_socket_enxio", r == -ENXIO, r);

    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, S, 0) == 0, 0);
    r = syscall1(SYS_swapoff, S);
    check("removed_enoent", r == -ENOENT, r);
    return finish();
}
