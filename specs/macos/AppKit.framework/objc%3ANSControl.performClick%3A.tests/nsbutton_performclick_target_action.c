/* poly-mac: appkit */
/* performClick: on an enabled button sends its action to its target at once (before it returns), exactly once,
 * with the button itself as the sender; the state of the sender is already 1 inside the action
 * (a click toggles the state, also for a push button) and stays 1 after the first click; -sendAction:to: with the button's own action and target calls the action
 * and returns YES; with a nil target and an action no object answers it returns NO and nothing is called;
 * the click leaves the button not highlighted; sendActionOn: returns the previous mask (mouse up, 4, by
 * default) and sets the new one. */
#include "poly_mak.h"

@interface T : NSObject
@property(nonatomic, strong) NSMutableString *log;
@property(nonatomic) id lastSender;
- (void)act:(id)sender;
@end
@implementation T
- (void)act:(id)sender {
    self.lastSender = sender;
    [self.log appendFormat:@"act(state=%ld);", (long)[(NSButton *)sender state]];
}
@end

int test_main(void) {
    T *t = [T new];
    t.log = [NSMutableString string];
    NSButton *b = [NSButton buttonWithTitle:@"Go" target:t action:@selector(act:)];
    [b performClick:nil];
    CK([t.log isEqualToString:@"act(state=1);"]);
    CK(t.lastSender == b);
    CK(b.state == NSControlStateValueOn && !b.isHighlighted);
    [t.log setString:@""];
    CK([b sendAction:b.action to:b.target]);
    CK([t.log isEqualToString:@"act(state=1);"]);
    [t.log setString:@""];
    CK(![b sendAction:@selector(act:) to:nil]);
    CK(t.log.length == 0);
    NSEventMask old = [b sendActionOn:NSEventMaskLeftMouseDown];
    CK(old == NSEventMaskLeftMouseUp && NSEventMaskLeftMouseUp == 4);
    CK([b sendActionOn:NSEventMaskLeftMouseUp] == NSEventMaskLeftMouseDown);
    NSButton *noaction = [[NSButton alloc] initWithFrame:NSMakeRect(0, 0, 50, 24)];
    [noaction performClick:nil];
    CK(noaction.state == NSControlStateValueOn || noaction.state == NSControlStateValueOff);
    return finish();
}
