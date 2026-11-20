/* poly-mac: iosurface
 * The platform expert's identity properties have these shapes (their values are the machine's own and are never printed here): kIOPlatformUUIDKey is a string of 36
 * characters, five groups of hex digits in capital letters separated by dashes after 8, 4, 4 and 4 characters; kIOPlatformSerialNumberKey is a non-empty string of
 * capital letters and digits (between 8 and 14 characters); "model" and "manufacturer" are data with a terminating 0 byte; the string key "IOPlatformSerialNumber" is the same
 * as kIOPlatformSerialNumberKey. */
#include "poly_miosurface.h"
int test_main(void) {
    io_service_t pe = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    CFTypeRef uuid = IORegistryEntryCreateCFProperty(pe, CFSTR(kIOPlatformUUIDKey), NULL, 0);
    check("the UUID is a string of 36 characters", ios_str_len(uuid) == 36, ios_str_len(uuid));
    char b[64] = {0};
    if (uuid) CFStringGetCString(uuid, b, sizeof b, kCFStringEncodingUTF8);
    int shape = strlen(b) == 36 && b[8] == '-' && b[13] == '-' && b[18] == '-' && b[23] == '-';
    for (int i = 0; i < 36 && shape; i++)
        if (i != 8 && i != 13 && i != 18 && i != 23) shape &= (b[i] >= '0' && b[i] <= '9') || (b[i] >= 'A' && b[i] <= 'F');
    check("with dashes at 8, 13, 18 and 23 and hex digits in capitals elsewhere", shape, 0);
    memset(b, 0, sizeof b);
    CFTypeRef serial = IORegistryEntryCreateCFProperty(pe, CFSTR(kIOPlatformSerialNumberKey), NULL, 0);
    check("the serial number is a string", serial && CFGetTypeID(serial) == CFStringGetTypeID(), 0);
    if (serial) CFStringGetCString(serial, b, sizeof b, kCFStringEncodingUTF8);
    int ok = strlen(b) >= 8 && strlen(b) <= 14;
    for (char *c = b; *c; c++) ok &= (*c >= '0' && *c <= '9') || (*c >= 'A' && *c <= 'Z');
    check("of 8 to 14 capital letters and digits", ok, strlen(b));
    CFTypeRef serial2 = IORegistryEntryCreateCFProperty(pe, CFSTR("IOPlatformSerialNumber"), NULL, 0);
    check("the string key gives the same value", serial && serial2 && CFEqual(serial, serial2), 0);
    CFTypeRef model = IORegistryEntryCreateCFProperty(pe, CFSTR("model"), NULL, 0);
    check("model: data ending in a 0 byte", model && CFGetTypeID(model) == CFDataGetTypeID() && CFDataGetLength(model) > 1 && CFDataGetBytePtr(model)[CFDataGetLength(model) - 1] == 0, 0);
    CFTypeRef manu = IORegistryEntryCreateCFProperty(pe, CFSTR("manufacturer"), NULL, 0);
    check("manufacturer: data ending in a 0 byte", manu && CFGetTypeID(manu) == CFDataGetTypeID() && CFDataGetBytePtr(manu)[CFDataGetLength(manu) - 1] == 0, 0);
    return finish();
}
