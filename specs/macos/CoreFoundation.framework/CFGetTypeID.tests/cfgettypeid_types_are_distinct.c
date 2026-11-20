/* poly-mac: corefoundation */
/* Every class has an identifier of its own, and none of them is 0. */
#include "poly_mcf.h"

int test_main(void) {
    CFTypeID ids[] = {CFStringGetTypeID(), CFDataGetTypeID(), CFNumberGetTypeID(), CFArrayGetTypeID(),
                      CFDictionaryGetTypeID(), CFSetGetTypeID(), CFNullGetTypeID(), CFBooleanGetTypeID(),
                      CFDateGetTypeID(), CFAllocatorGetTypeID(), CFBundleGetTypeID(), CFRunLoopGetTypeID(),
                      CFRunLoopTimerGetTypeID(), CFRunLoopSourceGetTypeID(), CFRunLoopObserverGetTypeID(),
                      CFURLGetTypeID(), CFUUIDGetTypeID(), CFErrorGetTypeID(), CFLocaleGetTypeID(),
                      CFCharacterSetGetTypeID()};
    int n = (int)(sizeof ids / sizeof ids[0]), distinct = 1, nonzero = 1;
    for (int i = 0; i < n; i++) {
        if (ids[i] == 0) nonzero = 0;
        for (int j = i + 1; j < n; j++)
            if (ids[i] == ids[j]) distinct = 0;
    }
    CK(distinct);
    CK(nonzero);
    CK(CFGetTypeID(CFSTR("")) != CFGetTypeID(kCFNull));
    return finish();
}
