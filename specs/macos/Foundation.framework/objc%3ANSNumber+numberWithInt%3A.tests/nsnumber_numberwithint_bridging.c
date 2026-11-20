/* poly-mac: foundation */
/* NSNumber is a CFNumber: CFNumberGetValue reads it, CFNumberGetType gives the fixed-size
 * form, and a CFNumber from CFNumberCreate answers intValue and isEqual:; the
 * Booleans @YES and @NO are the CFBoolean constants and not CFNumbers; NSNull is
 * kCFNull. */
#include "poly_mfn.h"

int test_main(void) {
    NSNumber *n = [NSNumber numberWithInt:5];
    CK(CFGetTypeID((__bridge CFTypeRef)n) == CFNumberGetTypeID());
    long long v = 0;
    CK(CFNumberGetValue((__bridge CFNumberRef)n, kCFNumberLongLongType, &v) && v == 5);
    CK(CFNumberGetType((__bridge CFNumberRef)n) == kCFNumberSInt32Type);
    int seven = 7;
    NSNumber *cf = (__bridge_transfer NSNumber *)CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &seven);
    CK(cf.intValue == 7 && [cf isEqual:@7] && [cf isKindOfClass:[NSNumber class]]);
    CK((__bridge CFTypeRef)@YES == kCFBooleanTrue && (__bridge CFTypeRef)@NO == kCFBooleanFalse);
    CK(CFGetTypeID((__bridge CFTypeRef)@YES) == CFBooleanGetTypeID());
    CK((__bridge CFTypeRef)[NSNull null] == kCFNull);
    CK([@YES isEqual:@1] && [@NO isEqual:@0]);
    CK(CFEqual((__bridge CFTypeRef)@5, (__bridge CFTypeRef)@5.0));
    return finish();
}
