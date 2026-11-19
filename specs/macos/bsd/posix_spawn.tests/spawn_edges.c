#include "poly_mtest.h"
/* posix_spawn, BSD 244, with no attributes and no file actions: a script with #! is run through its
 * interpreter and the child's argv holds the interpreter's argument and the script path before the
 * arguments; a text file without #! is ENOEXEC (8) returned by the call; arguments over 1 MiB are E2BIG (7);
 * a path of 256 bytes ENAMETOOLONG (63); an unreadable argv is EFAULT (14); the child's parent is the caller;
 * the child's exit status is delivered by wait4 whatever it is (0, 255) and a SIGKILL from the caller
 * ends it with signal 9; the caller gets SIGCHLD; each call gives a new pid; a NULL envp gives the child
 * an empty environment, an envp with two entries gives it those two. */
#define SYS_getppid 39
int test_main(void) {
    if (is_child("report")) {
        put("child ppid_is_parent="); put_num(bsd0(SYS_getppid).ret == poly_atoi(poly_argv[2]));
        int n = 0;
        for (char **e = poly_envp; e && *e; e++) n++;
        put(" envc="); put_num(n);
        put("\n");
        poly_exit(poly_argc > 3 ? 9 : 0);
    }
    if (is_child("code")) poly_exit((int)poly_atoi(poly_argv[poly_argc - 1]));
    if (is_child("hang")) { for (;;) bsd0(SYS_getpid); }
    i32 pid = 0;
    char *noenv[] = {0};
    sysret r;
    char me[24];
    i64 mypid = bsd0(SYS_getpid).ret;
    int n = 0;
    char tmp[24];
    int m = 0;
    i64 v = mypid;
    do tmp[m++] = (char)('0' + v % 10); while (v /= 10);
    while (m) me[n++] = tmp[--m];
    me[n] = 0;
    char *argv[] = {poly_argv[0], "report", me, 0};
    r = bsd6(SYS_posix_spawn, &pid, poly_argv[0], 0, argv, noenv, 0);
    check_ok("spawn the test itself", r);
    int st = poly_wait(pid);
    check("the child's parent is the caller, envc 0: it exits 0", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    i32 pid2 = 0;
    char *env2[] = {"A=1", "B=22", 0};
    r = bsd6(SYS_posix_spawn, &pid2, poly_argv[0], 0, argv, env2, 0);
    check("a second spawn gives a new pid", r.err == 0 && pid2 != pid, r.err);
    poly_wait(pid2);
    char *argv3[] = {poly_argv[0], "code", "255", 0};
    r = bsd6(SYS_posix_spawn, &pid, poly_argv[0], 0, argv3, noenv, 0);
    st = poly_wait(pid);
    check("exit status 255 is reported", W_EXITED(st) && W_EXITSTATUS(st) == 255, st);
    char *argvh[] = {poly_argv[0], "hang", 0};
    r = bsd6(SYS_posix_spawn, &pid, poly_argv[0], 0, argvh, noenv, 0);
    bsd2(SYS_kill, pid, SIGKILL);
    st = poly_wait(pid);
    check("a spawned child can be killed: signal 9", !W_EXITED(st) && W_TERMSIG(st) == SIGKILL, st);
    int s = bsd3(SYS_open, "script", O_CREAT | O_WRONLY, 0755).ret;
    char text[512];
    n = 0;
    const char *sb = "#!";
    for (int i = 0; sb[i]; i++) text[n++] = sb[i];
    for (int i = 0; poly_argv[0][i]; i++) text[n++] = poly_argv[0][i];
    const char *rest = " code\n";
    for (int i = 0; rest[i]; i++) text[n++] = rest[i];
    bsd3(SYS_write, s, text, n);
    bsd1(SYS_close, s);
    char *argvs[] = {"script", "17", 0};
    pid = 0;
    r = bsd6(SYS_posix_spawn, &pid, "script", 0, argvs, noenv, 0);
    check_ok("spawn a script with #!", r);
    st = poly_wait(pid);
    check("it ran through the interpreter, whose last argument was \"17\": exit 17", W_EXITED(st) && W_EXITSTATUS(st) == 17, st);
    int t = bsd3(SYS_open, "plain", O_CREAT | O_WRONLY, 0755).ret;
    bsd3(SYS_write, t, "plain text\n", 11);
    bsd1(SYS_close, t);
    pid = 0;
    r = bsd6(SYS_posix_spawn, &pid, "plain", 0, argvs, noenv, 0);
    check_err("a text file without #! is ENOEXEC", r, ENOEXEC);
    static char big[(1 << 20) + 4096];
    memset(big, 'a', sizeof big - 1);
    char *bigargv[] = {poly_argv[0], "report", big, 0};
    r = bsd6(SYS_posix_spawn, &pid, poly_argv[0], 0, bigargv, noenv, 0);
    check_err("arguments over 1 MiB are E2BIG", r, 7);
    char longname[300];
    memset(longname, 'n', 256);
    longname[256] = 0;
    r = bsd6(SYS_posix_spawn, &pid, longname, 0, argv, noenv, 0);
    check_err("a path of 256 bytes is ENAMETOOLONG", r, ENAMETOOLONG);
    r = bsd6(SYS_posix_spawn, &pid, poly_argv[0], 0, (char *)8, noenv, 0);
    check_err("an unreadable argv is EFAULT", r, EFAULT);
    r = bsd6(SYS_posix_spawn, &pid, (char *)8, 0, argv, noenv, 0);
    check_err("an unreadable path is EFAULT", r, EFAULT);
    return finish();
}
