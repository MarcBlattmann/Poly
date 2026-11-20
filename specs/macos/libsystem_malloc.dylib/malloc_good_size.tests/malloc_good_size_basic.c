/* malloc_good_size(n): the size malloc(n) really gives. Small requests
 * round up to 16 byte multiples (0 gives 16); the size classes above
 * 128 bytes differ between arm64 and x86_64 and are recorded per
 * architecture as values. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <malloc/malloc.h>
#include <stdlib.h>

int test_main(void) {
    check("0 -> 16", malloc_good_size(0) == 16, (long long)malloc_good_size(0));
    check("1 -> 16", malloc_good_size(1) == 16, 0);
    check("15 -> 16", malloc_good_size(15) == 16, 0);
    check("16 -> 16", malloc_good_size(16) == 16, 0);
    check("17 -> 32", malloc_good_size(17) == 32, (long long)malloc_good_size(17));
    check("31 -> 32", malloc_good_size(31) == 32, 0);
    check("33 -> 48", malloc_good_size(33) == 48, 0);
    check("100 -> 112", malloc_good_size(100) == 112, (long long)malloc_good_size(100));
    check("112 -> 112", malloc_good_size(112) == 112, 0);
    check("113 -> 128", malloc_good_size(113) == 128, 0);
    check("128 -> 128", malloc_good_size(128) == 128, 0);
    check("a size class is its own good size: good(good(n)) == good(n)", malloc_good_size(malloc_good_size(300)) == malloc_good_size(300), 0);
    size_t probes[] = {129, 256, 500, 513, 1000, 1008, 1025, 2000, 4096, 5000, 15000, 16384, 20000, 100000, 1000000};
    int bad = 0;
    for (unsigned i = 0; i < sizeof probes / sizeof *probes; i++) {
        void *p = malloc(probes[i]);
        bad += malloc_size(p) != malloc_good_size(probes[i]);
        value("good_size of", (long long)probes[i]);
        value("is", (long long)malloc_good_size(probes[i]));
        free(p);
    }
    check("it equals malloc_size of the real block", bad == 0, bad);
    return finish();
}
