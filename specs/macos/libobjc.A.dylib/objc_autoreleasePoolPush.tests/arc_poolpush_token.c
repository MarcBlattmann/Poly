// poly-mac: libobjc
// objc_autoreleasePoolPush: returns a non-null token; tokens of nested pools differ; the token
// pushed after a pop is the one the popped pool had (the stack of pools is what the tokens name).
#include "poly_mobjc.h"
int test_main(void) {
    void *a = objc_autoreleasePoolPush();
    check("the token is not null", a != 0, 0);
    void *b = objc_autoreleasePoolPush();
    check("the token is not null (inner)", b != 0, 0);
    check("nested pools have different tokens", a != b, 0);
    void *c = objc_autoreleasePoolPush();
    check("a third one differs from both", c != a && c != b, 0);
    objc_autoreleasePoolPop(c);
    void *c2 = objc_autoreleasePoolPush();
    check("pushing after a pop gives the popped token again", c2 == c, 0);
    objc_autoreleasePoolPop(c2);
    objc_autoreleasePoolPop(b);
    void *b2 = objc_autoreleasePoolPush();
    check("again one level up", b2 == b, 0);
    objc_autoreleasePoolPop(b2);
    objc_autoreleasePoolPop(a);
    return finish();
}
