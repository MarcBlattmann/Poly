/* File access checks use the filesystem gid, not the effective gid: a
 * file owned by root with group 300 and mode 0040 (group may read only)
 * is EACCES for open(O_RDONLY) by uid 1000 while the filesystem gid is
 * 200 (effective gid 200, saved gid 300), and opens once setfsgid(300)
 * has made the filesystem gid 300, though the effective gid is still 200.
 * setfsgid(200) takes it away again. Supplementary groups are empty.
 * poly-diff: needs=root (creates a root file and sets up other ids)
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_setfsgid 123

#define PATH "/tmp/ld_setfsgid_access.f"

static int body(void) {
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 100, 200, 300);
    check("drop_uids", syscall3(SYS_setresuid, 1000, 1000, 1000) == 0, 0);
    i64 fd = a_open(PATH, O_RDONLY, 0);
    check("fsgid_200_eacces", fd == -EACCES, fd);
    i64 v = syscall1(SYS_setfsgid, 300);
    check("setfsgid_300_returns_200", v == 200, v);
    fd = a_open(PATH, O_RDONLY, 0);
    check("fsgid_300_opens", fd >= 0, fd);
    char buf[8];
    check("reads", a_read(fd, buf, sizeof buf) == 2, 0);
    a_close(fd);
    check("egid_still_200", syscall0(SYS_getegid) == 200, syscall0(SYS_getegid));
    fd = a_open(PATH, O_WRONLY, 0);
    check("write_still_eacces", fd == -EACCES, fd);
    syscall1(SYS_setfsgid, 200);
    fd = a_open(PATH, O_RDONLY, 0);
    check("back_to_200_eacces", fd == -EACCES, fd);
    return 0;
}

int test_main(void) {
    a_block_sigchld(); /* children's ends must not interrupt at varying points */
    a_unlink(PATH);
    i64 r = a_make_file(PATH, "hi", 0600);
    check("file_made", r == 0, r);
    check("chown_group_300", syscall3(SYS_chown, PATH, 0, 300) == 0, 0);
    check("chmod_0040", syscall2(SYS_chmod, PATH, 0040) == 0, 0);
    volatile i32 *counts = a_shared();
    i64 c = a_fork_sync();
    if (c == 0) {
        body();
        counts[0] = passed;
        counts[1] = failed;
        exit_group(0);
    }
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    passed += counts[0];
    failed += counts[1];
    check("child_exited", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    check("cleanup", a_unlink(PATH) == 0, 0);
    return finish();
}
