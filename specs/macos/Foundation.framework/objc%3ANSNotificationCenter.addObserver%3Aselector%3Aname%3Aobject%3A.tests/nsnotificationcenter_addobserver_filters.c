/* poly-mac: foundation */
/* addObserver:selector:name:object: delivers a notification whose name matches
 * (nil name matches all names) and whose object is the same object as the
 * given one (nil matches any); the selector gets the NSNotification with name,
 * object and userInfo; adding the same observer twice delivers twice;
 * removeObserver: stops delivery; delivery order of two observers is the order
 * of adding; delivery happens before post returns, on the posting thread. */
#include "poly_mfn.h"

/* (Short strings are stored in the pointer itself, so two equal short strings are
 * one object; the identity test uses a long string.) */

@interface PolyObs : NSObject {
@public
    int hits;
    NSString *lastName;
    id lastObject;
    NSDictionary *lastInfo;
    char *log;
    int tag;
}
- (void)got:(NSNotification *)n;
@end
static char order[16];
static int order_n;
@implementation PolyObs
- (void)got:(NSNotification *)n {
    hits++;
    lastName = n.name;
    lastObject = n.object;
    lastInfo = n.userInfo;
    if (tag) order[order_n++] = (char)tag;
}
@end

int test_main(void) {
    NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
    PolyObs *o = [[PolyObs alloc] init];
    [nc addObserver:o selector:@selector(got:) name:@"PolyNote" object:nil];
    [nc postNotificationName:@"PolyNote" object:@"sender"];
    CK(o->hits == 1 && ns_is(o->lastName, "PolyNote") && ns_is(o->lastObject, "sender") && o->lastInfo == nil);
    [nc postNotificationName:@"PolyOther" object:nil];
    CK(o->hits == 1);
    [nc postNotificationName:@"PolyNote" object:nil userInfo:@{@"a": @1}];
    CK(o->hits == 2 && o->lastInfo.count == 1 && [o->lastInfo[@"a"] intValue] == 1);
    [nc removeObserver:o];
    [nc postNotificationName:@"PolyNote" object:nil];
    CK(o->hits == 2);
    PolyObs *f = [[PolyObs alloc] init];
    id only = ns_make("only-this, a string too long to be stored in a pointer");
    [nc addObserver:f selector:@selector(got:) name:@"PolyFiltered" object:only];
    [nc postNotificationName:@"PolyFiltered" object:ns_make("other")];
    CK(f->hits == 0);
    [nc postNotificationName:@"PolyFiltered" object:only];
    CK(f->hits == 1);
    [nc postNotificationName:@"PolyFiltered" object:ns_make("only-this, a string too long to be stored in a pointer")];
    CK(f->hits == 1);
    [nc removeObserver:f];
    PolyObs *all = [[PolyObs alloc] init];
    [nc addObserver:all selector:@selector(got:) name:nil object:nil];
    [nc postNotificationName:@"PolyAnyName1" object:nil];
    [nc postNotificationName:@"PolyAnyName2" object:nil];
    CK(all->hits >= 2);
    [nc removeObserver:all];
    PolyObs *twice = [[PolyObs alloc] init];
    [nc addObserver:twice selector:@selector(got:) name:@"PolyTwice" object:nil];
    [nc addObserver:twice selector:@selector(got:) name:@"PolyTwice" object:nil];
    [nc postNotificationName:@"PolyTwice" object:nil];
    CK(twice->hits == 2);
    [nc removeObserver:twice];
    PolyObs *a = [[PolyObs alloc] init], *b = [[PolyObs alloc] init];
    a->tag = 'a';
    b->tag = 'b';
    [nc addObserver:a selector:@selector(got:) name:@"PolyOrder" object:nil];
    [nc addObserver:b selector:@selector(got:) name:@"PolyOrder" object:nil];
    [nc postNotificationName:@"PolyOrder" object:nil];
    order[order_n] = 0;
    CK(str_eq(order, "ab"));
    [nc removeObserver:a];
    [nc removeObserver:b];
    return finish();
}
