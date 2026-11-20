/* poly-mac: corefoundation */
/* Set callbacks: retain per distinct member on creation, release per member
 * when freed; a NULL callback table compares by pointer. */
#include "poly_mcf.h"

static int ret, rel;
static const void *cr(CFAllocatorRef a, const void *v) { (void)a; ret++; return v; }
static void cd(CFAllocatorRef a, const void *v) { (void)a; (void)v; rel++; }

int test_main(void) {
    CFSetCallBacks cb = {0, cr, cd, NULL, NULL, NULL};
    const void *v[3] = {(void *)0x10, (void *)0x20, (void *)0x30};
    CFSetRef s = CFSetCreate(kCFAllocatorDefault, v, 3, &cb);
    CKV(ret == 3, ret);
    CK(CFSetContainsValue(s, (void *)0x20) && !CFSetContainsValue(s, (void *)0x40));
    CFRelease(s);
    CKV(rel == 3, rel);
    CFSetRef raw = CFSetCreate(kCFAllocatorDefault, (const void *[]){(void *)0x10, (void *)0x10, (void *)0x11}, 3, NULL);
    CK(CFSetGetCount(raw) == 2);
    CFDataRef d = CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"x", 1);
    CFSetRef ds = CFSetCreate(kCFAllocatorDefault, (const void *[]){d}, 1, &kCFTypeSetCallBacks);
    CKV(CFGetRetainCount(d) == 2, CFGetRetainCount(d));
    CFRelease(ds);
    CKV(CFGetRetainCount(d) == 1, CFGetRetainCount(d));
    return finish();
}
