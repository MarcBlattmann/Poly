/* poly-mac: corefoundation */
/* Without options strings compare by UTF-16 code unit: -1, 0 or 1; capitals
 * sort before lower case; a prefix is smaller; unit order is not code point
 * order (a surrogate sorts below U+FF61). */
#include "poly_mcf.h"

static CFComparisonResult cmp(const char *a, const char *b, CFOptionFlags o) { return CFStringCompare(S(a), S(b), o); }

int test_main(void) {
    CK(kCFCompareLessThan == -1 && kCFCompareEqualTo == 0 && kCFCompareGreaterThan == 1);
    CKV(cmp("a", "b", 0) == -1, cmp("a", "b", 0));
    CKV(cmp("b", "a", 0) == 1, cmp("b", "a", 0));
    CKV(cmp("a", "a", 0) == 0, cmp("a", "a", 0));
    CKV(cmp("a", "B", 0) == 1, cmp("a", "B", 0));
    CKV(cmp("Z", "a", 0) == -1, cmp("Z", "a", 0));
    CKV(cmp("ab", "abc", 0) == -1, cmp("ab", "abc", 0));
    CKV(cmp("abc", "ab", 0) == 1, cmp("abc", "ab", 0));
    CKV(cmp("", "a", 0) == -1, cmp("", "a", 0));
    CKV(cmp("", "", 0) == 0, cmp("", "", 0));
    CKV(cmp("a2", "a10", 0) == 1, cmp("a2", "a10", 0));
    CKV(cmp("\xf0\x9f\x98\x80", "\xef\xbd\xa1", 0) == -1, cmp("\xf0\x9f\x98\x80", "\xef\xbd\xa1", 0));
    CKV(cmp("\xc3\xa9", "e", 0) == 1, cmp("\xc3\xa9", "e", 0));
    return finish();
}
