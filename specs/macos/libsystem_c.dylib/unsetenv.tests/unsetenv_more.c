/* unsetenv removes every copy of a name when the program was started with
 * duplicates (a child with a made-up environment). */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <spawn.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>

extern char **environ;

static int child(void) {
    check_str("the first of the duplicates is found", getenv("D"), "1");
    check("unsetenv returns 0", unsetenv("D") == 0, 0);
    check("none of the duplicates is left", getenv("D") == NULL, 0);
    int n = 0;
    while (environ[n]) n++;
    check("the other variables are left", n == 2, n);
    check_str("order kept: [0]", environ[0], "A=a");
    check_str("order kept: [1]", environ[1], "Z=z");
    return finish();
}

int test_main(void) {
    if (is_child("child")) return child();
    char *argv[] = {poly_argv[0], "child", NULL};
    char *envp[] = {"A=a", "D=1", "D=2", "Z=z", "D=3", NULL};
    pid_t pid;
    check("spawn", posix_spawn(&pid, poly_argv[0], NULL, NULL, argv, envp) == 0, 0);
    int st = 0;
    waitpid(pid, &st, 0);
    check("the child passed", WIFEXITED(st) && WEXITSTATUS(st) == 0, st);
    return finish();
}
