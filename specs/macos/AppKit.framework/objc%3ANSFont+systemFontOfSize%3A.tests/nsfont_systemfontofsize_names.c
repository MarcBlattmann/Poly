/* poly-mac: appkit */
/* +systemFontOfSize: gives the system font of that size: its fontName is ".AppleSystemUIFont" (a hidden
 * font; its family name is the same), pointSize as asked, displayName "System Font Regular" (SYSTEM-DEPENDENT);
 * the size 0 gives 13 (the system font size); +systemFontSize is 13, +smallSystemFontSize 11, +labelFontSize 10;
 * systemFontSizeForControlSize: regular 13, small 11, mini 9, large 13; fonts of the same name and size are equal
 * (isEqual:) and not equal to another size; the font is not fixed pitch. */
#include "poly_mak.h"

int test_main(void) {
    NSFont *f = [NSFont systemFontOfSize:13];
    CK([f.fontName isEqualToString:@".AppleSystemUIFont"] && [f.familyName isEqualToString:@".AppleSystemUIFont"]);
    CKF(f.pointSize, 13);
    CK(!f.isFixedPitch);
    value_ns("sys_displayName", f.displayName);
    CKF([NSFont systemFontOfSize:0].pointSize, 13);
    CKF([NSFont systemFontOfSize:20.5].pointSize, 20.5);
    CKF(NSFont.systemFontSize, 13);
    CKF(NSFont.smallSystemFontSize, 11);
    CKF(NSFont.labelFontSize, 10);
    CKF([NSFont systemFontSizeForControlSize:NSControlSizeRegular], 13);
    CKF([NSFont systemFontSizeForControlSize:NSControlSizeSmall], 11);
    CKF([NSFont systemFontSizeForControlSize:NSControlSizeMini], 9);
    CKF([NSFont systemFontSizeForControlSize:NSControlSizeLarge], 13);
    CK([f isEqual:[NSFont systemFontOfSize:13]] && ![f isEqual:[NSFont systemFontOfSize:14]]);
    return finish();
}
