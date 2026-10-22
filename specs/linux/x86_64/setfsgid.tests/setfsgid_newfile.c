/* A new file in /tmp (not set-group-ID) gets the filesystem gid as its
 * group; after setfsgid to one's own egid that is the egid. Any user. */
#include "poly_ipc.h"

#ifndef SYS_fstat
#define SYS_fstat 5
#endif

int test_main(void) {
    const char *path = "/tmp/poly_setfsgid_newfile";
    syscall1(SYS_unlink, path);
    i64 e = syscall0(SYS_getegid);
    i64 v = syscall1(SYS_setfsgid, e);
    check("set_returns_egid", v == e, v);
    i64 fd = syscall3(SYS_open, path, O_CREAT | O_EXCL | O_RDWR, 0600);
    check("create_ok", fd >= 0, fd);
    unsigned st[36];
    i64 rc = syscall2(SYS_fstat, fd, st);
    check("fstat_ok", rc == 0, rc);
    check("group_is_fsgid", (i64)st[8] == e, (i64)st[8]);
    close((int)fd);
    syscall1(SYS_unlink, path);
    return finish();
}
