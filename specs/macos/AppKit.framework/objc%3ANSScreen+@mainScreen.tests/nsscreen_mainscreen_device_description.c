/* poly-mac: appkit */
/* deviceDescription of the main screen has exactly the keys NSDeviceBitsPerSample, NSDeviceColorSpaceName,
 * NSDeviceIsScreen, NSDeviceResolution, NSDeviceSize and NSScreenNumber; NSDeviceSize is the frame size, NSDeviceIsScreen is
 * the string "YES", NSDeviceResolution is 72 dpi times the scale factor (144 x 144 for a scale of 2: SYSTEM-DEPENDENT),
 * NSDeviceColorSpaceName is NSCalibratedRGBColorSpace (a string "NSCalibratedRGBColorSpace"), the number of bits per
 * sample is 8 or more; NSScreenNumber is a positive number and the same as in other calls. */
#include "poly_mak.h"

int test_main(void) {
    ak_init();
    NSScreen *s = NSScreen.mainScreen;
    NSDictionary *d = s.deviceDescription;
    NSString *keys = [[d.allKeys sortedArrayUsingSelector:@selector(compare:)] componentsJoinedByString:@","];
    CK([keys isEqualToString:@"NSDeviceBitsPerSample,NSDeviceColorSpaceName,NSDeviceIsScreen,NSDeviceResolution,NSDeviceSize,NSScreenNumber"]);
    CK(NSEqualSizes([d[NSDeviceSize] sizeValue], s.frame.size));
    CK([d[NSDeviceIsScreen] isEqual:@"YES"]);
    NSSize res = [d[NSDeviceResolution] sizeValue];
    CKF(res.width, 72 * s.backingScaleFactor);
    CKF(res.height, 72 * s.backingScaleFactor);
    CK([d[NSDeviceColorSpaceName] isEqual:NSCalibratedRGBColorSpace]);
    CK([d[NSDeviceBitsPerSample] integerValue] >= 8);
    CK([d[@"NSScreenNumber"] unsignedIntValue] > 0);
    CK([d[@"NSScreenNumber"] isEqual:NSScreen.mainScreen.deviceDescription[@"NSScreenNumber"]]);
    value_f("sys_resolution", res.width);
    return finish();
}
