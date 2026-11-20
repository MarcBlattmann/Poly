/* poly-mac: corefoundation */
/* kCFCompareNumerically reads runs of digits as numbers: 2 before 10, and
 * leading zeros do not count. */
#include "poly_mcf.h"

static CFComparisonResult num(const char *a, const char *b) { return CFStringCompare(S(a), S(b), kCFCompareNumerically); }

int test_main(void) {
    CKV(num("a2", "a10") == -1, num("a2", "a10"));
    CKV(num("a10", "a2") == 1, num("a10", "a2"));
    CKV(num("a02", "a2") == 0, num("a02", "a2"));
    CKV(num("a1", "a01") == 0, num("a1", "a01"));
    CKV(num("file9.txt", "file10.txt") == -1, num("file9.txt", "file10.txt"));
    CKV(num("1.5", "1.10") == -1, num("1.5", "1.10"));
    CKV(num("abc", "abd") == -1, num("abc", "abd"));
    CKV(num("100", "99") == 1, num("100", "99"));
    return finish();
}
