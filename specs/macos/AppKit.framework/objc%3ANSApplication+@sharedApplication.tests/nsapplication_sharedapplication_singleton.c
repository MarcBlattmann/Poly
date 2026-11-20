/* poly-mac: appkit */
/* sharedApplication makes the application object on the first call and gives the same
 * object afterwards; the global NSApp is nil before the first call and is that object after
 * it; the object is an NSApplication (not a subclass) and an NSResponder. */
#include "poly_mak.h"

int test_main(void) {
    CK(NSApp == nil);
    NSApplication *a = [NSApplication sharedApplication];
    CK(a != nil);
    CK(NSApp == a);
    CK([NSApplication sharedApplication] == a);
    CK(ak_init() == a);
    CK([a class] == [NSApplication class]);
    CK([a isKindOfClass:[NSResponder class]]);
    return finish();
}
