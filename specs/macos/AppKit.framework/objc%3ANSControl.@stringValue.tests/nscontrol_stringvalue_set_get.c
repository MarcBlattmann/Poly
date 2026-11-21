/* poly-mac: appkit */
/* stringValue of a text field: empty (not nil) at the start; the setter stores a copy (a changed mutable string does not
 * change the field); a nil objectValue gives an empty stringValue; non-ASCII, emoji and very long text (5000
 * characters) are kept whole; newlines are kept; setting the same text again does no harm; the field's
 * frame does not change; setStringValue: does not call the target's action. */
#include "poly_mak.h"

@interface T : NSObject
@property(nonatomic) int n;
- (void)act:(id)s;
@end
@implementation T
- (void)act:(id)s { (void)s; self.n++; }
@end

int test_main(void) {
    NSTextField *f = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 0, 100, 22)];
    T *t = [T new];
    f.target = t;
    f.action = @selector(act:);
    CK(f.stringValue != nil && f.stringValue.length == 0);
    NSMutableString *m = [NSMutableString stringWithString:@"abc"];
    f.stringValue = m;
    [m appendString:@"def"];
    CK([f.stringValue isEqualToString:@"abc"]);
    f.stringValue = @"Grüße 中\U0001F600";
    CK([f.stringValue isEqualToString:@"Grüße 中\U0001F600"]);
    f.stringValue = [@"" stringByPaddingToLength:5000 withString:@"x" startingAtIndex:0];
    CK(f.stringValue.length == 5000);
    NSString *nl = [NSString stringWithFormat:@"a%cb", 10];
    f.stringValue = nl;
    CK([f.stringValue isEqualToString:nl]);
    f.objectValue = nil;
    CK(f.stringValue.length == 0);
    f.stringValue = @"same";
    f.stringValue = @"same";
    CK([f.stringValue isEqualToString:@"same"]);
    CK(NSEqualRects(f.frame, NSMakeRect(0, 0, 100, 22)));
    CK(t.n == 0);
    return finish();
}
