/* execve(2), BSD 59, an interpreter line: "#!<program> child 3" runs the
 * program with the words of the line, then the script's path as it was
 * given to execve, then the caller's arguments after argv[0]. The program
 * here is the test itself. */
#include "poly_mtest.h"

int test_main(void) {
    if (is_child("child")) {
        poly_report_child();
        poly_exit((int)poly_atoi(poly_argv[2]));
    }
    char line[600];
    size_t n = 0;
    line[n++] = '#';
    line[n++] = '!';
    size_t len = str_len(poly_argv[0]);
    memcpy(line + n, poly_argv[0], len);
    n += len;
    memcpy(line + n, " child 3\n", 9);
    n += 9;
    sysret f = bsd3(SYS_open, "script", O_CREAT | O_WRONLY, 0755);
    bsd3(SYS_write, f.ret, line, n);
    bsd1(SYS_close, f.ret);
    sysret c = bsd0(SYS_fork);
    if (c.ret2 == 1) {
        char *argv[] = {"ignored-name", "extra", 0};
        char *env[] = {0};
        bsd3(SYS_execve, "script", argv, env);
        poly_exit(126);
    }
    int st = poly_wait(c.ret);
    check("the interpreter ran and exited with 3", W_EXITED(st) && W_EXITSTATUS(st) == 3, st);
    return finish();
}
