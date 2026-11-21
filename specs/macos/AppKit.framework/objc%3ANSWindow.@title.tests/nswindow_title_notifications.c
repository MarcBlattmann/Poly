/* poly-mac: appkit */
/* Changing the title posts no notification and calls no delegate method: observers on the window
 * itself see nothing; a window's title is per window (two windows do not share it) and survives
 * ordering the window front and out again; `contentLayoutGuide` exists and a window's
 * `windowTitlebarLayoutDirection` is left to right here. */
#include "poly_mak.h"

int test_main(void) {
    __block int n = 0;
    NSWindow *a = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    NSWindow *b = ak_window(NSWindowStyleMaskTitled, NSMakeRect(0, 0, 100, 100));
    id tok = [[NSNotificationCenter defaultCenter] addObserverForName:nil object:a queue:nil usingBlock:^(NSNotification *x) {
        (void)x;
        n++;
    }];
    a.title = @"A";
    a.title = @"A2";
    CKV(n == 0, n);
    b.title = @"B";
    CK(![a.title isEqualToString:b.title]);
    [a orderFront:nil];
    ak_spin(0.05);
    [a orderOut:nil];
    ak_spin(0.05);
    CK([a.title isEqualToString:@"A2"]);
    CK(a.contentLayoutGuide != nil);
    CK(a.windowTitlebarLayoutDirection == NSUserInterfaceLayoutDirectionLeftToRight);
    [[NSNotificationCenter defaultCenter] removeObserver:tok];
    return finish();
}
