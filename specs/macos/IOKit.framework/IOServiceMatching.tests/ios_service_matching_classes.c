/* poly-mac: iosurface
 * The classes Chrome asks for at start-up are found: IOPlatformExpertDevice (exactly one), IOSurfaceRoot (one), IOAccelerator (at least one), IOService (the
 * root's class: many); a class that does not exist (NoSuchPolyClass) finds none. Counts that depend on the machine are recorded as values only. */
#include "poly_miosurface.h"
int test_main(void) {
    check("IOPlatformExpertDevice: one", ios_count_services("IOPlatformExpertDevice") == 1, ios_count_services("IOPlatformExpertDevice"));
    check("IOSurfaceRoot: one", ios_count_services("IOSurfaceRoot") == 1, ios_count_services("IOSurfaceRoot"));
    check("IOAccelerator: at least one", ios_count_services("IOAccelerator") >= 1, ios_count_services("IOAccelerator"));
    check("IOService: many", ios_count_services("IOService") > 100, ios_count_services("IOService"));
    check("a class that does not exist: none", ios_count_services("NoSuchPolyClass") <= 0, ios_count_services("NoSuchPolyClass"));
    value("the number of IOAccelerator services is 1 or more", ios_count_services("IOAccelerator") >= 1);
    return finish();
}
