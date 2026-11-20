/* poly-mac: corefoundation */
/* The last CFRelease frees the object through the allocator that made it:
 * one deallocate call per object, none before. */
#include "poly_mcf.h"

int test_main(void) {
    CFAllocatorRef al = counting_allocator();
    CFMutableStringRef s = CFStringCreateMutable(al, 0);
    CFStringAppendCString(s, "text", kCFStringEncodingUTF8);
    CFDataRef d = CFDataCreate(al, (const UInt8 *)"ab", 2);
    CKV(ca_deallocs == 0, ca_deallocs);
    CFRelease(d);
    CKV(ca_deallocs == 1, ca_deallocs);
    CFRelease(s);
    CKV(ca_deallocs >= 2, ca_deallocs);
    return finish();
}
