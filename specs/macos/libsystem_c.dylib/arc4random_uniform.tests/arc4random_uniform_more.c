/* arc4random_uniform with every small upper bound: the result is in range. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdint.h>
#include <stdlib.h>

int test_main(void) {
    int bad = 0;
    for (uint32_t upper = 0; upper < 300; upper++)
        for (int i = 0; i < 40; i++) {
            uint32_t v = arc4random_uniform(upper);
            if (upper == 0 ? v != 0 : v >= upper) bad++;
        }
    check("every upper bound 0..299, 40 draws each", bad == 0, bad);
    bad = 0;
    for (int shift = 1; shift < 32; shift++) {
        uint32_t upper = (uint32_t)1 << shift;
        for (int i = 0; i < 20; i++) bad += arc4random_uniform(upper) >= upper;
        for (int i = 0; i < 20; i++) bad += arc4random_uniform(upper + 1) >= upper + 1;
        for (int i = 0; i < 20; i++) bad += arc4random_uniform(upper - 1) >= upper - 1 && upper > 1;
    }
    check("bounds around every power of two", bad == 0, bad);
    return finish();
}
