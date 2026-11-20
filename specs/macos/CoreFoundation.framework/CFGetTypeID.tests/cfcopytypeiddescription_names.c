/* poly-mac: corefoundation */
/* CFCopyTypeIDDescription gives the class's name, which a description shows
 * after the opening bracket. */
#include "poly_mcf.h"

static void name(const char *want, CFTypeID id) {
    CFStringRef s = CFCopyTypeIDDescription(id);
    char buf[64];
    cf_c(s, buf, sizeof buf);
    check(want, s && str_eq(buf, want), 0);
    if (s) CFRelease(s);
}

int test_main(void) {
    name("CFString", CFStringGetTypeID());
    name("CFArray", CFArrayGetTypeID());
    name("CFDictionary", CFDictionaryGetTypeID());
    name("CFSet", CFSetGetTypeID());
    name("CFData", CFDataGetTypeID());
    name("CFNumber", CFNumberGetTypeID());
    name("CFBoolean", CFBooleanGetTypeID());
    name("CFNull", CFNullGetTypeID());
    name("CFDate", CFDateGetTypeID());
    name("CFBundle", CFBundleGetTypeID());
    name("CFRunLoop", CFRunLoopGetTypeID());
    name("CFRunLoopSource", CFRunLoopSourceGetTypeID());
    name("CFRunLoopTimer", CFRunLoopTimerGetTypeID());
    name("CFURL", CFURLGetTypeID());
    return finish();
}
