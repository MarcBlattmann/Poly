/* poly-mac: appkit */
/* The numeric views of a control's text (an NSTextField): intValue, integerValue, doubleValue and
 * floatValue parse the start of the stringValue: "42" gives 42, "abc" gives 0, "3.9xyz" gives 3 as integer and 3.9
 * as double, "  12" (leading spaces) gives 12, "1e3" gives 1000 as double, "-5" gives -5; setting them writes the
 * number back as text: setIntValue:7 gives "7", setDoubleValue:2.5 gives "2.5", setFloatValue:1.25 gives "1.25";
 * the object value of the text is the string itself, and setObjectValue: with an NSNumber 99 gives "99". */
#include "poly_mak.h"

int test_main(void) {
    NSTextField *f = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 0, 100, 22)];
    f.stringValue = @"42";
    CK(f.intValue == 42 && f.integerValue == 42);
    CKF(f.doubleValue, 42);
    CKF(f.floatValue, 42);
    f.stringValue = @"abc";
    CK(f.intValue == 0);
    f.stringValue = @"3.9xyz";
    CK(f.intValue == 3);
    CKF(f.doubleValue, 3.9);
    f.stringValue = @"  12";
    CK(f.intValue == 12);
    f.stringValue = @"1e3";
    CKF(f.doubleValue, 1000);
    f.stringValue = @"-5";
    CK(f.intValue == -5);
    f.intValue = 7;
    CK([f.stringValue isEqualToString:@"7"]);
    f.doubleValue = 2.5;
    CK([f.stringValue isEqualToString:@"2.5"]);
    f.floatValue = 1.25f;
    CK([f.stringValue isEqualToString:@"1.25"]);
    f.integerValue = -300;
    CK([f.stringValue isEqualToString:@"-300"]);
    f.stringValue = @"hello";
    CK([f.objectValue isEqual:@"hello"]);
    f.objectValue = @99;
    CK([f.stringValue isEqualToString:@"99"]);
    return finish();
}
