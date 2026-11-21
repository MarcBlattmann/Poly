/* Test classes for Poly's Objective-C runtime (crates/poly-objc): the +load
 * order examples and the category method order of the libobjc spec sheets
 * (_objc_load_image, _objc_map_images), written for Poly. Manual reference
 * counting and no message sends, so the image imports only NSObject's
 * class objects and the empty cache. */
#include <objc/NSObject.h>

/* _objc_load_image, example 1: Mid, Zeta (superclass of Mid), Leaf (no
 * +load), Alpha, Lazy, Quiet; categories Zeta(CatZ), Alpha(CatB),
 * Alpha(CatA), Quiet(CatQ, no +load). */
@interface Zeta : NSObject @end
@interface Mid : Zeta @end
@interface Leaf : NSObject @end
@interface Alpha : NSObject @end
@interface Lazy : NSObject @end
@interface Quiet : NSObject @end
@implementation Mid + (void)load {} @end
@implementation Zeta + (void)load {} @end
@implementation Leaf @end
@implementation Alpha + (void)load {} @end
@implementation Lazy + (int)lazy { return 1; } @end
@implementation Quiet @end
@implementation Zeta (CatZ) + (void)load {} @end
@implementation Alpha (CatB) + (void)load {} @end
@implementation Alpha (CatA) + (void)load {} @end
@implementation Quiet (CatQ) - (int)q { return 2; } @end

/* Example 3: classes Q P S (subclass of R), R without +load; categories
 * Q(C4) R(C2) S(C3) P(C1). */
@interface R : NSObject @end
@interface Q : NSObject @end
@interface P : NSObject @end
@interface S : R @end
@implementation Q + (void)load {} @end
@implementation P + (void)load {} @end
@implementation S + (void)load {} @end
@implementation R @end
@implementation Q (C4) + (void)load {} @end
@implementation R (C2) + (void)load {} @end
@implementation S (C3) + (void)load {} @end
@implementation P (C1) + (void)load {} @end

/* _objc_map_images: -twice in two categories, -shadowed in one, -own,
 * -fromOne; the categories' methods come first, the last category first. */
@interface Shadow : NSObject
- (int)own;
- (int)shadowed;
@end
@implementation Shadow
- (int)own { return 1; }
- (int)shadowed { return 2; }
@end
@implementation Shadow (One)
- (int)twice { return 3; }
- (int)fromOne { return 4; }
- (int)shadowed { return 5; }
@end
@implementation Shadow (Two)
- (int)twice { return 6; }
@end
