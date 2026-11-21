/* poly-mac: appkit */
/* performClick: on a disabled button calls nothing and changes nothing (the state stays 0); setEnabled:YES makes
 * it work again; changing the action to another method makes the click call that one; a button whose
 * target was deallocated (the target is a weak reference) reads back a nil target and a click does
 * nothing and does not crash; -sendAction:to: called directly still sends while the button is disabled
 * (it returns YES and calls the action: the enabled flag only guards clicks). */
#include "poly_mak.h"

@interface T : NSObject
@property(nonatomic) int a, b;
- (void)act:(id)s;
- (void)other:(id)s;
@end
@implementation T
- (void)act:(id)s { (void)s; self.a++; }
- (void)other:(id)s { (void)s; self.b++; }
@end

int test_main(void) {
    T *t = [T new];
    NSButton *b = [NSButton buttonWithTitle:@"Go" target:t action:@selector(act:)];
    b.enabled = NO;
    [b performClick:nil];
    CK(t.a == 0 && b.state == NSControlStateValueOff);
    CK([b sendAction:@selector(act:) to:t]);
    CK(t.a == 1);
    b.enabled = YES;
    [b performClick:nil];
    CK(t.a == 2);
    b.action = @selector(other:);
    [b performClick:nil];
    CK(t.b == 1 && t.a == 2);
    @autoreleasepool {
        T *gone = [T new];
        b.target = gone;
    }
    [b performClick:nil];
    CK(b.target == nil);
    return finish();
}
