/* getenv: the program starts with an empty environment here; getenv finds
 * only exact names. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdlib.h>
#include <string.h>

extern char **environ;

int test_main(void) {
    check("the start environment is empty", poly_envp[0] == NULL && environ[0] == NULL, 0);
    check("an unset name is NULL", getenv("PATH") == NULL, 0);
    check("an empty name is NULL", getenv("") == NULL, 0);
    check("a name with '=' is NULL", getenv("A=B") == NULL, 0);

    setenv("POLYVAR", "value1", 1);
    char *p = getenv("POLYVAR");
    check_str("a set name", p, "value1");
    check("a prefix of the name is NULL", getenv("POLYVA") == NULL, 0);
    check("the name with a suffix is NULL", getenv("POLYVARX") == NULL, 0);
    check("names are case sensitive", getenv("polyvar") == NULL, 0);
    check("the result points into environ's string", p == environ[0] + 8, 0);

    setenv("EMPTY", "", 1);
    check_str("an empty value is the empty string, not NULL", getenv("EMPTY"), "");
    check("two variables", environ[0] != NULL && environ[1] != NULL && environ[2] == NULL, 0);

    setenv("P", "short", 1);
    check_str("a one-letter name", getenv("P"), "short");
    check_str("the longer name with the same first letter is unaffected", getenv("POLYVAR"), "value1");
    return finish();
}
