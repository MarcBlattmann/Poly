/*
 * The first process of the boot test group `corefoundation` (ADR 0260,
 * tools/xtask/src/corefoundation.rs). Written for Poly; no code is copied
 * from anywhere.
 *
 * It reads /etc/cftests.txt (one test per line: its name, and `app` for a
 * test that runs inside an application bundle) and starts each with the
 * posix_spawn system call (`__posix_spawn`: pid, path, argument
 * descriptor, argv, envp; no descriptor here) and an empty environment, as the macOS oracle does: a plain
 * test as /bin/<name> from /, an application test as
 * /apps/<name>/T.app/Contents/MacOS/t with the argument --in-app from
 * /apps/<name> (the boot test laid T.app out beforehand, which the test's
 * first half does with mkdir and execve on macOS). Around each it prints
 *
 *     cf-test: <name>
 *     cf-exit: <name> <exit status, or 128 + signal>
 *
 * and at the end `cf-runner: done`.
 */
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

/* The BSD call itself (libsystem_kernel), which takes one descriptor of
 * file actions and attributes where posix_spawn(3) takes two pointers. */
int __posix_spawn(pid_t *pid, const char *path, void *desc, char *const argv[], char *const envp[]);

static char list[16384];

int main(void) {
    int fd = open("/etc/cftests.txt", O_RDONLY);
    if (fd < 0) {
        printf("cf-runner: no /etc/cftests.txt\n");
        return 1;
    }
    long n = read(fd, list, sizeof list - 1);
    close(fd);
    if (n < 0) n = 0;
    list[n] = 0;
    char *line = list;
    while (*line) {
        char *end = strchr(line, '\n');
        if (end) *end = 0;
        char *name = line;
        line = end ? end + 1 : name + strlen(name);
        if (!*name) continue;
        char *space = strchr(name, ' ');
        int app = 0;
        if (space) {
            *space = 0;
            app = strcmp(space + 1, "app") == 0;
        }
        char path[512], dir[512];
        char *argv[3] = {0, 0, 0};
        char *envp[1] = {0};
        if (app) {
            snprintf(dir, sizeof dir, "/apps/%s", name);
            snprintf(path, sizeof path, "/apps/%s/T.app/Contents/MacOS/t", name);
            argv[0] = "T.app/Contents/MacOS/t";
            argv[1] = "--in-app";
        } else {
            snprintf(dir, sizeof dir, "/");
            snprintf(path, sizeof path, "/bin/%s", name);
            argv[0] = path;
        }
        printf("cf-test: %s\n", name);
        fflush(stdout);
        if (chdir(dir) != 0) {
            printf("cf-exit: %s chdir-failed\n", name);
            continue;
        }
        pid_t pid;
        int r = __posix_spawn(&pid, path, NULL, argv, envp);
        if (r != 0) {
            printf("cf-exit: %s spawn-error-%d\n", name, r);
            continue;
        }
        int status = 0;
        if (wait4(pid, &status, 0, NULL) != pid) {
            printf("cf-exit: %s wait-failed\n", name);
            continue;
        }
        int code = WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
        printf("cf-exit: %s %d\n", name, code);
        fflush(stdout);
    }
    printf("cf-runner: done\n");
    return 0;
}
