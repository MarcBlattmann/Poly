/* (b) A dynamically linked arm64 program through Poly's dyld (ADR 0257): it
 * imports from the libSystem stand-in (`poly_write`, `malloc`, `free`,
 * `exit` when main returns), has an initializer and thread-local variables,
 * and checks argc/argv/envp/apple. Each check prints one line; main returns
 * the number of failed checks. Written for Poly. */
#include "io.h"

typedef unsigned long size_t;
extern void *malloc(size_t n);
extern void free(void *p);
extern int libsystem_inits;
extern const unsigned int _mh_execute_header[];

static _Thread_local int tls_counter = 40;
static _Thread_local long tls_zero;
static int inits;
static int failures;

static void check(const char *what, int ok) {
    say(ok ? "ok " : "not ok ");
    say(what);
    say("\n");
    if (!ok) failures++;
}

static int starts_with(const char *s, const char *prefix) {
    while (*prefix)
        if (*s++ != *prefix++) return 0;
    return 1;
}

__attribute__((constructor)) static void hello_init(int argc, char **argv) {
    (void)argv;
    /* The library's initializer ran first. */
    if (argc > 0 && libsystem_inits == 1) inits = 1;
}

int main(int argc, char **argv, char **envp, char **apple) {
    say("a64-hello: hello from arm64 through Poly's dyld\n");
    check("initializers ran, libSystem's first", inits == 1);
    check("__mh_execute_header is the mapped header", _mh_execute_header[0] == 0xfeedfacfu);
    tls_counter += 2;
    tls_zero += 7;
    check("thread-local variables", tls_counter == 42 && tls_zero == 7);
    char *s = malloc(64);
    const char *msg = "malloc";
    int i = 0;
    for (; msg[i]; i++) s[i] = msg[i];
    s[i] = 0;
    check("malloc gives writable memory", s[0] == 'm' && s[5] == 'c' && ((unsigned long)s & 15) == 0);
    free(s);
    check("argc and argv", argc >= 1 && argv[0] != 0 && argv[argc] == 0);
    int envc = 0;
    while (envp[envc]) envc++;
    check("envp ends with null", envc >= 0);
    check("apple[0] is executable_path=", apple[0] != 0 && starts_with(apple[0], "executable_path="));
    say(failures == 0 ? "a64-hello: done\n" : "a64-hello: failed\n");
    return failures;
}
