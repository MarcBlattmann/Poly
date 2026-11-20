/* poly-mac: foundation */
/* poly-mac: no-arc */
/* An object that was autoreleased is released when the pool that was current
 * is drained (-drain) or released; before that it lives; nested pools release
 * their own objects first; an object retained again survives the pool and goes
 * with its own release. Built without ARC (NSAutoreleasePool is not available
 * under it). */
#include "poly_mfn.h"

static int deallocs;
@interface PolyTracked : NSObject
@end
@implementation PolyTracked
- (void)dealloc {
    deallocs++;
    [super dealloc];
}
@end

int test_main(void) {
    NSAutoreleasePool *pool = [[NSAutoreleasePool alloc] init];
    PolyTracked *t = [[[PolyTracked alloc] init] autorelease];
    (void)t;
    CKV(deallocs == 0, deallocs);
    [pool drain];
    CKV(deallocs == 1, deallocs);

    deallocs = 0;
    NSAutoreleasePool *outer = [NSAutoreleasePool new];
    PolyTracked *a = [[[PolyTracked alloc] init] autorelease];
    (void)a;
    NSAutoreleasePool *inner = [NSAutoreleasePool new];
    PolyTracked *b = [[[PolyTracked alloc] init] autorelease];
    (void)b;
    [inner release];
    CKV(deallocs == 1, deallocs);
    [outer release];
    CKV(deallocs == 2, deallocs);

    deallocs = 0;
    NSAutoreleasePool *p3 = [NSAutoreleasePool new];
    PolyTracked *c = [[[PolyTracked alloc] init] autorelease];
    [c retain];
    [p3 release];
    CKV(deallocs == 0, deallocs);
    [c release];
    CKV(deallocs == 1, deallocs);

    deallocs = 0;
    NSAutoreleasePool *p4 = [NSAutoreleasePool new];
    for (int i = 0; i < 100; i++) [[[PolyTracked alloc] init] autorelease];
    CKV(deallocs == 0, deallocs);
    [p4 drain];
    CKV(deallocs == 100, deallocs);
    return finish();
}
