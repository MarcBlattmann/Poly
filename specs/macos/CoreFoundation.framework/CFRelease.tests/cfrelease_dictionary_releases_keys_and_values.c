/* poly-mac: corefoundation */
/* A dictionary made with the standard callbacks releases keys and values when
 * it is released. */
#include "poly_mcf.h"

int test_main(void) {
    CFAllocatorRef al = counting_allocator();
    CFDataRef k = CFDataCreate(al, (const UInt8 *)"k", 1);
    CFDataRef v = CFDataCreate(al, (const UInt8 *)"v", 1);
    const void *keys[1] = {k}, *vals[1] = {v};
    CFDictionaryRef d = CFDictionaryCreate(al, keys, vals, 1, &kCFTypeDictionaryKeyCallBacks,
                                           &kCFTypeDictionaryValueCallBacks);
    CFRelease(k);
    CFRelease(v);
    CKV(ca_deallocs == 0, ca_deallocs);
    CFRelease(d);
    CKV(ca_deallocs >= 3, ca_deallocs);
    CKV(ca_deallocs == ca_allocs, ca_allocs - ca_deallocs);
    return finish();
}
