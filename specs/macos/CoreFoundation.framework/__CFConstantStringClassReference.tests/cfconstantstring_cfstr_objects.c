/* poly-mac: corefoundation */
/* CFSTR gives a CFString object that needs no creation or release, equal to
 * any other string of the same characters, usable everywhere a string is,
 * and the same object for the same literal in one place. */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef a = CFSTR("hello");
    CK(CFGetTypeID(a) == CFStringGetTypeID());
    CK(CFStringGetLength(a) == 5);
    CK(CFStringGetCharacterAtIndex(a, 1) == 'e');
    CK(CFEqual(a, S("hello")));
    CK(CFHash(a) == CFHash(S("hello")));
    CK(cf_is(a, "hello"));
    CFRetain(a);
    for (int i = 0; i < 5; i++) CFRelease(a);
    CK(CFStringGetLength(a) == 5);
    CFStringRef e = CFSTR("");
    CK(CFStringGetLength(e) == 0 && CFEqual(e, S("")));
    CFStringRef u = CFSTR("Aé€");
    CK(UNITS(u, 0x41, 0xe9, 0x20ac));
    CFMutableStringRef m = CFStringCreateMutableCopy(kCFAllocatorDefault, 0, a);
    CFStringAppend(m, CFSTR(" world"));
    CK(cf_is(m, "hello world") && cf_is(a, "hello"));
    return finish();
}
