/*
 * The launcher of the app tests (`tests/linux/apps/<name>.app`, see
 * docs/AGENT_PIPELINE.md, "App tests"). It is the first program of an app
 * test's file tree, `/test-binary`, on both systems: on real Linux the diff
 * runner starts it in a chroot of the tree, on Poly the kernel starts it
 * (`poly.init`, which takes no arguments). It reads what to run from
 * `/poly-app/launch`, sets up a fixed process state and runs the steps:
 *
 *   umask 022, working directory from the file, stdin from /poly-app/stdin,
 *   stdout and stderr into /poly-app/stdout and /poly-app/stderr (regular
 *   files on both systems, so the app sees the same kind of file);
 *   every step but the last as a child (fork, execve, wait4), the last one
 *   with execve in place, so the app's exit status is the run's.
 *
 * /poly-app/launch is a list of NUL-terminated strings:
 *
 *   "poly-launch/1", cwd, nsteps, then per step: path, argc, argv[0..argc),
 *   then nenv, env[0..nenv)            (counts in decimal)
 *
 * Errors before the app runs are written to fd 2 as `poly-launch: ...` and
 * end the launcher with status 125 (bad set-up) or 127 (exec failed).
 *
 * Written for Poly from the man pages (openat(2), execve(2), fork(2),
 * wait4(2), dup2(2), umask(2)) and the Linux UAPI headers for numbers.
 */
#include "poly_test.h"

/* asm/unistd_64.h */
#define SYS_close 3
#define SYS_dup2 33
#define SYS_fork 57
#define SYS_execve 59
#define SYS_wait4 61
#define SYS_chdir 80
#define SYS_umask 95
#define SYS_openat 257

/* fcntl.h (asm-generic/fcntl.h) */
#define AT_FDCWD -100
#define O_RDONLY 0
#define O_WRONLY 01
#define O_CREAT 0100
#define O_TRUNC 01000

#define MAX_CONFIG 65536
#define MAX_WORDS 4096
#define MAX_STEPS 16

static char config[MAX_CONFIG];
static char *pointers[MAX_WORDS];

static __attribute__((noreturn)) void fail(const char *what, const char *detail, int code) {
    put_fd(2, "poly-launch: ");
    put_fd(2, what);
    if (detail) {
        put_fd(2, " ");
        put_fd(2, detail);
    }
    put_fd(2, "\n");
    exit_group(code);
}

/* The next string of the configuration, or fail. */
static char *next(char **at, char *end) {
    char *s = *at;
    if (s >= end) fail("/poly-app/launch is truncated", 0, 125);
    while (*at < end && **at) (*at)++;
    if (*at >= end) fail("/poly-app/launch is truncated", 0, 125);
    (*at)++;
    return s;
}

static long number(char **at, char *end) {
    char *s = next(at, end);
    long n = 0;
    if (!*s) fail("/poly-app/launch: empty count", 0, 125);
    for (; *s; s++) {
        if (*s < '0' || *s > '9') fail("/poly-app/launch: bad count", 0, 125);
        n = n * 10 + (*s - '0');
    }
    return n;
}

/* Open `path` and make it descriptor `fd`. */
static void redirect(int fd, const char *path, long flags) {
    i64 got = syscall6(SYS_openat, AT_FDCWD, (i64)path, flags, 0644, 0, 0);
    if (got < 0) fail("cannot open", path, 125);
    if (got != fd) {
        if (syscall2(SYS_dup2, got, fd) < 0) fail("dup2 failed for", path, 125);
        syscall1(SYS_close, got);
    }
}

int test_main(void) {
    i64 fd = syscall6(SYS_openat, AT_FDCWD, (i64) "/poly-app/launch", O_RDONLY, 0, 0, 0);
    if (fd < 0) fail("cannot open /poly-app/launch", 0, 125);
    i64 n = syscall3(SYS_read, fd, config, sizeof config);
    syscall1(SYS_close, fd);
    if (n <= 0 || n == (i64)sizeof config) fail("/poly-app/launch is empty or too large", 0, 125);
    char *at = config, *end = config + n;
    if (!str_eq(next(&at, end), "poly-launch/1")) fail("/poly-app/launch: unknown format", 0, 125);

    char *cwd = next(&at, end);
    long steps = number(&at, end);
    if (steps < 1 || steps > MAX_STEPS) fail("/poly-app/launch: bad number of steps", 0, 125);
    char *path[MAX_STEPS];
    char **argv[MAX_STEPS];
    long used = 0;
    for (long i = 0; i < steps; i++) {
        path[i] = next(&at, end);
        long argc = number(&at, end);
        if (used + argc + 1 > MAX_WORDS) fail("/poly-app/launch: too many arguments", 0, 125);
        argv[i] = &pointers[used];
        for (long j = 0; j < argc; j++) pointers[used++] = next(&at, end);
        pointers[used++] = 0;
    }
    long nenv = number(&at, end);
    if (used + nenv + 1 > MAX_WORDS) fail("/poly-app/launch: too many variables", 0, 125);
    char **envp = &pointers[used];
    for (long j = 0; j < nenv; j++) pointers[used++] = next(&at, end);
    pointers[used++] = 0;

    syscall1(SYS_umask, 022);
    if (syscall1(SYS_chdir, cwd) < 0) fail("cannot change to", cwd, 125);
    redirect(0, "/poly-app/stdin", O_RDONLY);
    redirect(1, "/poly-app/stdout", O_WRONLY | O_CREAT | O_TRUNC);
    redirect(2, "/poly-app/stderr", O_WRONLY | O_CREAT | O_TRUNC);

    for (long i = 0; i + 1 < steps; i++) {
        i64 pid = syscall0(SYS_fork);
        if (pid < 0) fail("fork failed", 0, 125);
        if (pid == 0) {
            syscall3(SYS_execve, path[i], argv[i], envp);
            fail("cannot execute", path[i], 127);
        }
        syscall6(SYS_wait4, pid, 0, 0, 0, 0, 0);
    }
    syscall3(SYS_execve, path[steps - 1], argv[steps - 1], envp);
    fail("cannot execute", path[steps - 1], 127);
}
