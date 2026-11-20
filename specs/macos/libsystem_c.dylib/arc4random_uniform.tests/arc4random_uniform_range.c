/* arc4random_uniform(upper): a value in [0, upper); 0 and 1 give 0. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

int test_main(void) {
    check("upper 0 gives 0", arc4random_uniform(0) == 0, 0);
    check("upper 1 gives 0", arc4random_uniform(1) == 0, 0);
    int bad = 0;
    for (int i = 0; i < 1000; i++) bad += arc4random_uniform(2) >= 2;
    check("upper 2: always 0 or 1", bad == 0, bad);
    int seen[10];
    memset(seen, 0, sizeof seen);
    bad = 0;
    for (int i = 0; i < 2000; i++) {
        uint32_t v = arc4random_uniform(10);
        if (v >= 10) bad++; else seen[v]++;
    }
    check("upper 10: always below 10", bad == 0, bad);
    int missing = 0;
    for (int i = 0; i < 10; i++) missing += seen[i] == 0;
    check("upper 10: every value 0..9 seen in 2000 draws", missing == 0, missing);
    bad = 0;
    for (int i = 0; i < 1000; i++) bad += arc4random_uniform(0xffffffffu) >= 0xffffffffu;
    check("upper 0xffffffff: always below", bad == 0, bad);
    bad = 0;
    uint32_t hi = 0;
    for (int i = 0; i < 1000; i++) {
        uint32_t v = arc4random_uniform(0x80000001u);
        if (v >= 0x80000001u) bad++;
        if (v > hi) hi = v;
    }
    check("upper 0x80000001: always below, and large values appear", bad == 0 && hi > 0x40000000u, hi);
    return finish();
}
