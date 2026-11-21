/* poly-mac: appkit */
/* Which text fields take the keyboard focus: an editable and enabled field accepts first responder; a disabled
 * one, a non-editable one and a label (even a selectable one) do not; a button does. In a window (that is
 * not the key window: this program is never active) makeFirstResponder: on the field returns YES, installs
 * the window's field editor (an NSTextView; `currentEditor` of the field is not nil, is the window's first
 * responder (the field itself is not), and is the window's `fieldEditor:forObject:`), the editor holds the
 * field's text with ALL of it selected; makeFirstResponder:nil ends the editing (currentEditor nil, the
 * window is first responder again); makeFirstResponder: of a label returns YES but the label does not become the
 * first responder (the window keeps it; observed); of a button returns YES and the button becomes it. */
#include "poly_mak.h"

int test_main(void) {
    NSWindow *w = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 300, 100));
    NSTextField *tf = [NSTextField textFieldWithString:@"abc"];
    NSTextField *lb = [NSTextField labelWithString:@"lbl"];
    NSTextField *ro = [NSTextField labelWithString:@"ro"];
    ro.selectable = YES;
    NSButton *bt = [NSButton buttonWithTitle:@"B" target:nil action:NULL];
    for (NSView *v in @[tf, lb, ro, bt]) [w.contentView addSubview:v];
    CK(tf.acceptsFirstResponder && !lb.acceptsFirstResponder && !ro.acceptsFirstResponder && bt.acceptsFirstResponder);
    tf.enabled = NO;
    CK(!tf.acceptsFirstResponder);
    tf.enabled = YES;
    tf.editable = NO;
    CK(!tf.acceptsFirstResponder);
    tf.editable = YES;
    CK(w.firstResponder == w);
    CK([w makeFirstResponder:tf]);
    NSText *ed = tf.currentEditor;
    CK(ed != nil && w.firstResponder == ed && w.firstResponder != tf);
    CK([ed isKindOfClass:[NSTextView class]]);
    CK(ed == [w fieldEditor:NO forObject:tf]);
    CK([ed.string isEqualToString:@"abc"]);
    CK(ed.selectedRange.location == 0 && ed.selectedRange.length == 3);
    CK([w makeFirstResponder:nil]);
    CK(tf.currentEditor == nil && w.firstResponder == w);
    CK([w makeFirstResponder:tf]);
    [w endEditingFor:tf];
    CK(tf.currentEditor == nil);
    CK([w makeFirstResponder:lb]);
    CK(w.firstResponder != lb);
    CK([w makeFirstResponder:bt] && w.firstResponder == bt);
    CK(!w.isKeyWindow);
    [w close];
    return finish();
}
