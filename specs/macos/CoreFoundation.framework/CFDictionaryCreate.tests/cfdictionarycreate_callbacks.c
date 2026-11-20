/* poly-mac: corefoundation */
/* Key and value callbacks run independently: retain once per key and value on
 * creation, release once each when freed; the hash and equal callbacks of
 * the key set decide which keys are the same; NULL key callbacks compare keys
 * by pointer, NULL value callbacks store values raw. */
#include "poly_mcf.h"

static int kret, krel, vret, vrel;
static const void *kr(CFAllocatorRef a, const void *v) { (void)a; kret++; return v; }
static void kd(CFAllocatorRef a, const void *v) { (void)a; (void)v; krel++; }
static const void *vr(CFAllocatorRef a, const void *v) { (void)a; vret++; return v; }
static void vd(CFAllocatorRef a, const void *v) { (void)a; (void)v; vrel++; }

int test_main(void) {
    CFDictionaryKeyCallBacks kcb = {0, kr, kd, NULL, NULL, NULL};
    CFDictionaryValueCallBacks vcb = {0, vr, vd, NULL, NULL};
    const void *k[2] = {(void *)0x100, (void *)0x200}, *v[2] = {(void *)0x1, (void *)0x2};
    CFDictionaryRef d = CFDictionaryCreate(kCFAllocatorDefault, k, v, 2, &kcb, &vcb);
    CKV(kret == 2 && vret == 2, kret * 10 + vret);
    CK(CFDictionaryGetValue(d, (void *)0x200) == (void *)0x2);
    CK(CFDictionaryGetValue(d, (void *)0x300) == NULL);
    CFRelease(d);
    CKV(krel == 2 && vrel == 2, krel * 10 + vrel);
    CFDictionaryRef raw = CFDictionaryCreate(kCFAllocatorDefault, k, v, 2, NULL, NULL);
    CK(CFDictionaryGetCount(raw) == 2);
    CK(CFDictionaryGetValue(raw, (void *)0x100) == (void *)0x1);
    CFDictionaryRef p = CFDictionaryCreate(kCFAllocatorDefault, (const void *[]){CFSTR("k")}, (const void *[]){CFSTR("v")}, 1,
                                           &kCFTypeDictionaryKeyCallBacks, NULL);
    CK(CFDictionaryGetValue(p, S("k")) != NULL);
    return finish();
}
