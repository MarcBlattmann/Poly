/* Duplicates of an epoll descriptor share one instance (registrations
 * made through one are seen through the other); the instance lives until
 * its last descriptor is closed; F_SETFD/FD_CLOEXEC on one duplicate does
 * not affect the other; many instances can exist at once. */
#include "poly_ipc.h"

int test_main(void) {
    int e = (int)syscall1(SYS_epoll_create1, EPOLL_CLOEXEC);
    int d = (int)syscall1(SYS_dup, e);
    check("dup_has_no_cloexec", syscall2(SYS_fcntl, d, F_GETFD) == 0, 0);
    check("original_keeps_cloexec", syscall2(SYS_fcntl, e, F_GETFD) == FD_CLOEXEC, 0);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    struct k_epoll_event ev = { EPOLLOUT, 9 }, out[2];
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, p[1], &ev);
    i64 r = syscall4(SYS_epoll_wait, d, out, 2, 0);
    check("dup_sees_registration", r == 1 && out[0].data == 9, r);
    r = syscall4(SYS_epoll_ctl, d, EPOLL_CTL_ADD, p[1], &ev);
    check("dup_same_interest_list_eexist", r == -EEXIST, r);
    close(e);
    r = syscall4(SYS_epoll_wait, d, out, 2, 0);
    check("instance_lives_on_in_dup", r == 1, r);
    int many[50];
    int ok = 1;
    for (int i = 0; i < 50; i++) {
        many[i] = (int)syscall1(SYS_epoll_create1, 0);
        ok &= many[i] >= 0;
    }
    check("fifty_instances", ok, 0);
    for (int i = 0; i < 50; i++) close(many[i]);
    return finish();
}
