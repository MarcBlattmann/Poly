// poly-mac: libobjc
// What the runtime does with the executable's Objective-C sections before
// main runs: +load is sent once per class and category that implements it,
// superclasses before subclasses, all classes' +load before any category's,
// classes and categories otherwise in the order of the image (the source
// order here, not the alphabet); +initialize is not sent at load time but
// by the first message to the class, once. The order is read back from a log
// the +load and +initialize methods fill.
#include "poly_mobjc.h"

static char log_[256];
static int log_n;
static void note(const char *s) {
    while (*s && log_n < 250) log_[log_n++] = *s++;
    if (log_n < 250) log_[log_n++] = ' ';
    log_[log_n] = 0;
}
static int load_count;

@interface Zeta : NSObject
@end
@interface Mid : Zeta
@end
@interface Leaf : Mid
@end
@interface Alpha : NSObject
@end
@interface Lazy : NSObject
@end
@interface Quiet : NSObject
@end
@interface Zeta (CatZ)
@end
@interface Alpha (CatA)
@end
@interface Alpha (CatB)
@end
@interface Quiet (CatQ)
@end

/* Mid, whose superclass Zeta is implemented after it, comes first in the
 * source; Alpha is alphabetically before every other class but comes last
 * among those with +load. */
@implementation Mid
+ (void)load { note("Mid"); load_count++; }
@end
@implementation Zeta
+ (void)load { note("Zeta"); load_count++; }
@end
@implementation Leaf      /* no +load of its own: Mid's is not sent again */
@end
@implementation Alpha
+ (void)load { note("Alpha"); load_count++; }
@end
@implementation Lazy
+ (void)initialize { note("initialize(Lazy)"); }
@end
@implementation Quiet
@end
@implementation Zeta (CatZ)
+ (void)load { note("Zeta(CatZ)"); load_count++; }
@end
@implementation Alpha (CatB)
+ (void)load { note("Alpha(CatB)"); load_count++; }
@end
@implementation Alpha (CatA)
+ (void)load { note("Alpha(CatA)"); load_count++; }
@end
@implementation Quiet (CatQ)    /* a category without +load */
@end

int test_main(void) {
    value_str("+load order", log_);
    value("+load calls", load_count);
    check("each +load ran once", load_count == 6, load_count);
    int has_init = 0;
    for (int i = 0; log_[i]; i++) if (log_[i] == 'i' && log_[i + 1] == 'n' && log_[i + 2] == 'i') has_init = 1;
    check("no +initialize ran at load time", has_init == 0, has_init);
    check("the order: Zeta before Mid, then Alpha, then the categories in source order", str_eq(log_, "Zeta Mid Alpha Zeta(CatZ) Alpha(CatB) Alpha(CatA) "), 0);
    ((id (*)(id, SEL))objc_msgSend)((id)objc_getClass("Lazy"), sel_registerName("self"));
    check("the first message runs +initialize", log_[log_n - 2] == ')', log_n);
    value_str("log after the first message to Lazy", log_);
    ((id (*)(id, SEL))objc_msgSend)((id)objc_getClass("Lazy"), sel_registerName("self"));
    int n = log_n;
    ((id (*)(id, SEL))objc_msgSend)((id)objc_getClass("Lazy"), sel_registerName("self"));
    check("and only once", log_n == n, log_n);
    return finish();
}
