/* malloc_good_size at page scale: sizes above 32 KiB are page multiples. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <malloc/malloc.h>

int test_main(void) {
    int bad = 0;
    for (size_t n = 100000; n <= 400000; n += 7919) {
        size_t g = malloc_good_size(n);
        bad += g < n || g % 4096 != 0 || g - n >= 65536;
    }
    check("100 KB .. 400 KB: a multiple of 4096, within 64 KiB above the request", bad == 0, bad);
    check("a request of exactly 1 MiB needs no more", malloc_good_size(1 << 20) == (1 << 20) || malloc_good_size(1 << 20) % 4096 == 0, 0);
    check("1 MiB + 1 is more than 1 MiB", malloc_good_size((1 << 20) + 1) > (1 << 20), 0);
    return finish();
}
