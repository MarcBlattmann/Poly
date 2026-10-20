/* fstatfs on the standard descriptors and on dup'ed copies: succeeds and
 * copies agree with their originals. */
#include "a_sys.h"

int test_main(void) {
    struct a_statfs a, b;
    int ok = 1;
    for (int fd = 0; fd < 3; fd++) ok &= syscall2(SYS_fstatfs, fd, &a) == 0;
    check("stdio_fds_ok", ok, 0);
    i64 d = syscall1(SYS_dup, 1);
    syscall2(SYS_fstatfs, 1, &a);
    expect("dup_ok", syscall2(SYS_fstatfs, d, &b), 0);
    check("dup_same", a.f_type == b.f_type && a.f_fsid[0] == b.f_fsid[0] && a.f_bsize == b.f_bsize, b.f_type);
    a_close(d);
    i64 r = a_open("/", O_RDONLY | O_DIRECTORY, 0);
    syscall2(SYS_fstatfs, r, &a);
    syscall2(SYS_statfs, "/", &b);
    check("root_fd_same_as_path", a.f_type == b.f_type && a.f_fsid[0] == b.f_fsid[0], a.f_type);
    a_close(r);
    return finish();
}
