/* poly-mac: appkit */
/* Editing a field with the field editor, observed by the field's delegate: focusing the field calls nothing;
 * the first change (insertText:) calls, in order, control:textShouldBeginEditing:, controlTextDidBeginEditing:,
 * controlTextDidChange:; further changes only controlTextDidChange:; the field's stringValue follows the
 * editor text as it is typed ("X" replaces the selected "abc"); ending the editing (makeFirstResponder:nil) calls
 * control:textShouldEndEditing: and then controlTextDidEndEditing: and the stringValue is the typed text
 * ("hello" after "X" is replaced by selectAll and insert). With no change at all, ending the editing posts
 * no begin; the notifications go to the delegate and to the default centre (the log shows delegate calls only). */
#include "poly_mak.h"

@interface DG : NSObject <NSTextFieldDelegate>
@property(nonatomic, strong) NSMutableString *log;
@end
@implementation DG
- (void)controlTextDidBeginEditing:(NSNotification *)n { (void)n; [self.log appendString:@"begin;"]; }
- (void)controlTextDidChange:(NSNotification *)n { (void)n; [self.log appendString:@"change;"]; }
- (void)controlTextDidEndEditing:(NSNotification *)n { (void)n; [self.log appendString:@"end;"]; }
- (BOOL)control:(NSControl *)c textShouldBeginEditing:(NSText *)t { (void)c; (void)t; [self.log appendString:@"shouldBegin;"]; return YES; }
- (BOOL)control:(NSControl *)c textShouldEndEditing:(NSText *)t { (void)c; (void)t; [self.log appendString:@"shouldEnd;"]; return YES; }
@end

int test_main(void) {
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 300, 100));
    NSTextField *tf = [NSTextField textFieldWithString:@"abc"];
    [w.contentView addSubview:tf];
    DG *d = [DG new];
    d.log = [NSMutableString string];
    tf.delegate = d;
    CK([w makeFirstResponder:tf]);
    CK(d.log.length == 0);
    NSTextView *ed = (NSTextView *)tf.currentEditor;
    [ed insertText:@"X" replacementRange:NSMakeRange(NSNotFound, 0)];
    CK([ed.string isEqualToString:@"X"] && [tf.stringValue isEqualToString:@"X"]);
    CK([d.log isEqualToString:@"shouldBegin;begin;change;"]);
    [d.log setString:@""];
    [ed selectAll:nil];
    [ed insertText:@"hello" replacementRange:NSMakeRange(NSNotFound, 0)];
    CK([d.log isEqualToString:@"change;"]);
    [d.log setString:@""];
    CK([w makeFirstResponder:nil]);
    CK([d.log isEqualToString:@"shouldEnd;end;"]);
    CK([tf.stringValue isEqualToString:@"hello"]);
    [d.log setString:@""];
    CK([w makeFirstResponder:tf]);
    CK([w makeFirstResponder:nil]);
    value_ns("focus and leave without typing", d.log);
    [w close];
    return finish();
}
