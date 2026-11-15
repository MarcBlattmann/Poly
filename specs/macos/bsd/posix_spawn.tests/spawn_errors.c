/* posix_spawn, BSD 244: failures are reported by the call itself (the
 * errno, with the carry flag) and no child is made: ENOENT for a missing
 * file, EACCES for a directory and for a file without execute permission,
 * ENOTDIR; afterwards the caller has no children to wait for (a blocking
 * wait4 returns ECHILD at once; a WNOHANG one right after a failure can
 * still see the child being torn down, so the test does not use it). */
#include "poly_mtest.h"

int test_main(void) {
    char *argv[] = {"x", 0};
    char *env[] = {0};
    i32 pid = 0;
    bsd1(SYS_umask, 022);
    bsd1(SYS_close, bsd3(SYS_open, "plain", O_CREAT | O_WRONLY, 0644).ret);
    check_err("a missing file: ENOENT", bsd6(SYS_posix_spawn, &pid, "missing", 0, argv, env, 0), ENOENT);
    check_err("a directory: EACCES", bsd6(SYS_posix_spawn, &pid, ".", 0, argv, env, 0), EACCES);
    check_err("no execute permission: EACCES", bsd6(SYS_posix_spawn, &pid, "plain", 0, argv, env, 0), EACCES);
    check_err("a file used as a directory: ENOTDIR", bsd6(SYS_posix_spawn, &pid, "plain/x", 0, argv, env, 0), ENOTDIR);
    check_err("no child was made: ECHILD", bsd4(SYS_wait4, -1, 0, 0, 0), ECHILD);
    return finish();
}
