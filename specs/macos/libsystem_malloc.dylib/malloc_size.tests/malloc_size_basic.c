/* malloc_size: the usable size of a block malloc returned; 0 for NULL and
 * for addresses malloc does not own. Sizes are 16 byte multiples, at least
 * the request, and at least 16. The exact classes differ between arm64 and
 * x86_64 (see malloc_good_size). */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <malloc/malloc.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

static int global;

int test_main(void) {
    check("malloc_size(NULL) is 0", malloc_size(NULL) == 0, 0);
    int local = 0;
    check("malloc_size of a stack address is 0", malloc_size(&local) == 0, 0);
    check("malloc_size of a global is 0", malloc_size(&global) == 0, 0);
    check("malloc_size of a string literal is 0", malloc_size("literal") == 0, 0);

    size_t sizes[] = {0, 1, 2, 8, 15, 16, 17, 24, 32, 33, 48, 100, 127, 128, 129, 500, 1000, 1024, 1025, 4096, 5000, 16384, 100000, 1000000};
    int small = 0, mult = 0, align = 0;
    for (unsigned i = 0; i < sizeof sizes / sizeof *sizes; i++) {
        void *p = malloc(sizes[i]);
        size_t s = malloc_size(p);
        small += s < sizes[i];
        mult += s % 16 != 0 || s == 0;
        align += ((uintptr_t)p & 15) != 0;
        value("request", (long long)sizes[i]);
        value("malloc_size", (long long)s);
        free(p);
    }
    check("never smaller than the request", small == 0, small);
    check("a nonzero multiple of 16 (also for a request of 0)", mult == 0, mult);
    check("every block is 16 byte aligned", align == 0, align);

    void *p = malloc(0);
    check("malloc(0) returns a real, freeable block of 16", p != NULL && malloc_size(p) == 16, (long long)malloc_size(p));
    free(p);
    p = malloc(10);
    size_t usable = malloc_size(p);
    for (size_t i = 0; i < usable; i++) ((volatile char *)p)[i] = 1;
    check("the whole malloc_size (16) is writable", usable == 16, (long long)usable);
    free(p);

    p = calloc(10, 10);
    check("calloc(10,10) is sized like malloc(100)", malloc_size(p) == malloc_good_size(100), (long long)malloc_size(p));
    free(p);
    p = malloc(100);
    size_t before = malloc_size(p);
    void *q = realloc(p, before);
    check("realloc to its own size keeps the block", q == p, 0);
    q = realloc(q, 5000);
    check("realloc grows the block: size >= 5000", malloc_size(q) >= 5000, (long long)malloc_size(q));
    free(q);
    return finish();
}
