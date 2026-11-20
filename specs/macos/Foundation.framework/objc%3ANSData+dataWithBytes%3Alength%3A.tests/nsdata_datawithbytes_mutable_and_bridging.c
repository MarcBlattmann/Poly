/* poly-mac: foundation */
/* NSMutableData appends bytes and changes its length (growing with zero bytes);
 * NSData is a CFData: CFDataGetLength and CFDataGetBytePtr give the same length and
 * the same pointer, CFEqual agrees with isEqualToData:, and a CFData from
 * CFDataCreate answers the NSData methods. */
#include "poly_mfn.h"

int test_main(void) {
    NSData *d = [NSData dataWithBytes:"abc" length:3];
    NSMutableData *m = [NSMutableData dataWithData:d];
    [m appendBytes:"de" length:2];
    CK(m.length == 5 && ((const char *)m.bytes)[3] == 'd');
    [m setLength:7];
    CK(m.length == 7 && ((const char *)m.bytes)[4] == 'e' && ((const char *)m.bytes)[6] == 0);
    [m setLength:2];
    CK(m.length == 2);
    CK(CFGetTypeID((__bridge CFTypeRef)d) == CFDataGetTypeID());
    CK(CFDataGetLength((__bridge CFDataRef)d) == 3);
    CK(CFDataGetBytePtr((__bridge CFDataRef)d) == d.bytes);
    NSData *ns = (__bridge_transfer NSData *)CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"abc", 3);
    CK([ns isEqualToData:d] && CFEqual((__bridge CFTypeRef)ns, (__bridge CFTypeRef)d));
    CFMutableDataRef cm = CFDataCreateMutable(kCFAllocatorDefault, 0);
    NSMutableData *nm = (__bridge_transfer NSMutableData *)cm;
    [nm appendBytes:"q" length:1];
    CK(CFDataGetLength(cm) == 1);
    return finish();
}
