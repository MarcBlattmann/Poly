/* getenv after many variables were set and some removed. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdlib.h>
#include <string.h>

int test_main(void) {
    char name[16], val[16];
    int bad = 0;
    for (int i = 0; i < 300; i++) {
        name[0] = 'V'; val[0] = 'v';
        int n = i, k = 1;
        char d[8]; int m = 0;
        do d[m++] = (char)('0' + n % 10); while (n /= 10);
        while (m) { name[k] = val[k] = d[--m]; k++; }
        name[k] = val[k] = 0;
        if (setenv(name, val, 1) != 0) bad++;
    }
    check("300 variables set", bad == 0, bad);
    bad = 0;
    for (int i = 0; i < 300; i += 7) {
        name[0] = 'V';
        int n = i, k = 1; char d[8]; int m = 0;
        do d[m++] = (char)('0' + n % 10); while (n /= 10);
        while (m) name[k++] = d[--m];
        name[k] = 0;
        char *v = getenv(name);
        if (!v || v[0] != 'v' || strcmp(v + 1, name + 1) != 0) bad++;
    }
    check("every 7th found with its own value", bad == 0, bad);
    check("V300 is not set", getenv("V300") == NULL, 0);
    check("V1 and V10 are different variables", getenv("V1") && getenv("V10") && getenv("V1") != getenv("V10"), 0);
    unsetenv("V1");
    check("V1 is gone, V10 and V11 remain", getenv("V1") == NULL && getenv("V10") && getenv("V11"), 0);
    return finish();
}
