/* A dynamically linked test program for Poly's dyld: imports from libgreet
 * (two-level namespace, through @rpath) and from the libSystem stand-in, a
 * missing weak import, thread-local variables (initialised and zero fill),
 * an initializer, __mh_execute_header, argc/argv/envp/apple, and an exit
 * through the stand-in's `exit` when main returns. Each check prints one
 * line; main returns the number of failed checks. */
#include "sys.h"

extern long poly_write(int fd, const char *buf, long len);
extern int greet(const char *who);
extern const char *greet_word(int i);
extern int greet_inits;
extern int poly_missing(void) __attribute__((weak_import));
extern const unsigned int _mh_execute_header[];

static _Thread_local int tls_init = 40;
static _Thread_local long tls_zero;
static int main_inits;
static int failures;

static void say(const char *s) { poly_write(1, s, (long)poly_strlen(s)); }

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

__attribute__((constructor)) static void hello_init(int argc, char **argv, char **envp, char **apple) {
    (void)argv;
    (void)envp;
    (void)apple;
    /* Dependencies first: libgreet's initializer has run. */
    if (argc > 0 && greet_inits == 1) main_inits = 1;
    say("hello: init\n");
}

int main(int argc, char **argv, char **envp, char **apple) {
    check("initializers ran, the library's first", main_inits == 1);
    check("greet binds across images", greet("mach-o") == 6);
    check("a rebased pointer table in the library", greet_word(1)[0] == 'o' && greet_word(1)[1] == 'k');
    check("a missing weak import is null", poly_missing == 0);
    check("__mh_execute_header is the mapped header", _mh_execute_header[0] == 0xfeedfacfu);
    tls_init += 2;
    tls_zero += 7;
    check("thread-local variables", tls_init == 42 && tls_zero == 7);
    check("argc and argv", argc >= 1 && argv[0] != 0 && argv[argc] == 0);
    int envc = 0;
    while (envp[envc]) envc++;
    check("envp ends with null", envc >= 0);
    check("apple[0] is executable_path=", apple[0] != 0 && starts_with(apple[0], "executable_path="));
    say(failures == 0 ? "hello: done\n" : "hello: failed\n");
    return failures;
}
