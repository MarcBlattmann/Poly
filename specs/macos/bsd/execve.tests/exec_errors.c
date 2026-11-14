/* execve(2), BSD 59, errors that leave the caller running: ENOENT for a
 * missing file and the empty path, EACCES for a directory and for a file
 * without execute permission, ENOEXEC for an executable file that is not a
 * program, ENOTDIR, EFAULT for the path and (in a child) for argv. */
#include "poly_mtest.h"

int test_main(void) {
    char *argv[] = {"x", 0};
    char *env[] = {0};
    bsd1(SYS_umask, 022);
    bsd1(SYS_close, bsd3(SYS_open, "plain", O_CREAT | O_WRONLY, 0644).ret);
    check_err("a missing file: ENOENT", bsd3(SYS_execve, "missing", argv, env), ENOENT);
    check_err("the empty path: ENOENT", bsd3(SYS_execve, "", argv, env), ENOENT);
    check_err("a directory: EACCES", bsd3(SYS_execve, ".", argv, env), EACCES);
    check_err("a file without execute permission: EACCES", bsd3(SYS_execve, "plain", argv, env), EACCES);
    bsd2(SYS_chmod, "plain", 0755);
    check_err("an executable text file: ENOEXEC", bsd3(SYS_execve, "plain", argv, env), ENOEXEC);
    check_err("a file used as a directory: ENOTDIR", bsd3(SYS_execve, "plain/x", argv, env), ENOTDIR);
    check_err("the path at address 8: EFAULT", bsd3(SYS_execve, (void *)8, argv, env), EFAULT);
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        sysret r = bsd3(SYS_execve, poly_argv[0], (void *)8, env);
        poly_exit((int)r.err);
    }
    int st = poly_wait(f.ret);
    check("argv at address 8: EFAULT", W_EXITED(st) && W_EXITSTATUS(st) == EFAULT, st);
    return finish();
}
