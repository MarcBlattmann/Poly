/* poly-mac: foundation */
/* The center hands the observer the very notification posted with postNotification: (same
 * pointer), makes a new one for postNotificationName: (distinct per post), and keeps
 * the object pointer and userInfo dictionary it was given. */
#include "poly_mfn.h"

@interface PolyKeep : NSObject {
@public
    NSNotification *last;
    int hits;
}
- (void)got:(NSNotification *)n;
@end
@implementation PolyKeep
- (void)got:(NSNotification *)n {
    last = n;
    hits++;
}
@end

int test_main(void) {
    NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
    PolyKeep *k = [[PolyKeep alloc] init];
    [nc addObserver:k selector:@selector(got:) name:@"PolyKeep" object:nil];
    NSNotification *mine = [NSNotification notificationWithName:@"PolyKeep" object:nil userInfo:@{@"a": @"b"}];
    [nc postNotification:mine];
    CK(k->hits == 1 && k->last == mine);
    [nc postNotificationName:@"PolyKeep" object:nil];
    NSNotification *first = k->last;
    [nc postNotificationName:@"PolyKeep" object:nil];
    CK(k->hits == 3 && k->last != first && k->last != mine);
    CK(ns_is(k->last.name, "PolyKeep"));
    [nc removeObserver:k];
    return finish();
}
