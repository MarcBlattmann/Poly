/* setenv: repeated overwrites with longer and shorter values, and the
 * variable count. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdlib.h>
#include <string.h>

int test_main(void) {
    extern char **environ;
    setenv("K", "first", 1);
    setenv("K", "second-and-longer", 1);
    check_str("the new, longer value", getenv("K"), "second-and-longer");
    setenv("K", "x", 1);
    check_str("a shorter value", getenv("K"), "x");
    setenv("K", "a value that is longer than any before it, by far", 1);
    check_str("a longer one again", getenv("K"), "a value that is longer than any before it, by far");
    for (int i = 0; i < 100; i++) setenv("K", i % 2 ? "odd" : "even", 1);
    check_str("after 100 overwrites", getenv("K"), "odd");
    int n = 0;
    while (environ[n]) n++;
    check("still one variable", n == 1, n);
    setenv("K", "kept", 0);
    check_str("overwrite 0 keeps it", getenv("K"), "odd");
    return finish();
}
