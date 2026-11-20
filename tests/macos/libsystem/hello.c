/* A C "hello" for Poly's own libSystem.B.dylib (ADR 0258), written for
 * Poly. It is linked the ordinary way, against the macOS SDK's libSystem
 * (generate.sh), and on Poly runs with Poly's libSystem in its place: it
 * uses stdio and printf, malloc, errno, atexit, the string and number
 * functions, getenv, the dyld and crt_externs accessors and the stack
 * protector. Each check prints one line; main returns the number of
 * failed checks. */
#include <crt_externs.h>
#include <errno.h>
#include <mach-o/dyld.h>
#include <mach-o/ldsyms.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

extern unsigned long __stack_chk_guard;

static int fails;

static void check(const char *what, int ok) {
    printf("%s %s\n", ok ? "ok" : "not ok", what);
    if (!ok) fails++;
}

static void registered_first(void) {
    puts("atexit: then the first");
    fputs("no newline, flushed by exit", stdout);
}

static void registered_last(void) { puts("atexit: last registered runs first"); }

/* An array on the stack: -fstack-protector-strong guards this frame. */
static int copy_name(char *out, size_t size, const char *entry) {
    char local[64];
    size_t i = 0;
    while (entry[i] && entry[i] != '=' && i + 1 < sizeof local) {
        local[i] = entry[i];
        i++;
    }
    local[i] = 0;
    return snprintf(out, size, "%s", local);
}

int main(int argc, char **argv) {
    puts("hello, world");
    printf("ok printf: %d %d %x %#x %.2f %.1e [%5s] [%-5s] %s %%\n", 42, -7, 42, 42, 3.14159, 1500.0, "pad", "left",
           (char *)0);

    fputs("ok puts and putcha", stdout);
    putchar('r');
    putchar('\n');

    char *p = malloc(100);
    strcpy(p, "heap");
    int *z = calloc(10, sizeof(int));
    int zero = z != 0;
    for (int i = 0; zero && i < 10; i++) zero = z[i] == 0;
    p = realloc(p, 5000);
    check("malloc, calloc, realloc and free", p != 0 && strcmp(p, "heap") == 0 && zero);
    free(p);
    free(z);

    char buf[8];
    int n = snprintf(buf, sizeof buf, "%s", "truncated");
    printf("%s snprintf truncates: %d \"%s\"\n", n == 9 && strcmp(buf, "truncat") == 0 ? "ok" : "not ok", n, buf);

    char *a = 0;
    int k = asprintf(&a, "%s %d", "asprintf", 7);
    printf("%s asprintf: \"%s\"\n", k == 10 && a != 0 ? "ok" : "not ok", a ? a : "");
    free(a);

    errno = 0;
    ssize_t r = write(-1, "x", 1);
    int e = errno;
    check("errno is EBADF after write(-1)", r == -1 && e == EBADF);
    if (r != -1 || e != EBADF) printf("  write returned %ld, errno %d\n", (long)r, e);

    char *end = 0;
    long v = strtol("  -0x1f", &end, 0);
    check("strtol, atoi, strcmp, strlen, memcmp",
          v == -31 && end && *end == 0 && atoi("123") == 123 && strcmp("a", "b") < 0 && strlen("four") == 4 &&
              memcmp("ab", "ab", 2) == 0);

    char **env = *_NSGetEnviron();
    int env_ok = getenv("POLY_NO_SUCH_VARIABLE") == 0;
    if (env && env[0]) {
        char name[64];
        copy_name(name, sizeof name, env[0]);
        env_ok = env_ok && getenv(name) == strchr(env[0], '=') + 1;
    }
    check("getenv", env_ok);

    char path[256];
    uint32_t size = sizeof path;
    int got = _NSGetExecutablePath(path, &size);
    check("_NSGetExecutablePath and _dyld_image_count",
          got == 0 && strcmp(path, "/bin/mhello-libsystem") == 0 && _dyld_image_count() == 11 &&
              (const void *)_dyld_get_image_header(0) == (const void *)&_mh_execute_header &&
              strcmp(_dyld_get_image_name(0), "/bin/mhello-libsystem") == 0);

    check("_NSGetArgc and getprogname", *_NSGetArgc() == argc && *_NSGetArgv() == argv &&
                                            strcmp(getprogname(), "mhello-libsystem") == 0);

    check("the stack protector's guard is set", __stack_chk_guard != 0);

    fprintf(stderr, "ok fprintf to stderr\n");

    puts(fails ? "hello: failed" : "hello: done");
    atexit(registered_first);
    atexit(registered_last);
    return fails;
}
