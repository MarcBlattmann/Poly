#include "poly_mtest.h"
/* execve, BSD 59: refusals. An empty executable file is ENOEXEC (8) and so is a text file without a #!
 * line; a script whose interpreter is missing is ENOENT (2); a directory is EACCES (13); a path through
 * a file ENOTDIR (20); a missing file ENOENT; a name of 256 bytes ENAMETOOLONG (63); an unreadable path or
 * argv EFAULT (14); arguments of more than ARG_MAX (1 MiB) bytes are E2BIG (7); after every failed
 * execve the calling process goes on. */
#define E2BIG 7
static char big[(1 << 20) + 4096];
int test_main(void) {
    if (is_child("report")) {
        put("child argc="); put_num(poly_argc); put(" argv0_is_program="); put_num(poly_argc > 0);
        int n = 0;
        for (char **e = poly_envp; e && *e; e++) n++;
        put(" envc="); put_num(n);
        put("\n");
        poly_exit(0);
    }
    bsd3(SYS_open, "/dev/null", O_RDONLY, 0);
    char *argv[] = {poly_argv[0], "report", 0};
    char *noenv[] = {0};
    sysret r;
    bsd1(SYS_close, bsd3(SYS_open, "emptyx", O_CREAT | O_WRONLY, 0755).ret);
    r = bsd3(SYS_execve, "emptyx", argv, noenv);
    check_err("an empty executable file: ENOEXEC", r, ENOEXEC);
    int t = bsd3(SYS_open, "text", O_CREAT | O_WRONLY, 0755).ret;
    bsd3(SYS_write, t, "just text\n", 10);
    bsd1(SYS_close, t);
    check_err("a text file without #! is ENOEXEC", bsd3(SYS_execve, "text", argv, noenv), ENOEXEC);
    int s = bsd3(SYS_open, "script", O_CREAT | O_WRONLY, 0755).ret;
    bsd3(SYS_write, s, "#!/no/such/interpreter\n", 23);
    bsd1(SYS_close, s);
    check_err("a script with a missing interpreter is ENOENT", bsd3(SYS_execve, "script", argv, noenv), ENOENT);
    bsd2(SYS_mkdir, "dir", 0755);
    check_err("a directory is EACCES", bsd3(SYS_execve, "dir", argv, noenv), EACCES);
    check_err("a path through a file is ENOTDIR", bsd3(SYS_execve, "text/x", argv, noenv), ENOTDIR);
    check_err("a missing file is ENOENT", bsd3(SYS_execve, "nosuch", argv, noenv), ENOENT);
    char longname[300];
    memset(longname, 'n', 256);
    longname[256] = 0;
    check_err("a name of 256 bytes is ENAMETOOLONG", bsd3(SYS_execve, longname, argv, noenv), ENAMETOOLONG);
    check_err("an unreadable path is EFAULT", bsd3(SYS_execve, (char *)8, argv, noenv), EFAULT);
    check_err("an unreadable argv is EFAULT", bsd3(SYS_execve, poly_argv[0], (char *)8, noenv), EFAULT);
    memset(big, 'a', sizeof big - 1);
    char *bigargv[] = {poly_argv[0], "report", big, 0};
    check_err("arguments over ARG_MAX (1 MiB) are E2BIG", bsd3(SYS_execve, poly_argv[0], bigargv, noenv), E2BIG);
    check("after the failures the process goes on", bsd0(SYS_getpid).ret > 0, 0);
    return finish();
}
