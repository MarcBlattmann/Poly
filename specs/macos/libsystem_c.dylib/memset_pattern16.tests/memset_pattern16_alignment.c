/* memset_pattern16 starts the pattern at b, whatever b's alignment, and
 * len may be any size, also large. The pattern may come from an unaligned
 * address. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <string.h>

int test_main(void) {
    char raw[512];
    char praw[40];
    for (int i = 0; i < 40; i++) praw[i] = (char)('A' + i);
    int bad = 0;
    for (int off = 0; off < 16; off++) {
        memset(raw, '.', sizeof raw);
        memset_pattern16(raw + off, praw + 3, 100);
        for (int i = 0; i < 100; i++)
            if (raw[off + i] != praw[3 + (i % 16)]) bad++;
        if (raw[off + 100] != '.' || (off > 0 && raw[off - 1] != '.')) bad++;
    }
    check("every destination alignment 0..15, unaligned pattern", bad == 0, bad);

    static char big[100000];
    char pat[16];
    for (int i = 0; i < 16; i++) pat[i] = (char)(i + 1);
    memset_pattern16(big, pat, sizeof big);
    int badbig = 0;
    for (unsigned long i = 0; i < sizeof big; i++)
        if (big[i] != (char)(i % 16 + 1)) badbig++;
    check("100000 bytes", badbig == 0, badbig);

    /* The pattern is read before it is overwritten? A pattern inside the
     * destination is not tested (undefined). */
    char z[32];
    char zero[16];
    memset(zero, 0, 16);
    memset(z, 'x', sizeof z);
    memset_pattern16(z, zero, 32);
    int allz = 1;
    for (int i = 0; i < 32; i++) allz &= z[i] == 0;
    check("an all-zero pattern", allz, 0);
    return finish();
}
