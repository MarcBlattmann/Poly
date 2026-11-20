// poly-mac: libobjc
// +load order of categories on a class that has no +load of its own:
// when that class is the superclass of a class with a +load, its category's
// +load runs right before that subclass's +load (the superclass is prepared
// first); the categories of classes that have a +load, or are nobody's
// superclass here, run after every class's +load, in the order the
// categories appear in the image.
#include "poly_mobjc.h"

static char log_[200];
static int log_n;
static void note(const char *s) {
    while (*s && log_n < 190) log_[log_n++] = *s++;
    log_[log_n++] = ' ';
    log_[log_n] = 0;
}

@interface P : NSObject
@end
@interface Q : NSObject
@end
@interface R : NSObject
@end
@interface S : R
@end
@interface P (C1)
@end
@interface R (C2)
@end
@interface S (C3)
@end
@interface Q (C4)
@end

@implementation Q
+ (void)load { note("Q"); }
@end
@implementation P
+ (void)load { note("P"); }
@end
@implementation S
+ (void)load { note("S"); }
@end
@implementation R
@end
@implementation Q (C4)
+ (void)load { note("Q(C4)"); }
@end
@implementation R (C2)
+ (void)load { note("R(C2)"); }
@end
@implementation S (C3)
+ (void)load { note("S(C3)"); }
@end
@implementation P (C1)
+ (void)load { note("P(C1)"); }
@end

int test_main(void) {
    value_str("order", log_);
    check("Q P, R(C2) before its subclass S, then the other categories in image order",
          str_eq(log_, "Q P R(C2) S Q(C4) S(C3) P(C1) "), 0);
    return finish();
}
