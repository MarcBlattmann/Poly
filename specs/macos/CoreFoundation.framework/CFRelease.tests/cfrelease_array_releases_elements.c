/* poly-mac: corefoundation */
/* Releasing a collection releases what it holds: an element only the array
 * kept alive is freed with the array, and not earlier. */
#include "poly_mcf.h"

int test_main(void) {
    CFAllocatorRef al = counting_allocator();
    CFDataRef a = CFDataCreate(al, (const UInt8 *)"a", 1);
    CFDataRef b = CFDataCreate(al, (const UInt8 *)"b", 1);
    const void *vals[2] = {a, b};
    CFArrayRef arr = CFArrayCreate(al, vals, 2, &kCFTypeArrayCallBacks);
    CFRelease(a);
    CFRelease(b);
    CKV(ca_deallocs == 0, ca_deallocs);
    CK(CFDataGetLength((CFDataRef)CFArrayGetValueAtIndex(arr, 1)) == 1);
    CFRelease(arr);
    CKV(ca_deallocs >= 3, ca_deallocs);
    CKV(ca_deallocs == ca_allocs, ca_allocs - ca_deallocs);
    return finish();
}
