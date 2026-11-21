/* poly-mac: appkit */
/* The class method +buttonWithTitle:target:action: gives a push button with that title, target and action; its
 * frame starts at the origin (0,0) and is sized to fit the title (width and height positive; the
 * number is printed as SYSTEM-DEPENDENT) and its bezel style is automatic; +checkboxWithTitle:target:action:
 * gives a check box (state off, buttonType switch; its bezel style reads back as NSBezelStyleRegularSquare, 2)
 * and +radioButtonWithTitle:target:action: a radio button; both are sized to fit; pressing them toggles the
 * state (see the performClick: entry). */
#include "poly_mak.h"

@interface T : NSObject
- (void)act:(id)s;
@end
@implementation T
- (void)act:(id)s { (void)s; }
@end

int test_main(void) {
    T *t = [T new];
    NSButton *b = [NSButton buttonWithTitle:@"Go" target:t action:@selector(act:)];
    CK([b.title isEqualToString:@"Go"] && b.target == t && b.action == @selector(act:));
    CKF(b.frame.origin.x, 0);
    CKF(b.frame.origin.y, 0);
    CK(b.frame.size.width > 0 && b.frame.size.height > 0);
    CK(b.bezelStyle == NSBezelStyleAutomatic);
    value_f("sys_push_width", b.frame.size.width);
    value_f("sys_push_height", b.frame.size.height);
    NSButton *c = [NSButton checkboxWithTitle:@"Check" target:t action:@selector(act:)];
    CK([c.title isEqualToString:@"Check"] && c.state == NSControlStateValueOff);
    CK(c.bezelStyle == NSBezelStyleRegularSquare && NSBezelStyleRegularSquare == 2);
    CK(c.frame.size.width > 0 && c.frame.size.height > 0);
    value_f("sys_checkbox_height", c.frame.size.height);
    NSButton *r = [NSButton radioButtonWithTitle:@"Radio" target:t action:@selector(act:)];
    CK([r.title isEqualToString:@"Radio"] && r.target == t);
    CK(r.frame.size.width > 0 && r.frame.size.height > 0);
    return finish();
}
