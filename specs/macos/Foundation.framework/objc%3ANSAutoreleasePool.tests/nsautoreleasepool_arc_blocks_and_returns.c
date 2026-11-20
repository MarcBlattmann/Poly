/* poly-mac: foundation */
/* poly-mac: no-arc */
/* @autoreleasepool is the same pool: objects autoreleased inside it are gone at its
 * end; autoreleased results of Foundation methods (a string from stringWithFormat:)
 * live until the pool they were made in ends and stay valid after a pool of
 * an inner scope; an autoreleased object that is returned out of a function keeps
 * the caller's pool alive until its drain. */
#include "poly_mfn.h"

static int deallocs;
@interface PolyBox : NSObject
@end
@implementation PolyBox
- (void)dealloc {
    deallocs++;
    [super dealloc];
}
@end

static PolyBox *make_autoreleased(void) { return [[[PolyBox alloc] init] autorelease]; }

int test_main(void) {
    @autoreleasepool {
        (void)make_autoreleased();
        CKV(deallocs == 0, deallocs);
    }
    CKV(deallocs == 1, deallocs);
    deallocs = 0;
    NSString *kept;
    @autoreleasepool {
        kept = [[NSString stringWithFormat:@"%d-kept", 7] retain];
        @autoreleasepool {
            (void)make_autoreleased();
            (void)make_autoreleased();
        }
        CKV(deallocs == 2, deallocs);
        CK(ns_is(kept, "7-kept"));
    }
    CK(ns_is(kept, "7-kept"));
    [kept release];
    NSAutoreleasePool *p = [[NSAutoreleasePool alloc] init];
    PolyBox *b = make_autoreleased();
    CK(b != nil && [b retainCount] >= 1);
    [p drain];
    CKV(deallocs == 3, deallocs);
    return finish();
}
