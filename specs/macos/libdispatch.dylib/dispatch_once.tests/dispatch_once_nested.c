/* dispatch_once with another predicate inside its block works; the main
 * thread can run it on its own. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static dispatch_once_t outer, inner;
static int order[4], n;

int test_main(void) {
    dispatch_once(&outer, ^{
        order[n++] = 1;
        dispatch_once(&inner, ^{ order[n++] = 2; });
        order[n++] = 3;
    });
    check("inner ran inside outer", n == 3 && order[0] == 1 && order[1] == 2 && order[2] == 3, n);
    dispatch_once(&inner, ^{ order[n++] = 9; });
    dispatch_once(&outer, ^{ order[n++] = 9; });
    check("nothing runs twice", n == 3, n);
    return finish();
}
