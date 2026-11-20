/* poly-mac: corefoundation */
/* By default é and e + U+0301 differ; kCFCompareNonliteral makes them equal;
 * kCFCompareDiacriticInsensitive ignores accents and kCFCompareWidthInsensitive
 * full-width forms. */
#include "poly_mcf.h"

static CFComparisonResult cmp(const char *a, const char *b, CFOptionFlags o) { return CFStringCompare(S(a), S(b), o); }

int test_main(void) {
    CKV(cmp("\xc3\xa9", "e\xcc\x81", 0) == 1, cmp("\xc3\xa9", "e\xcc\x81", 0));
    CKV(cmp("\xc3\xa9", "e\xcc\x81", kCFCompareNonliteral) == 0, cmp("\xc3\xa9", "e\xcc\x81", kCFCompareNonliteral));
    CKV(cmp("\xc3\xa9", "e", kCFCompareDiacriticInsensitive) == 0, cmp("\xc3\xa9", "e", kCFCompareDiacriticInsensitive));
    CKV(cmp("resume", "r\xc3\xa9sum\xc3\xa9", kCFCompareDiacriticInsensitive) == 0, 0);
    CKV(cmp("\xef\xbc\xa1", "A", 0) == 1, cmp("\xef\xbc\xa1", "A", 0));
    CKV(cmp("\xef\xbc\xa1", "A", kCFCompareWidthInsensitive) == 0, cmp("\xef\xbc\xa1", "A", kCFCompareWidthInsensitive));
    return finish();
}
