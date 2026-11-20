// poly-mac: libobjc
// What a +load method sees: self is the class (for a category's +load too),
// _cmd is "load", it runs on the main thread, before main, and the other
// classes of the image are already usable: a class whose own +load has not
// run yet can be messaged, instantiated and have its +initialize run on the
// first message.
#include "poly_mobjc.h"
#include <pthread.h>

static int log_n;
static Class self_seen_by_a, self_seen_by_cat;
static SEL cmd_seen;
static int a_main_thread, a_other_ok, a_new_ok, b_load_ran, b_init_ran_before_load, b_init_total;
static int a_load_before_main = 1, b_load_seen_by_a = -1;

@interface A : NSObject
@end
@interface B : NSObject
+ (int)answer;
@end
@interface A (Cat)
@end

@implementation A
+ (void)load {
    self_seen_by_a = self;
    b_load_seen_by_a = b_load_ran;
    cmd_seen = _cmd;
    a_main_thread = pthread_main_np();
    a_other_ok = ((int (*)(id, SEL))objc_msgSend)((id)objc_getClass("B"), sel_registerName("answer")) == 42;
    id o = [objc_getClass("B") new];
    a_new_ok = o != 0 && object_getClass(o) == objc_getClass("B");
    b_init_ran_before_load = b_init_total;   /* B's +initialize ran because of the messages above */
    log_n++;
}
@end
@implementation B
+ (void)initialize { b_init_total++; }
+ (void)load { b_load_ran = 1; }
+ (int)answer { return 42; }
@end
@implementation A (Cat)
+ (void)load { self_seen_by_cat = self; log_n++; }
@end

int test_main(void) {
    a_load_before_main = log_n == 2;
    check("both +load methods ran before main", a_load_before_main, log_n);
    check("self in +load is the class", self_seen_by_a == objc_getClass("A"), 0);
    check("self in a category's +load is the class", self_seen_by_cat == objc_getClass("A"), 0);
    check("_cmd is load", cmd_seen == sel_registerName("load"), 0);
    check("+load runs on the main thread", a_main_thread == 1, a_main_thread);
    check("another class of the image answers messages from +load", a_other_ok, 0);
    check("and can be instantiated", a_new_ok, 0);
    check("whose own +load had not run yet", b_load_seen_by_a == 0, b_load_seen_by_a);
    check("and runs afterwards", b_load_ran == 1, 0);
    check("its +initialize ran on the first message, once", b_init_ran_before_load == 1 && b_init_total == 1, b_init_ran_before_load);
    return finish();
}
