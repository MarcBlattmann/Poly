/* getsockname for abstract AF_UNIX names: the length is exactly what was
 * bound (2 + name bytes, no terminating NUL added; embedded NULs count);
 * an autobound name is 2 + 6 (a leading NUL and 5 hex digits) and stays
 * the same on later calls; a datagram socket that sends without binding
 * stays unnamed (no autobind). */
#include "poly_ipc.h"

int test_main(void) {
    int s = (int)syscall3(SYS_socket, AF_UNIX, SOCK_DGRAM, 0);
    struct k_sockaddr_un a = { AF_UNIX, { 0, 'x', 0, 'y' } };
    syscall3(SYS_bind, s, &a, 2 + 4);
    struct k_sockaddr_un got;
    int len = sizeof got;
    memset(&got, 0x55, sizeof got);
    syscall3(SYS_getsockname, s, &got, &len);
    check("exact_length", len == 6, len);
    check("embedded_nul_kept", mem_eq(got.sun_path, "\0x\0y", 4), 0);
    check("nothing_appended", (unsigned char)got.sun_path[4] == 0x55, got.sun_path[4]);

    int t = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    struct k_sockaddr_un any = { AF_UNIX, { 0 } };
    syscall3(SYS_bind, t, &any, 2);
    struct k_sockaddr_un n1, n2;
    int l1 = sizeof n1, l2 = sizeof n2;
    syscall3(SYS_getsockname, t, &n1, &l1);
    syscall3(SYS_getsockname, t, &n2, &l2);
    check("autobind_length_8", l1 == 8, l1);
    check("autobind_stable", l1 == l2 && mem_eq(&n1, &n2, (u64)l1), 0);

    int u = (int)syscall3(SYS_socket, AF_UNIX, SOCK_DGRAM, 0);
    syscall6(SYS_sendto, u, (i64) "z", 1, 0, (i64)&a, 2 + 4);
    len = sizeof got;
    syscall3(SYS_getsockname, u, &got, &len);
    check("sending_does_not_autobind", len == 2, len);
    return finish();
}
