/* poly-mac: foundation */
/* dataWithBytes:length: copies the bytes (later changes to the source do not show),
 * length and bytes read back, empty data has length 0 and a NULL bytes pointer,
 * isEqualToData: compares byte for byte, subdataWithRange: takes a part, the
 * description shows the length and the bytes in hex. */
#include "poly_mfn.h"

int test_main(void) {
    char buf[3] = {'x', 'y', 'z'};
    NSData *d = [NSData dataWithBytes:buf length:3];
    buf[0] = 'Q';
    CK(d.length == 3 && ((const char *)d.bytes)[0] == 'x' && ((const char *)d.bytes)[2] == 'z');
    NSData *e = [NSData dataWithBytes:NULL length:0];
    CK(e != nil && e.length == 0 && e.bytes == NULL);
    CK([d isEqualToData:[NSData dataWithBytes:"xyz" length:3]] && ![d isEqualToData:[NSData dataWithBytes:"xyq" length:3]]);
    NSData *sub = [d subdataWithRange:NSMakeRange(1, 2)];
    CK(sub.length == 2 && ((const char *)sub.bytes)[0] == 'y');
    CK(ns_is([[NSData dataWithBytes:"abc" length:3] description], "{length = 3, bytes = 0x616263}"));
    CK(ns_is([e description], "{length = 0, bytes = 0x}"));
    CK([d isEqual:[NSData dataWithBytes:"xyz" length:3]] && [d hash] == [[NSData dataWithBytes:"xyz" length:3] hash]);
    @try {
        (void)[d subdataWithRange:NSMakeRange(2, 5)];
        CK(0);
    } @catch (NSException *ex) {
        CK(ns_is(ex.name, "NSRangeException"));
    }
    return finish();
}
