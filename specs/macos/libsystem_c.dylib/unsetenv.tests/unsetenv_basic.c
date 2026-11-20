/* unsetenv removes a variable; the rest keep their order. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <stdlib.h>
#include <string.h>

extern char **environ;

int test_main(void) {
    setenv("A", "1", 1);
    setenv("B", "2", 1);
    setenv("C", "3", 1);
    check("unsetenv returns 0", unsetenv("B") == 0, 0);
    check("B is gone", getenv("B") == NULL, 0);
    check_str("A remains", getenv("A"), "1");
    check_str("C remains", getenv("C"), "3");
    check_str("environ[0]", environ[0], "A=1");
    check_str("environ[1] (C moved up)", environ[1], "C=3");
    check("environ[2] is the terminator", environ[2] == NULL, 0);
    check("unsetenv of an unset name returns 0", unsetenv("NOPE") == 0, 0);
    check("a prefix is not the name: A1 does not remove A", unsetenv("A1") == 0 && getenv("A") != NULL, 0);
    check("unsetenv A", unsetenv("A") == 0 && getenv("A") == NULL, 0);
    check("unsetenv C", unsetenv("C") == 0 && environ[0] == NULL, 0);
    check("unsetenv on an empty environment", unsetenv("A") == 0, 0);
    setenv("A", "again", 1);
    check_str("set again after unset", getenv("A"), "again");
    return finish();
}
