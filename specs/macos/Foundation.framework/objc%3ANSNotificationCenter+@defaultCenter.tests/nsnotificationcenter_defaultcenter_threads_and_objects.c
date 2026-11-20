/* poly-mac: foundation */
/* The default center is the same object seen from a worker thread; an
 * NSNotification made by notificationWithName:object:userInfo: keeps name, object and
 * info (nil for a missing info), two with the same content are equal
 * (isEqual:) and different objects; the notification's description mentions
 * its name. */
#include "poly_mfn.h"

static volatile int done;
static NSNotificationCenter *from_thread;

int test_main(void) {
    NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
    [NSThread detachNewThreadWithBlock:^{
        @autoreleasepool {
            from_thread = [NSNotificationCenter defaultCenter];
        }
        done = 1;
    }];
    for (int i = 0; i < 500 && !done; i++) [NSThread sleepForTimeInterval:0.01];
    CK(done && from_thread == nc);
    NSString *obj = ns_make("a sender object whose text is long enough to be a heap string");
    NSNotification *n = [NSNotification notificationWithName:@"PolyNote" object:obj userInfo:@{@"k": @"v"}];
    CK(ns_is(n.name, "PolyNote") && n.object == obj && ns_is(n.userInfo[@"k"], "v"));
    NSNotification *bare = [NSNotification notificationWithName:@"Bare" object:nil];
    CK(bare.object == nil && bare.userInfo == nil);
    NSNotification *twin = [NSNotification notificationWithName:@"PolyNote" object:obj userInfo:@{@"k": @"v"}];
    CK(n != twin && [n isEqual:twin]);
    CK(![n isEqual:bare]);
    CK([n.description containsString:@"PolyNote"]);
    return finish();
}
