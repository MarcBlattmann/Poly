/* libgreet: a test dylib. Its initializer runs before the program's (it is
 * a dependency), it imports `poly_write` from the libSystem stand-in, it
 * exports a function, a data symbol and a pointer table that needs rebasing,
 * and it has a thread-local variable of its own. */
#include "sys.h"

extern long poly_write(int fd, const char *buf, long len);

int greet_inits;                 /* counted by the initializer */
static _Thread_local int greet_tls = 5;
const char *greet_words[] = {"greet", "ok"}; /* rebased pointers, exported */

static void say(const char *s) { poly_write(1, s, (long)poly_strlen(s)); }

__attribute__((constructor)) static void greet_init(int argc, char **argv) {
    (void)argv;
    if (argc > 0) greet_inits += 1;
    say("libgreet: init\n");
}

int greet(const char *who) {
    say(greet_words[0]);
    say(": hello, ");
    say(who);
    say("\n");
    greet_tls += 1;
    return greet_tls; /* 6 on the first call */
}

const char *greet_word(int i) { return greet_words[i & 1]; }
