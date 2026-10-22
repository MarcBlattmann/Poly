/* Group permission checks use the filesystem gid. With the filesystem uid
 * off 0 (no DAC override) and no supplementary groups, a file of mode 0040
 * owned by group 4243 opens for reading only while the fsgid is 4243.
 * poly-diff: needs=root (chown to another group, set*id to others)
 */
#include "poly_ipc.h"

#ifndef SYS_fchown
#define SYS_fchown 93
#endif
#ifndef SYS_fchmod
#define SYS_fchmod 91
#endif

int test_main(void) {
    const char *path = "/tmp/poly_setfsgid_access";
    syscall1(SYS_unlink, path);
    i64 fd = syscall3(SYS_open, path, O_CREAT | O_EXCL | O_RDWR, 0600);
    check("create_ok", fd >= 0, fd);
    check("chown_ok", syscall3(SYS_fchown, fd, 0, 4243) == 0, 0);
    check("chmod_ok", syscall2(SYS_fchmod, fd, 0040) == 0, 0);
    close((int)fd);
    check("setgroups_empty", syscall2(SYS_setgroups, 0, 0) == 0, 0);

    syscall1(SYS_setfsuid, 1234);
    syscall1(SYS_setfsgid, 4243);
    fd = syscall3(SYS_open, path, O_RDONLY, 0);
    check("group_read_ok", fd >= 0, fd);
    if (fd >= 0) close((int)fd);
    fd = syscall3(SYS_open, path, O_WRONLY, 0);
    check("group_write_eacces", fd == -EACCES, fd);
    if (fd >= 0) close((int)fd);

    syscall1(SYS_setfsgid, 4244);
    fd = syscall3(SYS_open, path, O_RDONLY, 0);
    check("other_fsgid_eacces", fd == -EACCES, fd);
    if (fd >= 0) close((int)fd);
    check("egid_still_0", syscall0(SYS_getegid) == 0, syscall0(SYS_getegid));

    syscall1(SYS_setfsgid, 4243);
    fd = syscall3(SYS_open, path, O_RDONLY, 0);
    check("back_to_group_ok", fd >= 0, fd);
    if (fd >= 0) close((int)fd);

    syscall1(SYS_setfsuid, 0);
    syscall1(SYS_unlink, path);
    return finish();
}
