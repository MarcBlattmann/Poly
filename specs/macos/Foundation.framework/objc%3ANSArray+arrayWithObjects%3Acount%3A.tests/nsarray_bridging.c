/* poly-mac: foundation */
/* NSArray and CFArray are one object: an NSArray answers the CFArray functions
 * (type, count, values), an array made with CFArrayCreate answers the NSArray
 * methods, and edits made through one show in the other; an element is the very
 * pointer in both. */
#include "poly_mfn.h"

int test_main(void) {
    NSString *a = ns_make("one");
    NSArray *arr = @[a, @"two"];
    CFArrayRef cf = (__bridge CFArrayRef)arr;
    CK(CFGetTypeID(cf) == CFArrayGetTypeID());
    CK(CFArrayGetCount(cf) == 2);
    CK(CFArrayGetValueAtIndex(cf, 0) == (__bridge const void *)a);
    CFArrayRef made = CFArrayCreate(kCFAllocatorDefault, (const void *[]){CFSTR("x"), CFSTR("y")}, 2, &kCFTypeArrayCallBacks);
    NSArray *ns = (__bridge_transfer NSArray *)made;
    CK(ns.count == 2 && ns_is(ns[1], "y"));
    CK([ns isKindOfClass:[NSArray class]]);
    CK([ns isEqualToArray:@[@"x", @"y"]]);
    CK(CFEqual((__bridge CFTypeRef)ns, (__bridge CFTypeRef)@[@"x", @"y"]));
    CK(CFHash((__bridge CFTypeRef)ns) == [ns hash]);
    CFMutableArrayRef cfm = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    NSMutableArray *nm = (__bridge_transfer NSMutableArray *)cfm;
    [nm addObject:@"viaNS"];
    CK(CFArrayGetCount((__bridge CFArrayRef)nm) == 1);
    CFArrayAppendValue((__bridge CFMutableArrayRef)nm, CFSTR("viaCF"));
    CK(nm.count == 2 && ns_is(nm[1], "viaCF"));
    NSString *d = [@[@"a"] description];
    CK(ns_is(d, "(\n    a\n)"));
    return finish();
}
