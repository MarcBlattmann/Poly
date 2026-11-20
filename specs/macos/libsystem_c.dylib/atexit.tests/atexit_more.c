/* atexit and _exit: _exit() skips the handlers; abort-free. The parent
 * runs a child that registers a handler and calls _exit; then one that
 * returns normally. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <spawn.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <unistd.h>

static void h(void) { put("child handler ran\n"); }

static int run(const char *mode) {
    char *argv[] = {poly_argv[0], (char *)mode, NULL};
    pid_t pid;
    if (posix_spawn(&pid, poly_argv[0], NULL, NULL, argv, poly_envp) != 0) return -1;
    int st = 0;
    waitpid(pid, &st, 0);
    return WIFEXITED(st) ? WEXITSTATUS(st) : -2;
}

int test_main(void) {
    if (is_child("underscore")) {
        atexit(h);
        put("child: calling _exit(7)\n");
        _exit(7);
    }
    if (is_child("normal")) {
        atexit(h);
        put("child: returning\n");
        return 9;
    }
    check("_exit: status 7 and no handler", run("underscore") == 7, 0);
    check("return: status 9 after the handler", run("normal") == 9, 0);
    return finish();
}
