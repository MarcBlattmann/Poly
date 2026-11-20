/* poly-mac: foundation */
/* postNotificationName:object: is synchronous: every matching observer has run
 * when it returns; with no observer it does nothing; an observer that posts
 * another notification from its handler gets that one delivered inside; an
 * observer removed during the delivery is not called after; the notification passed
 * to the observer is an NSNotification with the posted name. */
#include "poly_mfn.h"

@interface PolyChain : NSObject {
@public
    int outer, inner, remover_hits, victim_hits;
    id victim;
}
- (void)outerGot:(NSNotification *)n;
- (void)innerGot:(NSNotification *)n;
- (void)removerGot:(NSNotification *)n;
- (void)victimGot:(NSNotification *)n;
@end
@implementation PolyChain
- (void)outerGot:(NSNotification *)n {
    (void)n;
    outer++;
    [[NSNotificationCenter defaultCenter] postNotificationName:@"PolyInner" object:nil];
    inner += 100;
}
- (void)innerGot:(NSNotification *)n {
    (void)n;
    inner++;
}
- (void)removerGot:(NSNotification *)n {
    (void)n;
    remover_hits++;
    [[NSNotificationCenter defaultCenter] removeObserver:victim];
}
- (void)victimGot:(NSNotification *)n {
    (void)n;
    victim_hits++;
}
@end

int test_main(void) {
    NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
    [nc postNotificationName:@"PolyNobody" object:nil];
    PolyChain *c = [[PolyChain alloc] init];
    [nc addObserver:c selector:@selector(outerGot:) name:@"PolyOuter" object:nil];
    [nc addObserver:c selector:@selector(innerGot:) name:@"PolyInner" object:nil];
    [nc postNotificationName:@"PolyOuter" object:nil];
    CK(c->outer == 1 && c->inner == 101);
    [nc removeObserver:c];
    PolyChain *r = [[PolyChain alloc] init], *v = [[PolyChain alloc] init];
    r->victim = v;
    [nc addObserver:r selector:@selector(removerGot:) name:@"PolyRemove" object:nil];
    [nc addObserver:v selector:@selector(victimGot:) name:@"PolyRemove" object:nil];
    [nc postNotificationName:@"PolyRemove" object:nil];
    CK(r->remover_hits == 1 && v->victim_hits == 0);
    [nc postNotificationName:@"PolyRemove" object:nil];
    CK(r->remover_hits == 2 && v->victim_hits == 0);
    [nc removeObserver:r];
    return finish();
}
