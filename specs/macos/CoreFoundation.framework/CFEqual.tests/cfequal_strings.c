/* poly-mac: corefoundation */
/* Strings are equal when their UTF-16 code units are equal, whichever way
 * they were made; case, accents and composition all count. */
#include "poly_mcf.h"

int test_main(void) {
    CK(CFEqual(S("same text"), S("same text")));
    CK(!CFEqual(S("abc"), S("abd")));
    CK(!CFEqual(S("abc"), S("ABC")));
    CK(!CFEqual(S("abc"), S("abcd")));
    CK(CFEqual(S(""), CFSTR("")));
    CK(CFEqual(CFSTR("constant"), S("constant")));
    CFMutableStringRef m = CFStringCreateMutable(kCFAllocatorDefault, 0);
    CFStringAppendCString(m, "constant", kCFStringEncodingUTF8);
    CK(CFEqual(m, CFSTR("constant")));
    CK(CFEqual(CFSTR("constant"), m));
    UniChar composed[1] = {0x00e9}, decomposed[2] = {0x0065, 0x0301};
    CFStringRef c = CFStringCreateWithCharacters(kCFAllocatorDefault, composed, 1);
    CFStringRef d = CFStringCreateWithCharacters(kCFAllocatorDefault, decomposed, 2);
    CK(!CFEqual(c, d));
    CK(CFEqual(c, c));
    CK(CFEqual(S("h\xc3\xa9llo"), CFSTR("héllo")));
    return finish();
}
