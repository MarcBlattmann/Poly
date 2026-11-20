/* poly-mac: foundation */
/* operatingSystemVersion is a struct of three NSIntegers (24 bytes: major,
 * minor, patch); on any system Poly stands for it is 13 or later (the oracle
 * answers 27.0.0); isOperatingSystemAtLeastVersion: agrees with it, for the same
 * version, an older one, a newer minor and a far future one; the version string
 * starts with 'Version '. The numbers are not printed. */
#include "poly_mfn.h"

int test_main(void) {
    NSOperatingSystemVersion v = [NSProcessInfo processInfo].operatingSystemVersion;
    CK(sizeof v == 24);
    CK(v.majorVersion >= 13);
    CK(v.minorVersion >= 0 && v.patchVersion >= 0);
    NSProcessInfo *pi = [NSProcessInfo processInfo];
    CK([pi isOperatingSystemAtLeastVersion:(NSOperatingSystemVersion){13, 0, 0}]);
    CK([pi isOperatingSystemAtLeastVersion:(NSOperatingSystemVersion){10, 15, 7}]);
    CK([pi isOperatingSystemAtLeastVersion:v]);
    CK(![pi isOperatingSystemAtLeastVersion:(NSOperatingSystemVersion){v.majorVersion + 1, 0, 0}]);
    CK(![pi isOperatingSystemAtLeastVersion:(NSOperatingSystemVersion){v.majorVersion, v.minorVersion + 1, 0}]);
    CK(![pi isOperatingSystemAtLeastVersion:(NSOperatingSystemVersion){99, 0, 0}]);
    CK([pi.operatingSystemVersionString hasPrefix:@"Version "]);
    return finish();
}
