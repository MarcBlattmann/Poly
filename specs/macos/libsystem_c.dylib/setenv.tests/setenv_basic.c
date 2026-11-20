/* setenv: overwrite flag, copies of name and value, the order in environ. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <stdlib.h>
#include <string.h>

extern char **environ;

int test_main(void) {
    char name[8] = "ABC", val[8] = "one";
    check("setenv returns 0", setenv(name, val, 1) == 0, 0);
    strcpy(name, "XYZ");
    strcpy(val, "two");
    check_str("the name and value were copied", getenv("ABC"), "one");
    check("the old name is the only one", getenv("XYZ") == NULL, 0);

    check("overwrite 0 on an existing name returns 0", setenv("ABC", "new", 0) == 0, 0);
    check_str("overwrite 0 keeps the old value", getenv("ABC"), "one");
    check("overwrite 1 returns 0", setenv("ABC", "newer", 1) == 0, 0);
    check_str("overwrite 1 replaces the value", getenv("ABC"), "newer");
    check("overwrite 0 on a new name returns 0", setenv("DEF", "d", 0) == 0, 0);
    check_str("overwrite 0 on a new name sets it", getenv("DEF"), "d");
    check("a nonzero overwrite other than 1 overwrites", setenv("DEF", "e", 7) == 0, 0);
    check_str("value e", getenv("DEF"), "e");
    check("an empty value", setenv("GHI", "", 1) == 0, 0);
    check_str("is the empty string", getenv("GHI"), "");

    check_str("environ[0] after the first setenv", environ[0], "ABC=newer");
    check_str("environ[1]: new names are appended", environ[1], "DEF=e");
    check_str("environ[2]", environ[2], "GHI=");
    check("environ is NULL terminated after 3", environ[3] == NULL, 0);
    check_str("a replaced name keeps its place", (setenv("ABC", "x", 1), environ[0]), "ABC=x");

    char big[2000];
    memset(big, 'v', sizeof big - 1);
    big[sizeof big - 1] = 0;
    check("a long value", setenv("BIG", big, 1) == 0 && strlen(getenv("BIG")) == 1999, 0);
    return finish();
}
